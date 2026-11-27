//! Text → speech with Kokoro-82M (ONNX, CPU). Output is 24 kHz mono.

use std::{collections::HashMap, path::Path};

use anyhow::{Context, Result, anyhow, bail};
use ort::{session::Session, value::Tensor};

use crate::phonemes::phonemize;

pub const SAMPLE_RATE: u32 = 24_000;
const STYLE: usize = 256;
const MAX_TOKENS: usize = 510;

pub struct Tts {
    session: Session,
    vocab: HashMap<char, i64>,
    /// One 256-value style per phoneme count (510 of them).
    voice: Vec<f32>,
    pub speed: f32,
}

impl Tts {
    /// `model_dir` holds `onnx/model.onnx`, `tokenizer.json` and `voices/<voice>.bin`;
    /// `threads` CPU threads (8 is fast on Apple Silicon and leaves room for Chrome).
    ///
    /// # Errors
    /// Fails when a file is missing or invalid.
    pub fn load(model_dir: &Path, voice: &str, threads: usize) -> Result<Self> {
        let session = audio::onnx::session(&model_dir.join("onnx/model.onnx"), threads)?;
        let tokenizer: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(model_dir.join("tokenizer.json"))?)?;
        let vocab = tokenizer["model"]["vocab"]
            .as_object()
            .context("tokenizer.json has no vocab")?
            .iter()
            .filter_map(|(k, v)| Some((k.chars().next()?, v.as_i64()?)))
            .collect();
        let bytes = std::fs::read(model_dir.join(format!("voices/{voice}.bin")))
            .with_context(|| format!("voice {voice:?}"))?;
        let voice: Vec<f32> = bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| f32::from_le_bytes(*b))
            .collect();
        if !voice.len().is_multiple_of(STYLE) {
            bail!("voice file has an unexpected size");
        }
        Ok(Self {
            session,
            vocab,
            voice,
            speed: 1.0,
        })
    }

    /// Audio for one sentence or short paragraph.
    ///
    /// # Errors
    /// Fails when phonemizing or the model fails.
    pub fn synthesize(&mut self, text: &str) -> Result<Vec<f32>> {
        let phonemes = phonemize(text)?;
        let mut ids: Vec<i64> = phonemes
            .chars()
            .filter_map(|c| self.vocab.get(&c).copied())
            .collect();
        ids.truncate(MAX_TOKENS - 2);
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let styles = self.voice.len() / STYLE;
        let row = ids.len().min(styles - 1);
        let style = self.voice[row * STYLE..(row + 1) * STYLE].to_vec();
        let n = ids.len();
        let mut input = Vec::with_capacity(n + 2);
        input.push(0);
        input.extend(ids);
        input.push(0);

        let outputs = self
            .session
            .run(ort::inputs![
                "input_ids" => Tensor::from_array(([1_usize, n + 2], input))?,
                "style" => Tensor::from_array(([1_usize, STYLE], style))?,
                "speed" => Tensor::from_array(([1_usize], vec![self.speed]))?,
            ])
            .map_err(|e| anyhow!("Kokoro: {e}"))?;
        let (_, wave) = outputs["waveform"].try_extract_tensor::<f32>()?;
        Ok(wave.to_vec())
    }
}
