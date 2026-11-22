//! `meet-agent login`: opens the agent's Chrome so you sign in to Google once.

use std::time::Duration;

use crate::config::Config;
use anyhow::{Context, Result, bail};

pub fn run(cfg: &Config) -> Result<()> {
    let chrome = cfg.chrome();
    if chrome.wait_for_port(Duration::ZERO) {
        bail!(
            "port {} is already in use; is the agent's Chrome already open? Quit it and try again",
            chrome.debug_port
        );
    }
    let mut child = chrome
        .launch("https://accounts.google.com/")
        .context("starting Chrome")?;
    if !chrome.wait_for_port(Duration::from_secs(15)) {
        bail!(
            "Chrome started but its control port {} never opened",
            chrome.debug_port
        );
    }

    println!(
        "Agent Chrome is open (profile: {}).",
        cfg.chrome.profile_dir
    );
    println!("1. Sign in to the agent's Google account.");
    println!("2. Open https://meet.google.com once and allow camera and microphone if asked.");
    println!("3. Quit Chrome (Cmd+Q) to finish.");

    child.wait().context("waiting for Chrome to close")?;
    println!("Chrome closed. The sign-in is saved in the profile.");
    Ok(())
}
