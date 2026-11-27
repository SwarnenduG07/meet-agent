//! `meet-agent listen`: talk to the agent with a mic and speakers (no Meet).

use std::{
    path::{Path, PathBuf},
    time::Instant,
};

use anyhow::{Context, Result};
use audio::listener::Heard;
use brain_client::{Brain, protocol::Event};
use tokio::sync::mpsc::UnboundedReceiver;

use crate::{
    config::{Config, device},
    voice::{self, Input},
};

pub struct Options {
    pub brief: PathBuf,
    /// Stop talking when someone else talks (needs headphones or `BlackHole`).
    pub barge_in: bool,
    /// Listen to a recording instead of the mic.
    pub file: Option<PathBuf>,
    /// Don't play any sound.
    pub mute: bool,
}

/// Whisper's usual output for noise or silence.
const NOISE: [&str; 5] = ["Thank you.", "Thanks for watching!", "you", "Bye.", "."];

struct Turn {
    asked: Instant,
    id: u64,
    events: UnboundedReceiver<Event>,
    ended: Instant,
    question: String,
    spoke: bool,
}

/// Prints a log line with seconds since the agent started.
pub fn log(stage: &str, msg: &str) {
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    let t = START.get_or_init(Instant::now).elapsed().as_secs_f32();
    println!("[{t:>7.2}s] [{stage:<7}] {msg}");
}

pub async fn run(cfg: &Config, opts: Options) -> Result<()> {
    let context = std::fs::read_to_string(&opts.brief)
        .with_context(|| format!("reading {}", opts.brief.display()))?;
    let input = match &opts.file {
        Some(path) => Input::Samples(read_wav_16k(path)?),
        None => Input::Device(device(&cfg.audio.listen_input)),
    };
    let outputs = if opts.mute {
        Vec::new()
    } else {
        vec![device(&cfg.audio.monitor_output)]
    };

    let t = Instant::now();
    log("startup", "loading brain and speech models...");
    let brain_settings = cfg.brain();
    let (brain, loaded) = tokio::join!(
        Brain::start(&brain_settings),
        voice::load(cfg, input, outputs, !opts.barge_in)
    );
    let (brain, (voice, mut ears)) = (brain?, loaded?);
    brain.warm_up(&context).await?;
    log(
        "startup",
        &format!(
            "ready in {:.1}s; listening on {}, speaking on {:?}",
            t.elapsed().as_secs_f32(),
            ears.device,
            voice.outputs()
        ),
    );

    let allowed = ["answer", "defer_to_sunny", "wait"];
    let mut current: Option<Turn> = None;
    let mut pending: Option<(String, Instant)> = None;
    let mut input_done = false;

    loop {
        tokio::select! {
            heard = ears.heard.recv(), if !input_done => match heard {
                None => input_done = true,
                Some(Heard::SpeechStarted) => {
                    log("vad", "speech started");
                    if opts.barge_in && (voice.is_speaking() || current.is_some()) {
                        voice.stop();
                        if let Some(c) = current.take() {
                            brain.cancel(c.id);
                        }
                        log("barge", "interrupted; stopped talking");
                    }
                }
                Some(Heard::Turn { audio, ended, pause_ms, done_prob }) => {
                    log("turn", &format!("finished (done={done_prob:.2}, pause {pause_ms}ms, {:.1}s)", seconds(audio.len())));
                    let t = Instant::now();
                    let text = voice.transcribe(audio).await?;
                    log("whisper", &format!("{:.2}s  heard: {text}", t.elapsed().as_secs_f32()));
                    if text.is_empty() || NOISE.contains(&text.as_str()) {
                        continue;
                    }
                    let question = match pending.take() {
                        Some((before, at)) if at.elapsed().as_secs() < 10 => format!("{before} {text}"),
                        _ => text,
                    };
                    let (id, events) = brain.think(&question, "LISTENING", &allowed, &context, None)?;
                    current = Some(Turn { asked: Instant::now(), id, events, ended, question, spoke: false });
                }
            },
            event = next_event(&mut current) => {
                on_brain_event(&voice, &mut current, &mut pending, event).await?;
            },
        }
        if input_done && current.is_none() {
            break;
        }
    }
    voice.finish().await;
    brain.shutdown().await;
    Ok(())
}

/// Speaks streamed sentences as they arrive and closes the turn on its reply.
async fn on_brain_event(
    voice: &voice::Voice,
    current: &mut Option<Turn>,
    pending: &mut Option<(String, Instant)>,
    event: Option<Event>,
) -> Result<()> {
    let Some(turn) = current.as_mut() else {
        return Ok(());
    };
    match event {
        Some(Event::Sentence { text, .. }) => {
            if !turn.spoke {
                log(
                    "brain",
                    &format!(
                        "first sentence after {:.2}s",
                        turn.asked.elapsed().as_secs_f32()
                    ),
                );
            }
            let since_end = turn.ended.elapsed();
            let to_audio = voice.say(&text).await?;
            log(
                "kokoro",
                &format!("first audio after {:.2}s", to_audio.as_secs_f32()),
            );
            if !turn.spoke {
                turn.spoke = true;
                let total = (since_end + to_audio).as_secs_f32();
                log(
                    "total",
                    &format!("{total:.2}s from end of question to first audio"),
                );
            }
            log("say", &text);
        }
        Some(Event::Reply { action, .. }) => {
            log("brain", &format!("action: {action}"));
            if action == "wait" {
                log("brain", "unfinished sentence, waiting for the rest");
                *pending = Some((turn.question.clone(), Instant::now()));
            }
            *current = None;
        }
        other => {
            log("brain", &format!("{other:?}"));
            *current = None;
        }
    }
    Ok(())
}

async fn next_event(current: &mut Option<Turn>) -> Option<Event> {
    match current {
        Some(turn) => turn.events.recv().await,
        None => std::future::pending().await,
    }
}

/// Reads a WAV file as 16 kHz mono floats.
fn read_wav_16k(path: &Path) -> Result<Vec<f32>> {
    let mut reader =
        hound::WavReader::open(path).with_context(|| format!("opening {}", path.display()))?;
    let spec = reader.spec();
    let raw: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader.samples::<f32>().collect::<Result<_, _>>()?,
        hound::SampleFormat::Int => reader
            .samples::<i16>()
            .map(|s| s.map(|v| f32::from(v) / 32_768.0))
            .collect::<Result<_, _>>()?,
    };
    let channels = usize::from(spec.channels);
    let mono: Vec<f32> = raw
        .chunks(channels)
        .map(|f| f.iter().sum::<f32>() / f32::from(spec.channels))
        .collect();
    Ok(audio::resample::Resampler::new(spec.sample_rate, audio::SAMPLE_RATE).process(&mono))
}

#[allow(
    clippy::cast_precision_loss,
    reason = "a turn is at most a few minutes of samples"
)]
fn seconds(samples: usize) -> f32 {
    samples as f32 / audio::SAMPLE_RATE as f32
}
