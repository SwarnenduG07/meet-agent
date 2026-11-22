//! `meet-agent doctor`: checks everything the agent needs before a meeting.

use std::{net::TcpListener, path::Path, process::Command};

use crate::config::{Config, expand};

struct Report {
    failed: usize,
}

impl Report {
    fn check(&mut self, ok: bool, what: &str, fix: &str) {
        if ok {
            println!("  ok    {what}");
        } else {
            self.failed += 1;
            println!("  MISS  {what}\n        fix: {fix}");
        }
    }
}

fn on_path(tool: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|p| std::env::split_paths(&p).any(|dir| dir.join(tool).is_file()))
}

fn port_free(port: u16) -> bool {
    TcpListener::bind(("127.0.0.1", port)).is_ok()
}

fn audio_devices() -> String {
    Command::new("system_profiler")
        .arg("SPAudioDataType")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}

/// Returns the number of failed checks.
pub fn run(cfg: &Config) -> usize {
    let mut r = Report { failed: 0 };

    println!("Tools");
    r.check(
        Path::new(&cfg.chrome.binary).is_file(),
        "Google Chrome",
        "install Chrome or fix chrome.binary in config/agent.toml",
    );
    r.check(
        on_path("uv"),
        "uv (runs the Python brain)",
        "brew install uv",
    );
    r.check(
        on_path("espeak-ng"),
        "espeak-ng (Kokoro pronunciation, Phase 2b)",
        "brew install espeak-ng",
    );

    println!("Audio devices");
    let devices = audio_devices();
    for name in [&cfg.audio.call_input, &cfg.audio.agent_output] {
        r.check(
            devices.contains(name.as_str()),
            name,
            "brew install blackhole-2ch blackhole-16ch, then reboot",
        );
    }

    println!("Models in {}", cfg.paths.hf_cache);
    let cache = expand(&cfg.paths.hf_cache);
    let brain_role = "brain".to_string();
    for (role, repo) in std::iter::once((&brain_role, &cfg.brain.model)).chain(&cfg.models) {
        let dir = cache
            .join(format!("models--{}", repo.replace('/', "--")))
            .join("snapshots");
        r.check(
            dir.is_dir(),
            &format!("{role}: {repo}"),
            "run scripts/download-models.sh",
        );
    }

    println!("Local ports");
    r.check(
        port_free(cfg.chrome.debug_port),
        &format!("{} free for Chrome control", cfg.chrome.debug_port),
        "another program uses it; close it or change chrome.debug_port",
    );
    r.check(
        port_free(cfg.brain.port),
        &format!("{} free for the brain server", cfg.brain.port),
        "another program uses it; close it or change brain.port",
    );

    println!("Chrome profile");
    r.check(
        expand(&cfg.chrome.profile_dir).is_dir(),
        &format!("agent profile at {}", cfg.chrome.profile_dir),
        "`just login` arrives in Phase 1: it opens this profile so you sign in to the agent's Google account once",
    );

    println!();
    if r.failed == 0 {
        println!("All checks passed.");
    } else {
        println!("{} check(s) need attention.", r.failed);
    }
    r.failed
}
