//! `config/agent.toml`: ports, device names, model repos, timeouts.
//! Only the settings in use are read; later phases add their fields here.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Config {
    pub paths: Paths,
    pub chrome: Chrome,
    pub audio: Audio,
    pub brain: Brain,
    /// Role → Hugging Face repo, e.g. `vad = "onnx-community/silero-vad"`.
    pub models: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
pub struct Paths {
    pub hf_cache: String,
}

#[derive(Debug, Deserialize)]
pub struct Chrome {
    pub binary: String,
    pub profile_dir: String,
    pub debug_port: u16,
}

#[derive(Debug, Deserialize)]
pub struct Audio {
    pub call_input: String,
    pub agent_output: String,
}

#[derive(Debug, Deserialize)]
pub struct Brain {
    pub model: String,
    pub port: u16,
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))
    }
}

/// Expands a leading `~/` to the home directory.
pub fn expand(path: &str) -> PathBuf {
    match (path.strip_prefix("~/"), std::env::var_os("HOME")) {
        (Some(rest), Some(home)) => PathBuf::from(home).join(rest),
        _ => PathBuf::from(path),
    }
}
