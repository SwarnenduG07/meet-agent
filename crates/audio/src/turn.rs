//! Smart Turn v3: has the speaker finished their turn? Looks at the last 8 s.

use std::path::Path;

use anyhow::{Result, anyhow};
use ort::{session::Session, value::Tensor};

use crate::{
    SAMPLE_RATE,
    mel::{LogMel, N_MELS},
    onnx,
};

const SECONDS: usize = 8;
const FRAMES: usize = SECONDS * 100;

pub struct SmartTurn {
    session: Session,
    mel: LogMel,
}

impl SmartTurn {
    /// # Errors
    /// Fails when the model can't be loaded.
    pub fn load(model: &Path) -> Result<Self> {
        Ok(Self {
            session: onnx::session(model, 2)?,
            mel: LogMel::new(),
        })
    }

    /// Probability (0-1) that the speaker has finished.
    ///
    /// # Errors
    /// Fails when the model fails.
    pub fn done_prob(&mut self, audio: &[f32]) -> Result<f32> {
        let max = SECONDS * SAMPLE_RATE as usize;
        let audio = &audio[audio.len().saturating_sub(max)..];
        let normalized = normalize(audio);
        let features = self.mel.features(&normalized, FRAMES);
        let outputs = self
            .session
            .run(ort::inputs!["input_features" => Tensor::from_array(([1_usize, N_MELS, FRAMES], features))?])
            .map_err(|e| anyhow!("Smart Turn: {e}"))?;
        let (_, p) = outputs[0].try_extract_tensor::<f32>()?;
        Ok(p[0])
    }
}

/// Zero mean, unit variance, as `WhisperFeatureExtractor(do_normalize=True)`.
fn normalize(audio: &[f32]) -> Vec<f32> {
    if audio.is_empty() {
        return Vec::new();
    }
    let n = audio.len() as f64;
    let mean = audio.iter().map(|&x| f64::from(x)).sum::<f64>() / n;
    let var = audio
        .iter()
        .map(|&x| (f64::from(x) - mean).powi(2))
        .sum::<f64>()
        / n;
    let scale = 1.0 / (var + 1e-7).sqrt();
    audio
        .iter()
        .map(|&x| ((f64::from(x) - mean) * scale) as f32)
        .collect()
}
