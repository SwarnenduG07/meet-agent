//! `meet-agent talk <link>`: the agent joins, then talks in the call. It can share
//! and stop sharing the screen and leave when asked; you can take over any time.

use std::time::Instant;

use anyhow::{Context, Result};
use brain_client::Brain;
use meet::call::Call;

use crate::{
    config::{Config, device},
    conversation::{self, Control, Director, Flow, Tools, log},
    join,
    voice::{self, Input},
};

pub struct Options {
    pub link: String,
    pub brief: std::path::PathBuf,
    /// App, file or URL to bring forward whenever sharing starts.
    pub show: Option<String>,
}

pub struct MeetDirector {
    pub call: Call,
    pub context: String,
    pub show: Option<String>,
    pub agent_mic: String,
    pub your_mic: String,
    pub sharing: bool,
    pub taken_over: bool,
    pub left: bool,
}

impl MeetDirector {
    pub async fn share(&mut self) -> Result<()> {
        self.call.share().await?;
        self.sharing = true;
        if let Some(target) = &self.show {
            desktop::windows::show(target)?;
        }
        log("meet", "sharing the screen");
        Ok(())
    }

    pub async fn stop_sharing(&mut self) -> Result<()> {
        self.call.stop_sharing().await?;
        self.sharing = false;
        log("meet", "stopped sharing");
        Ok(())
    }

    /// You speak in the call with your own mic; the agent stops and listens.
    pub async fn toggle_take_over(&mut self, tools: &Tools<'_>) -> Result<()> {
        if self.taken_over {
            let mic = self.call.switch_mic(&self.agent_mic).await?;
            self.taken_over = false;
            log("control", &format!("agent is back (mic: {mic})"));
        } else {
            tools.voice.stop();
            let mic = self.call.switch_mic(&self.your_mic).await?;
            self.taken_over = true;
            log(
                "control",
                &format!("you have the call (mic: {mic}); type t to hand it back"),
            );
        }
        Ok(())
    }
}

impl Director for MeetDirector {
    fn state(&self) -> &'static str {
        if self.sharing {
            "PRESENTING"
        } else {
            "LISTENING"
        }
    }

    fn allowed(&self) -> Vec<&'static str> {
        let screen = if self.sharing {
            "stop_sharing"
        } else {
            "share_screen"
        };
        vec!["answer", "defer_to_sunny", "wait", screen, "leave"]
    }

    fn context(&self) -> String {
        self.context.clone()
    }

    fn listening(&self) -> bool {
        !self.taken_over
    }

    async fn act(
        &mut self,
        _question: &str,
        action: &str,
        _say: &str,
        tools: &Tools<'_>,
    ) -> Result<Flow> {
        match action {
            "share_screen" if !self.sharing => self.share().await?,
            "stop_sharing" if self.sharing => self.stop_sharing().await?,
            "leave" => {
                tools.voice.finish().await;
                return Ok(Flow::Stop);
            }
            _ => {}
        }
        Ok(Flow::Continue)
    }

    async fn tick(&mut self, _tools: &Tools<'_>) -> Result<Flow> {
        if self.call.in_call().await {
            return Ok(Flow::Continue);
        }
        log("meet", "the call ended or the agent was removed");
        self.left = true;
        Ok(Flow::Stop)
    }

    async fn control(&mut self, control: Control, tools: &Tools<'_>) -> Result<Flow> {
        match control {
            Control::TakeOver => self.toggle_take_over(tools).await?,
            Control::Share if self.sharing => self.stop_sharing().await?,
            Control::Share => self.share().await?,
            Control::Leave => {
                tools
                    .voice
                    .say("Thanks everyone, I'll leave the call now.")
                    .await?;
                tools.voice.finish().await;
                return Ok(Flow::Stop);
            }
            Control::Next => {}
        }
        Ok(Flow::Continue)
    }

    async fn finish(&mut self, _tools: &Tools<'_>) -> Result<()> {
        if self.sharing {
            let _ = self.stop_sharing().await;
        }
        Ok(())
    }
}

/// Starts brain + voice on the call's audio devices.
pub async fn start_voice(
    cfg: &Config,
    context: &str,
) -> Result<(Brain, voice::Voice, voice::Ears)> {
    let outputs = vec![
        Some(cfg.audio.agent_output.clone()),
        device(&cfg.audio.monitor_output),
    ];
    let input = Input::Device(Some(cfg.audio.call_input.clone()));
    let brain_settings = cfg.brain();
    // The agent can't hear itself through BlackHole, so no muting while it talks.
    let (brain, loaded) = tokio::join!(
        Brain::start(&brain_settings),
        voice::load(cfg, input, outputs, false)
    );
    let (brain, (voice, ears)) = (brain?, loaded?);
    brain.warm_up(context).await?;
    Ok((brain, voice, ears))
}

pub async fn run(cfg: &Config, opts: Options) -> Result<()> {
    let context = std::fs::read_to_string(&opts.brief)
        .with_context(|| format!("reading {}", opts.brief.display()))?;
    let t = Instant::now();
    log("startup", "loading brain and speech models...");
    let (brain, voice, mut ears) = start_voice(cfg, &context).await?;
    log(
        "startup",
        &format!(
            "ready in {:.1}s; hearing {}, speaking on {:?}",
            t.elapsed().as_secs_f32(),
            ears.device,
            voice.outputs()
        ),
    );

    let session = join::attach(cfg).await?;
    let Some(call) = join::enter(cfg, &session, &opts.link, false).await? else {
        brain.shutdown().await;
        return Ok(());
    };
    log(
        "meet",
        "in the call. Type t + Enter to take over, s to share, l to leave.",
    );
    voice
        .say(&format!("Hi everyone, I'm {}.", cfg.brain.name))
        .await?;

    let mut director = MeetDirector {
        call,
        context,
        show: opts.show,
        agent_mic: cfg.audio.agent_output.clone(),
        your_mic: cfg.audio.your_mic.clone(),
        sharing: false,
        taken_over: false,
        left: false,
    };
    let conv = conversation::Options {
        barge_in: true,
        stop_at_end_of_input: false,
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
    if !director.left {
        director.call.leave().await?;
        log("meet", "left the call");
    }
    brain.shutdown().await;
    result
}
