//! One ONNX Runtime environment for the whole process.

use std::{path::Path, sync::OnceLock};

use anyhow::{Context, Result, anyhow};
use ort::{environment::Environment, session::Session};

static ENV: OnceLock<Environment> = OnceLock::new();

/// Loads an ONNX model on the CPU with a small thread budget.
///
/// # Errors
/// Fails when the file is missing or isn't a valid model.
pub fn session(path: &Path, threads: usize) -> Result<Session> {
    if ENV.get().is_none() {
        let env = ort::init()
            .with_name("meet-agent")
            .build()
            .map_err(|e| anyhow!("onnxruntime: {e}"))?;
        let _ = ENV.set(env);
    }
    let env = ENV.get().context("onnxruntime environment")?;
    Session::builder(env)
        .map_err(|e| anyhow!("{e}"))?
        .with_intra_threads(threads)
        .map_err(|e| anyhow!("{e}"))?
        .commit_from_file(path)
        .map_err(|e| anyhow!("{e}"))
        .with_context(|| format!("loading {}", path.display()))
}
