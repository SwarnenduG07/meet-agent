//! `meet-agent join <link>`: the agent's Chrome joins a meeting. No AI.

use std::{path::Path, time::Duration};

use anyhow::{Context, Result, bail};
use meet::{
    call::{Call, Entry},
    labels::Labels,
    session::Session,
};

use crate::config::Config;

pub struct Options {
    pub dry_run: bool,
    pub share: bool,
    /// App, file or URL to bring forward once sharing starts.
    pub show: Option<String>,
    pub stay: Duration,
}

/// Starts the agent's Chrome if needed and attaches to it.
pub async fn attach(cfg: &Config) -> Result<Session> {
    let chrome = cfg.chrome();
    if !chrome.wait_for_port(Duration::ZERO) {
        chrome
            .launch("about:blank")
            .context("starting the agent's Chrome")?;
        if !chrome.wait_for_port(Duration::from_secs(15)) {
            bail!(
                "Chrome started but its control port {} never opened",
                chrome.debug_port
            );
        }
    }
    let session = Session::attach(chrome.debug_port).await?;
    session.grant_media().await?;
    Ok(session)
}

/// Opens the link and gets the agent into the call (unless it's a dry run).
pub async fn enter(
    cfg: &Config,
    session: &Session,
    link: &str,
    dry_run: bool,
) -> Result<Option<Call>> {
    let labels = Labels::load(Path::new(&cfg.meet.labels))?;
    let (call, entry) = Call::open(session, link, labels).await?;
    match entry {
        Entry::InCall if dry_run => {
            call.abandon().await;
            println!("Meet joined straight away (no green room); left again because of --dry-run.");
            Ok(None)
        }
        Entry::InCall => {
            println!("Meet joined straight away (no green room).");
            Ok(Some(call))
        }
        Entry::GreenRoom { .. } if dry_run => {
            println!("Green room reached. Visible buttons:");
            for c in meet::session::controls(call.page())
                .await
                .unwrap_or_default()
            {
                println!("  label={:?} text={:?}", c.label, c.text);
            }
            call.abandon().await;
            println!("Dry run: closed the tab without joining.");
            Ok(None)
        }
        Entry::GreenRoom { can_join_now } => {
            let prepared = call
                .prepare(&cfg.audio.agent_output, &cfg.audio.meet_speaker)
                .await;
            let (mic, speaker) = match prepared {
                Ok(p) => p,
                Err(e) => {
                    call.abandon().await;
                    return Err(e);
                }
            };
            println!("Camera off. Microphone: {mic}");
            match speaker {
                Ok(s) => println!("Speaker: {s}"),
                Err(e) => println!(
                    "Speaker unchanged ({e}). Create the \"Meet Output\" device so the agent hears the call."
                ),
            }
            let what = if can_join_now {
                "Joining"
            } else {
                "Asking to join"
            };
            println!(
                "{what}; waiting up to {}s to be let in...",
                cfg.meet.admit_timeout_secs
            );
            if let Err(e) = call
                .enter(
                    can_join_now,
                    Duration::from_secs(cfg.meet.admit_timeout_secs),
                )
                .await
            {
                call.abandon().await;
                return Err(e);
            }
            println!("In the call.");
            Ok(Some(call))
        }
    }
}

pub async fn run(cfg: &Config, link: &str, opts: Options) -> Result<()> {
    let session = attach(cfg).await?;
    let Some(call) = enter(cfg, &session, link, opts.dry_run).await? else {
        return Ok(());
    };
    let result = present_and_wait(&call, &opts).await;
    call.leave().await?;
    println!("Left the call.");
    result
}

async fn present_and_wait(call: &Call, opts: &Options) -> Result<()> {
    if opts.share {
        call.share().await?;
        println!("Sharing the screen.");
        if let Some(target) = &opts.show {
            desktop::windows::show(target)?;
            println!("Brought {target:?} to the front.");
        }
    }
    println!("Staying {}s.", opts.stay.as_secs());
    tokio::time::sleep(opts.stay).await;
    if opts.share {
        call.stop_sharing().await?;
        println!("Stopped presenting.");
    }
    Ok(())
}
