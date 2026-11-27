//! Brings demo material to the front: an app, a file or a URL.

use std::{path::Path, process::Command};

use anyhow::{Context, Result, bail};

/// Shows `target` in front of everything else:
/// - a URL or an existing path is opened with its default app,
/// - anything else is treated as an app name and activated.
///
/// # Errors
/// Fails when macOS can't open or activate it.
pub fn show(target: &str) -> Result<()> {
    if target.contains("://") || Path::new(target).exists() {
        run("open", &[target])
    } else {
        activate(target)
    }
}

/// Activates an app by name, e.g. `"Visual Studio Code"`.
///
/// # Errors
/// Fails when the app isn't installed.
pub fn activate(app: &str) -> Result<()> {
    let script = format!("tell application {} to activate", applescript_string(app));
    run("osascript", &["-e", &script])
}

/// Opens `target` in a specific app, e.g. a URL in Safari.
///
/// # Errors
/// Fails when the app or target can't be opened.
pub fn open_in(app: &str, target: &str) -> Result<()> {
    run("open", &["-a", app, target])
}

fn run(cmd: &str, args: &[&str]) -> Result<()> {
    let out = Command::new(cmd)
        .args(args)
        .output()
        .with_context(|| format!("running {cmd}"))?;
    if !out.status.success() {
        bail!(
            "{cmd} {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

/// Quotes text for `AppleScript`.
fn applescript_string(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}
