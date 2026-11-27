//! Silero VAD: probability that a 32 ms chunk contains speech.

use std::path::Path;

use anyhow::{Result, anyhow};
use ort::{session::Session, value::Tensor};

use crate::{CHUNK, SAMPLE_RATE, onnx};

const CONTEXT: usize = 64;

pub struct Vad {
    session: Session,
    state: Vec<f32>,
    context: Vec<f32>,
}

impl Vad {
    /// # Errors
    /// Fails when the model can't be loaded.
    pub fn load(model: &Path) -> Result<Self> {
        Ok(Self {
            session: onnx::session(model, 1)?,
            state: vec![0.0; 2 * 128],
            context: vec![0.0; CONTEXT],
        })
    }

    /// Forget previous audio (after a turn ends).
    pub fn reset(&mut self) {
        self.state.fill(0.0);
        self.context.fill(0.0);
    }

    /// Speech probability for one chunk of [`CHUNK`] samples.
    ///
    /// # Errors
    /// Fails when the chunk has the wrong size or the model fails.
    pub fn speech_prob(&mut self, chunk: &[f32]) -> Result<f32> {
        if chunk.len() != CHUNK {
            return Err(anyhow!("VAD expects {CHUNK} samples, got {}", chunk.len()));
        }
        let mut input = Vec::with_capacity(CONTEXT + CHUNK);
        input.extend_from_slice(&self.context);
        input.extend_from_slice(chunk);
        self.context.copy_from_slice(&chunk[CHUNK - CONTEXT..]);

        let outputs = self
            .session
            .run(ort::inputs![
                "input" => Tensor::from_array(([1_usize, CONTEXT + CHUNK], input))?,
                "state" => Tensor::from_array(([2_usize, 1, 128], self.state.clone()))?,
                "sr" => Tensor::from_array(((), vec![i64::from(SAMPLE_RATE)]))?,
            ])
            .map_err(|e| anyhow!("VAD: {e}"))?;
        let (_, prob) = outputs["output"].try_extract_tensor::<f32>()?;
        let p = prob[0];
        let (_, state) = outputs["stateN"].try_extract_tensor::<f32>()?;
        self.state.copy_from_slice(state);
        Ok(p)
    }
}
