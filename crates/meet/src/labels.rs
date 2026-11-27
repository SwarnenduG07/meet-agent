//! Meet button labels from `config/meet-labels.toml`. Meet renames buttons from time
//! to time; when joining breaks, the fix is one line in that file.

use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct Labels {
    pub join_now: String,
    pub ask_to_join: String,
    pub turn_off_camera: String,
    pub present: String,
    pub stop_presenting: String,
    pub leave_call: String,
    pub in_call_audio: String,
    pub presenting_texts: Vec<String>,
    pub dismiss: Vec<String>,
}

impl Labels {
    /// # Errors
    /// Fails when the file is missing or a label is missing.
    pub fn load(path: &Path) -> Result<Self> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))
    }
}
