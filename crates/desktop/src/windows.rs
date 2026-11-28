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

/// Types `text` into the front app (needs Accessibility permission).
///
/// # Errors
/// Fails when System Events refuses.
pub fn type_text(text: &str) -> Result<()> {
    let script = format!(
        "tell application \"System Events\" to keystroke {}",
        applescript_string(text)
    );
    run("osascript", &["-e", &script])
}

/// Presses a shortcut like `cmd+t`, `cmd+shift+4`, `return` or `escape`.
///
/// # Errors
/// Fails for an unknown key or when System Events refuses.
pub fn keys(combo: &str) -> Result<()> {
    let mut parts: Vec<&str> = combo.split('+').map(str::trim).collect();
    let key = parts.pop().unwrap_or_default();
    let modifiers: Vec<&str> = parts
        .iter()
        .map(|m| match m.to_ascii_lowercase().as_str() {
            "cmd" | "command" => Ok("command down"),
            "shift" => Ok("shift down"),
            "alt" | "option" | "opt" => Ok("option down"),
            "ctrl" | "control" => Ok("control down"),
            other => bail!("unknown modifier {other:?} in {combo:?}"),
        })
        .collect::<Result<_>>()?;
    let press = match key.to_ascii_lowercase().as_str() {
        "return" | "enter" => "key code 36".to_owned(),
        "tab" => "key code 48".to_owned(),
        "escape" | "esc" => "key code 53".to_owned(),
        "space" => "key code 49".to_owned(),
        "left" => "key code 123".to_owned(),
        "right" => "key code 124".to_owned(),
        "down" => "key code 125".to_owned(),
        "up" => "key code 126".to_owned(),
        k if k.chars().count() == 1 => format!("keystroke {}", applescript_string(k)),
        other => bail!("unknown key {other:?} in {combo:?}"),
    };
    let using = if modifiers.is_empty() {
        String::new()
    } else {
        format!(" using {{{}}}", modifiers.join(", "))
    };
    run(
        "osascript",
        &[
            "-e",
            &format!("tell application \"System Events\" to {press}{using}"),
        ],
    )
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
