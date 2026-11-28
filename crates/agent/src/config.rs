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
    pub meet: Meet,
    pub speech: Speech,
    /// Role → model file in the Hugging Face cache.
    pub models: BTreeMap<String, ModelFile>,
}

#[derive(Debug, Deserialize)]
pub struct ModelFile {
    pub repo: String,
    pub file: String,
}

#[derive(Debug, Deserialize)]
pub struct Speech {
    pub voice: String,
    pub tts_threads: usize,
    pub expect_words: String,
}

#[derive(Debug, Deserialize)]
pub struct Paths {
    pub hf_cache: String,
    pub runs: String,
}

#[derive(Debug, Deserialize)]
pub struct Chrome {
    pub binary: String,
    pub profile_dir: String,
    pub debug_port: u16,
    pub share_source: String,
}

#[derive(Debug, Deserialize)]
pub struct Audio {
    pub call_input: String,
    pub agent_output: String,
    pub meet_speaker: String,
    pub monitor_output: String,
    pub listen_input: String,
    pub your_mic: String,
}

#[derive(Debug, Deserialize)]
pub struct Meet {
    pub labels: String,
    pub admit_timeout_secs: u64,
}

#[derive(Debug, Deserialize)]
pub struct Brain {
    pub project: String,
    pub model: String,
    pub port: u16,
    pub startup_timeout_secs: u64,
    pub name: String,
    pub owner: String,
}

impl Config {
    /// The agent's Chrome: binary, profile and control port.
    pub fn chrome(&self) -> meet::chrome::Chrome {
        meet::chrome::Chrome {
            binary: self.chrome.binary.clone().into(),
            profile_dir: expand(&self.chrome.profile_dir),
            debug_port: self.chrome.debug_port,
            share_source: self.chrome.share_source.clone(),
        }
    }

    /// How to start the Python brain server.
    pub fn brain(&self) -> brain_client::Settings {
        brain_client::Settings {
            project_dir: expand(&self.brain.project),
            model: self.brain.model.clone(),
            port: self.brain.port,
            startup_timeout: std::time::Duration::from_secs(self.brain.startup_timeout_secs),
            name: self.brain.name.clone(),
            owner: self.brain.owner.clone(),
        }
    }

    /// Path of a model file for `role` (e.g. "vad") in the Hugging Face cache.
    pub fn model_path(&self, role: &str) -> Result<PathBuf> {
        let m = self
            .models
            .get(role)
            .with_context(|| format!("no [models] entry for {role:?}"))?;
        Ok(snapshot_dir(&expand(&self.paths.hf_cache), &m.repo)?.join(&m.file))
    }

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

/// The downloaded snapshot folder of a Hugging Face repo.
pub fn snapshot_dir(cache: &Path, repo: &str) -> Result<PathBuf> {
    let dir = cache
        .join(format!("models--{}", repo.replace('/', "--")))
        .join("snapshots");
    std::fs::read_dir(&dir)
        .with_context(|| format!("{repo} is not downloaded (run `just models`)"))?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .find(|p| p.is_dir())
        .with_context(|| format!("{repo} has no snapshot in {}", dir.display()))
}

/// Empty string in the config means "the system default device".
pub fn device(name: &str) -> Option<String> {
    (!name.trim().is_empty()).then(|| name.to_owned())
}
