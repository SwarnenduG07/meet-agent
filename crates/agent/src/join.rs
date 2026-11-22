//! `meet-agent join <link>`: the agent's Chrome joins a meeting. No AI.

use std::{path::Path, time::Duration};

use anyhow::{Context, Result, bail};
use chromiumoxide::Page;
use meet::{
    labels::Labels,
    session::{self, Control},
};

use crate::config::Config;

/// Texts Meet shows when the agent can't get in.
const REFUSED: [&str; 4] = [
    "denied your request",
    "can't join this call",
    "No one responded to your request",
    "You've been removed from the meeting",
];

pub async fn run(
    cfg: &Config,
    link: &str,
    dry_run: bool,
    share: bool,
    stay: Duration,
) -> Result<()> {
    let labels = Labels::load(Path::new(&cfg.meet.labels))?;
    let chrome = cfg.chrome();
    if chrome.wait_for_port(Duration::ZERO) {
        println!("Using the agent's Chrome that is already open.");
    } else {
        chrome
            .launch("about:blank")
            .context("starting the agent's Chrome")?;
        if !chrome.wait_for_port(Duration::from_secs(15)) {
            bail!(
                "Chrome started but its control port {} never opened",
                chrome.debug_port
            );
        }
        println!("Started the agent's Chrome.");
    }

    let s = session::Session::attach(chrome.debug_port).await?;
    s.grant_media().await?;
    println!("Attached over CDP; Meet may use mic and camera.");

    let page = s.open(link).await?;
    println!("Opened {link}, waiting for the green room...");
    let result = join(cfg, &labels, &page, dry_run, share, stay).await;
    if result.is_err() {
        // Never leave the agent half-joined or the camera on after a failure.
        let _ = session::click(&page, &labels.leave_call).await;
        let _ = page.close().await;
    }
    result
}

async fn join(
    cfg: &Config,
    labels: &Labels,
    page: &Page,
    dry_run: bool,
    share: bool,
    stay: Duration,
) -> Result<()> {
    let names = [
        labels.join_now.as_str(),
        labels.ask_to_join.as_str(),
        labels.leave_call.as_str(),
    ];
    let controls = session::wait_for_any(page, &names, Duration::from_secs(30)).await?;
    let has = |label: &str| controls.iter().any(|c: &Control| c.is(label));

    if has(&labels.leave_call) {
        // Meet skipped the green room (the agent's account hosts this meeting).
        if dry_run {
            session::click(page, &labels.leave_call).await?;
            page.clone().close().await?;
            println!("Meet joined straight away (no green room); left again because of --dry-run.");
            return Ok(());
        }
        println!("Meet joined straight away (no green room).");
    } else {
        println!("Green room reached. Visible buttons:");
        for c in &controls {
            println!("  label={:?} text={:?}", c.label, c.text);
        }
        if dry_run {
            page.clone().close().await.context("closing the Meet tab")?;
            println!("Dry run: closed the tab without joining.");
            return Ok(());
        }
        prepare_and_ask(cfg, labels, page, has(&labels.join_now)).await?;
    }

    if share {
        share_screen(labels, page).await?;
        println!(
            "Sharing the entire screen. Staying {}s, then stopping.",
            stay.as_secs()
        );
        tokio::time::sleep(stay).await;
        session::click(page, &labels.stop_presenting).await?;
        println!("Stopped presenting.");
        tokio::time::sleep(Duration::from_secs(2)).await;
    } else {
        println!("In the call. Staying {}s, then leaving.", stay.as_secs());
        tokio::time::sleep(stay).await;
    }
    session::click(page, &labels.leave_call).await?;
    tokio::time::sleep(Duration::from_secs(2)).await;
    page.clone().close().await?;
    println!("Left the call.");
    Ok(())
}

/// Camera off, devices set, then Join now / Ask to join and wait to be admitted.
async fn prepare_and_ask(
    cfg: &Config,
    labels: &Labels,
    page: &Page,
    can_join_now: bool,
) -> Result<()> {
    if session::click(page, &labels.turn_off_camera).await.is_ok() {
        println!("Camera off.");
    }
    let mic = session::select_device(page, "Microphone:", &cfg.audio.agent_output).await?;
    println!("Microphone: {mic}");
    match session::select_device(page, "Speaker:", &cfg.audio.meet_speaker).await {
        Ok(speaker) => println!("Speaker: {speaker}"),
        Err(e) => println!(
            "Speaker unchanged ({e}). Create the Multi-Output device so the agent hears the call."
        ),
    }

    let button = if can_join_now {
        &labels.join_now
    } else {
        &labels.ask_to_join
    };
    session::click(page, button).await?;
    println!(
        "Clicked {button:?}. Waiting up to {}s to be let in...",
        cfg.meet.admit_timeout_secs
    );

    let timeout = Duration::from_secs(cfg.meet.admit_timeout_secs);
    let start = std::time::Instant::now();
    loop {
        let found = session::controls(page).await.unwrap_or_default();
        if found.iter().any(|c| c.is(&labels.leave_call)) {
            return Ok(());
        }
        let text = session::body_text(page).await.unwrap_or_default();
        if let Some(why) = REFUSED.iter().find(|r| text.contains(**r)) {
            bail!("not let in: Meet says {why:?}");
        }
        if start.elapsed() > timeout {
            bail!("nobody let the agent in within {}s", timeout.as_secs());
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

/// Clicks Share screen; Chrome's `--auto-select-desktop-capture-source` flag picks
/// the source, so no picker needs a click. Waits until Meet shows we're presenting.
async fn share_screen(labels: &Labels, page: &Page) -> Result<()> {
    session::click(page, &labels.present).await?;
    println!(
        "Clicked {:?}, waiting for Meet to start presenting...",
        labels.present
    );
    session::wait_for_any(
        page,
        &[labels.stop_presenting.as_str()],
        Duration::from_secs(20),
    )
    .await
    .context(
        "sharing did not start; if macOS asked about Screen Recording, allow Chrome and retry",
    )?;
    Ok(())
}
