//! `meet-agent listen`: talk to the agent with a mic and speakers (no Meet).

use std::{
    path::{Path, PathBuf},
    time::Instant,
};

use anyhow::{Context, Result};
use brain_client::Brain;

use crate::{
    config::{Config, device},
    conversation::{self, Director, log},
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

/// Answers questions; never acts.
struct FreeTalk {
    context: String,
}

impl Director for FreeTalk {
    fn state(&self) -> &'static str {
        "LISTENING"
    }
    fn allowed(&self) -> Vec<&'static str> {
        vec!["answer", "defer_to_sunny", "wait"]
    }
    fn context(&self) -> String {
        self.context.clone()
    }
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

    let mut director = FreeTalk { context };
    let opts = conversation::Options {
        barge_in: opts.barge_in,
        stop_at_end_of_input: opts.file.is_some(),
    };
    let result = conversation::run(
        &brain,
        &voice,
        &mut ears,
        &mut director,
        conversation::keyboard(),
        opts,
    )
    .await;
    brain.shutdown().await;
    result
}

/// Reads a WAV file as 16 kHz mono floats.
pub fn read_wav_16k(path: &Path) -> Result<Vec<f32>> {
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
