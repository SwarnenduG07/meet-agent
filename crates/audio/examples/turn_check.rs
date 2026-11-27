//! Compares Rust VAD / Smart Turn / log-mel with the Python versions.
//! `cargo run -p audio --example turn_check -- <wav> <silero.onnx> <smart-turn.onnx>`

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "test tool: converts sample counts and rates"
)]

use std::path::Path;

use audio::{CHUNK, SAMPLE_RATE, mel::LogMel, resample::Resampler, turn::SmartTurn, vad::Vad};

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let mut reader = hound::WavReader::open(&args[1])?;
    let spec = reader.spec();
    let raw: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader.samples::<f32>().collect::<Result<_, _>>()?,
        hound::SampleFormat::Int => reader
            .samples::<i16>()
            .map(|s| s.map(|v| f32::from(v) / 32768.0))
            .collect::<Result<_, _>>()?,
    };
    let audio = Resampler::new(spec.sample_rate, SAMPLE_RATE).process(&raw);

    let mut vad = Vad::load(Path::new(&args[2]))?;
    let mut probs = Vec::new();
    for c in audio.as_chunks::<CHUNK>().0 {
        probs.push(vad.speech_prob(c)?);
    }
    let speech = probs.iter().filter(|&&p| p > 0.5).count();
    let mean: f32 = probs.iter().sum::<f32>() / probs.len() as f32;
    println!(
        "rust:   vad_chunks={} speech_chunks={speech} mean_prob={mean:.4}",
        probs.len()
    );

    let feats = LogMel::new().features(&normalize(&audio), 800);
    let fm: f64 = feats.iter().map(|&v| f64::from(v)).sum::<f64>() / feats.len() as f64;
    let fs = (feats
        .iter()
        .map(|&v| (f64::from(v) - fm).powi(2))
        .sum::<f64>()
        / feats.len() as f64)
        .sqrt();
    let mut turn = SmartTurn::load(Path::new(&args[3]))?;
    let full = turn.done_prob(&audio)?;
    let part = turn.done_prob(&audio[..(1.4 * f64::from(SAMPLE_RATE)) as usize])?;
    println!(
        "rust:   turn_full={full:.4} turn_first_1.4s={part:.4} feat_mean={fm:.5} feat_std={fs:.5} feat[0,10,100]={:.5}",
        feats[10 * 800 + 100]
    );
    Ok(())
}

fn normalize(a: &[f32]) -> Vec<f32> {
    let n = a.len() as f64;
    let mean = a.iter().map(|&x| f64::from(x)).sum::<f64>() / n;
    let var = a
        .iter()
        .map(|&x| (f64::from(x) - mean).powi(2))
        .sum::<f64>()
        / n;
    a.iter()
        .map(|&x| ((f64::from(x) - mean) / (var + 1e-7).sqrt()) as f32)
        .collect()
}
