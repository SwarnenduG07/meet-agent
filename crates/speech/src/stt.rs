//! Speech → text with Whisper large-v3-turbo through `whisper.cpp` (Apple GPU).

use std::path::Path;

use anyhow::{Context, Result, anyhow};
use whisper_rs::{
    FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters, WhisperState,
};

pub struct Stt {
    state: WhisperState,
    /// Words Whisper should expect (names, products), so "meet" isn't heard as "meat".
    prompt: String,
}

impl Stt {
    /// Loads the GGML model once; it stays on the GPU.
    ///
    /// # Errors
    /// Fails when the model file is missing or invalid.
    pub fn load(model: &Path, prompt: &str) -> Result<Self> {
        whisper_rs::install_logging_hooks(); // keep whisper.cpp's own logs out of the terminal
        let ctx = WhisperContext::new_with_params(model, WhisperContextParameters::default())
            .map_err(|e| anyhow!("{e:?}"))
            .with_context(|| format!("loading {}", model.display()))?;
        let state = ctx
            .create_state()
            .map_err(|e| anyhow!("whisper state: {e:?}"))?;
        Ok(Self {
            state,
            prompt: prompt.to_owned(),
        })
    }

    /// Text for 16 kHz mono audio.
    ///
    /// # Errors
    /// Fails when Whisper fails.
    pub fn transcribe(&mut self, audio: &[f32]) -> Result<String> {
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_language(Some("en"));
        params.set_initial_prompt(&self.prompt);
        params.set_no_context(true);
        params.set_suppress_nst(true);
        params.set_n_threads(4);
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        self.state
            .full(params, audio)
            .map_err(|e| anyhow!("whisper: {e:?}"))?;
        let text: Vec<String> = self
            .state
            .as_iter()
            .map(|seg| seg.to_string().trim().to_owned())
            .collect();
        Ok(text.join(" ").trim().to_owned())
    }
}
