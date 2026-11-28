//! `meet-agent run <brief>`: the whole meeting. Prepare, join, present, answer,
//! leave, and write a summary. `--rehearse` does all of it without Meet.

use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use brain_client::Brain;
use serde_json::json;

use crate::{
    config::{Config, device, expand},
    conversation::{self, log},
    join,
    listen::read_wav_16k,
    orchestrator::Orchestrator,
    plan::{Plan, SharePolicy, sentences},
    timeline::Timeline,
    voice::{self, Input, Voice},
};

pub struct Options {
    pub brief: PathBuf,
    /// Overrides the brief's link.
    pub link: Option<String>,
    /// Overrides the brief's share policy.
    pub share: Option<SharePolicy>,
    /// Join now instead of waiting for the brief's start time.
    pub now: bool,
    /// Run everything locally with your mic and speakers, without Meet.
    pub rehearse: bool,
    /// Rehearsal: listen to a recording instead of the mic; it starts after
    /// `file_delay` seconds.
    pub file: Option<PathBuf>,
    pub file_delay: u32,
    /// Rehearsal: play no sound.
    pub mute: bool,
}

pub async fn run(cfg: &Config, opts: Options) -> Result<()> {
    let mut plan = Plan::from_brief(&opts.brief)?;
    if let Some(link) = &opts.link {
        plan.link = Some(link.clone());
    }
    if let Some(share) = opts.share {
        plan.share = share;
    }
    let link = plan
        .link
        .clone()
        .filter(|l| l.starts_with("https://meet.google.com/"));
    if !opts.rehearse && link.is_none() {
        bail!("the brief has no Meet link (add `link:` to its header, or pass --link)");
    }
    let mut timeline = Timeline::create(&expand(&cfg.paths.runs), &plan.title)?;
    log(
        "prepare",
        &format!(
            "{} ({} sections); notes in {}",
            plan.title,
            plan.sections.len(),
            timeline.dir.display()
        ),
    );

    let t = Instant::now();
    let (brain, voice, mut ears) = start(cfg, &plan.context, &opts).await?;
    log(
        "prepare",
        &format!("models ready in {:.1}s", t.elapsed().as_secs_f32()),
    );
    write_scripts(&brain, &mut plan, &cfg.brain.owner).await?;
    plan.save(&timeline.dir.join("plan.json"))?;

    let call = if opts.rehearse {
        log(
            "rehearse",
            "no Meet: speaking locally; type s/n/t/l + Enter to share, skip, take over, leave",
        );
        None
    } else {
        if !opts.now {
            wait_for_start(plan.start.as_deref()).await;
        }
        let session = join::attach(cfg).await?;
        let Some(call) =
            join::enter(cfg, &session, link.as_deref().unwrap_or_default(), false).await?
        else {
            brain.shutdown().await;
            return Ok(());
        };
        log(
            "meet",
            "in the call. Type t + Enter to take over, s to share, n for next section, l to leave.",
        );
        Some(call)
    };
    timeline.record("joined", json!({ "rehearsal": opts.rehearse }));

    let greeting = format!("Hi everyone, I'm {}.", cfg.brain.name);
    voice.say(&greeting).await?;
    let start_presenting = plan.share == SharePolicy::AtStart && !plan.sections.is_empty();
    let owner = cfg.brain.owner.clone();
    let mut director = Orchestrator::new(
        call,
        plan,
        timeline,
        owner,
        cfg.audio.agent_output.clone(),
        cfg.audio.your_mic.clone(),
    );
    if start_presenting {
        voice
            .say("Let me share my screen and walk you through it.")
            .await?;
        director.start_presenting_now().await?;
    }

    let conv = conversation::Options {
        barge_in: !opts.rehearse || opts.file.is_some(),
        stop_at_end_of_input: opts.file.is_some(),
    };
    let result = conversation::run(
        &brain,
        &voice,
        &mut ears,
        &mut director,
        conversation::keyboard(),
        conv,
    )
    .await;

    wrap_up(cfg, &brain, &mut director).await?;
    brain.shutdown().await;
    result
}

/// Leaves the call (if still in it) and writes the summary.
async fn wrap_up(cfg: &Config, brain: &Brain, director: &mut Orchestrator) -> Result<()> {
    if let Some(call) = director.call.take()
        && !director.left
    {
        call.leave().await?;
        log("meet", "left the call");
    }
    director.timeline.record("left", json!({}));
    let recap = recap(brain, director).await;
    let summary = director.timeline.write_summary(
        &director.plan.title,
        &cfg.brain.owner,
        recap.as_deref(),
    )?;
    log("done", &format!("summary: {}", summary.display()));
    Ok(())
}

async fn start(cfg: &Config, context: &str, opts: &Options) -> Result<(Brain, Voice, voice::Ears)> {
    let (input, outputs, mute_while_speaking) = if opts.rehearse {
        let input = match &opts.file {
            Some(path) => {
                let mut samples = vec![0.0; usize::try_from(opts.file_delay)? * 16_000];
                samples.extend(read_wav_16k(path)?);
                Input::Samples(samples)
            }
            None => Input::Device(device(&cfg.audio.listen_input)),
        };
        let outputs = if opts.mute {
            Vec::new()
        } else {
            vec![device(&cfg.audio.monitor_output)]
        };
        (input, outputs, opts.file.is_none())
    } else {
        let outputs = vec![
            Some(cfg.audio.agent_output.clone()),
            device(&cfg.audio.monitor_output),
        ];
        // The agent can't hear itself through BlackHole, so it never mutes.
        (
            Input::Device(Some(cfg.audio.call_input.clone())),
            outputs,
            false,
        )
    };
    let brain_settings = cfg.brain();
    let (brain, loaded) = tokio::join!(
        Brain::start(&brain_settings),
        voice::load(cfg, input, outputs, mute_while_speaking)
    );
    let (brain, (voice, ears)) = (brain?, loaded?);
    brain.warm_up(context).await?;
    log(
        "prepare",
        &format!("hearing {}, speaking on {:?}", ears.device, voice.outputs()),
    );
    Ok((brain, voice, ears))
}

/// Before the meeting: the brain writes what to say for each section.
async fn write_scripts(brain: &Brain, plan: &mut Plan, owner: &str) -> Result<()> {
    let count = plan.sections.len();
    for (i, section) in plan.sections.iter_mut().enumerate() {
        let transition = match i {
            0 => "This is the first part.",
            _ if i + 1 == count => {
                "This is the last part; start with a short transition like \"Finally,\"."
            }
            _ => "Start with a short transition like \"Next,\".",
        };
        let prompt = format!(
            "You are presenting part of a meeting out loud on behalf of {owner}.\n\
             Part {} of {count}: {}\nTalking points:\n- {}\n\n\
             {transition} Write exactly what you say: 2 to 4 short, natural spoken sentences that cover the \
             talking points. Only facts from the talking points. No lists, no markdown, no quotes.",
            i + 1,
            section.title,
            section.talk.join("\n- ")
        );
        let t = Instant::now();
        let text = brain
            .complete(&prompt, 220, None)
            .await
            .with_context(|| format!("script for {}", section.title))?;
        section.script = sentences(&text);
        log(
            "prepare",
            &format!(
                "script {}/{count} ({:.1}s): {}",
                i + 1,
                t.elapsed().as_secs_f32(),
                section.title
            ),
        );
    }
    Ok(())
}

/// Waits until one minute before `HH:MM` today (local time), if that's ahead.
async fn wait_for_start(start: Option<&str>) {
    let Some((h, m)) = start.and_then(|s| s.split_once(':')) else {
        return;
    };
    let (Ok(h), Ok(m)) = (h.trim().parse::<u64>(), m.trim().parse::<u64>()) else {
        return;
    };
    let now = std::process::Command::new("date")
        .arg("+%H %M %S")
        .output()
        .ok();
    let Some(now) = now.map(|o| String::from_utf8_lossy(&o.stdout).to_string()) else {
        return;
    };
    let parts: Vec<u64> = now
        .split_whitespace()
        .filter_map(|p| p.parse().ok())
        .collect();
    let [nh, nm, ns] = parts[..] else { return };
    let now_s = nh * 3600 + nm * 60 + ns;
    let join_s = (h * 3600 + m * 60).saturating_sub(60);
    if join_s > now_s {
        log(
            "prepare",
            &format!(
                "waiting until {:02}:{:02} to join (1 minute early)",
                join_s / 3600,
                (join_s / 60) % 60
            ),
        );
        tokio::time::sleep(Duration::from_secs(join_s - now_s)).await;
    }
}

/// A short spoken-style recap for the summary, written by the brain.
async fn recap(brain: &Brain, o: &Orchestrator) -> Option<String> {
    if o.timeline.questions.is_empty() && o.timeline.covered.is_empty() {
        return None;
    }
    let qa: Vec<String> = o
        .timeline
        .questions
        .iter()
        .map(|(q, a)| format!("Q: {q}\nA: {a}"))
        .collect();
    let prompt = format!(
        "Write a 2-3 sentence recap of this meeting for {owner}, who wasn't there. Mention what was \
         presented and anything {owner} must follow up on. Plain text.\n\nPresented: {}\nDeferred to {owner}: {}\n{}",
        o.timeline.covered.join(", "),
        o.timeline.deferred.join("; "),
        qa.join("\n"),
        owner = o.owner
    );
    brain.complete(&prompt, 160, None).await.ok()
}
