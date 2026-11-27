//! Kokoro speaks, Whisper listens: `cargo run -p speech --example roundtrip -- <whisper.bin> <kokoro dir> "text"`

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "test tool: converts sample counts and rates"
)]

use std::{path::Path, time::Instant};

use speech::{
    phonemes::phonemize,
    stt::Stt,
    tts::{self, Tts},
};

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let text = args.get(3).map_or(
        "Hi everyone, I'm Sunny's meet agent. Let me share my screen.",
        String::as_str,
    );
    println!("phonemes: {}", phonemize(text)?);

    let t = Instant::now();
    let mut tts = Tts::load(Path::new(&args[2]), "af_heart", 8)?;
    tts.synthesize("Ready.")?;
    println!("kokoro loaded + warm in {:.2}s", t.elapsed().as_secs_f32());
    let t = Instant::now();
    let wave = tts.synthesize(text)?;
    println!(
        "kokoro: {:.2}s for {:.1}s of speech",
        t.elapsed().as_secs_f32(),
        wave.len() as f32 / tts::SAMPLE_RATE as f32
    );
    let mut w = hound::WavWriter::create(
        std::env::temp_dir().join("meet-agent-roundtrip.wav"),
        hound::WavSpec {
            channels: 1,
            sample_rate: tts::SAMPLE_RATE,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        },
    )?;
    for s in &wave {
        w.write_sample(*s)?;
    }
    w.finalize()?;

    let t = Instant::now();
    let mut stt = Stt::load(
        Path::new(&args[1]),
        "Google Meet, meet agent, Sunny, Flowline, Rust API.",
    )?;
    stt.transcribe(&vec![0.0; 16_000])?;
    println!("whisper loaded + warm in {:.2}s", t.elapsed().as_secs_f32());
    let audio16 =
        audio::resample::Resampler::new(tts::SAMPLE_RATE, audio::SAMPLE_RATE).process(&wave);
    for run in 1..=2 {
        let t = Instant::now();
        let heard = stt.transcribe(&audio16)?;
        println!(
            "whisper run {run}: {:.2}s -> {heard}",
            t.elapsed().as_secs_f32()
        );
    }
    Ok(())
}
