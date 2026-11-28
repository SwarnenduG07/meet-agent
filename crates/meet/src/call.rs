//! One Google Meet call: green room, join, present, stop presenting, leave.
//! The orchestrator drives these steps; `meet-agent join` runs them in order.

use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use chromiumoxide::Page;

use crate::{
    labels::Labels,
    session::{self, Session},
};

/// Texts Meet shows when the agent can't get in.
const REFUSED: [&str; 4] = [
    "denied your request",
    "can't join this call",
    "No one responded to your request",
    "You've been removed from the meeting",
];

/// Where Meet put the agent after opening the link.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Entry {
    /// The green room; `can_join_now` is true for the host, false for guests.
    GreenRoom { can_join_now: bool },
    /// Meet skipped the green room (the agent's account hosts this meeting).
    InCall,
}

pub struct Call {
    page: Page,
    labels: Labels,
}

impl Call {
    /// Opens `link` and waits for the green room or the call.
    ///
    /// # Errors
    /// Fails when neither appears within 30 s or Google asks to sign in.
    pub async fn open(session: &Session, link: &str, labels: Labels) -> Result<(Self, Entry)> {
        let page = session.open(link).await?;
        let names = [
            labels.join_now.as_str(),
            labels.ask_to_join.as_str(),
            labels.leave_call.as_str(),
        ];
        let controls = match session::wait_for_any(&page, &names, Duration::from_secs(30)).await {
            Ok(c) => c,
            Err(e) => {
                let _ = page.close().await;
                return Err(e);
            }
        };
        let has = |l: &str| controls.iter().any(|c| c.is(l));
        let entry = if has(&labels.leave_call) {
            Entry::InCall
        } else {
            Entry::GreenRoom {
                can_join_now: has(&labels.join_now),
            }
        };
        Ok((Self { page, labels }, entry))
    }

    #[must_use]
    pub fn page(&self) -> &Page {
        &self.page
    }

    /// Clicks away known tips and popups that can cover the controls.
    pub async fn dismiss_popups(&self) {
        for name in &self.labels.dismiss {
            let _ = session::click(&self.page, name).await;
        }
    }

    /// Green room: camera off, then the agent's mic and speaker. Returns the chosen
    /// mic and the speaker, or why the speaker stayed unchanged.
    ///
    /// # Errors
    /// Fails when the microphone can't be set; the speaker is best effort.
    pub async fn prepare(&self, mic: &str, speaker: &str) -> Result<(String, Result<String>)> {
        self.dismiss_popups().await;
        let _ = session::click(&self.page, &self.labels.turn_off_camera).await;
        let mic = session::select_device(&self.page, "Microphone:", mic).await?;
        let speaker = session::select_device(&self.page, "Speaker:", speaker).await;
        Ok((mic, speaker))
    }

    /// Clicks Join now or Ask to join and waits until the agent is in the call.
    ///
    /// # Errors
    /// Fails when the host denies the request or nobody admits within `timeout`.
    pub async fn enter(&self, can_join_now: bool, timeout: Duration) -> Result<()> {
        self.dismiss_popups().await;
        let button = if can_join_now {
            &self.labels.join_now
        } else {
            &self.labels.ask_to_join
        };
        session::click(&self.page, button).await?;
        let start = Instant::now();
        loop {
            if self.in_call().await {
                self.dismiss_popups().await;
                return Ok(());
            }
            let text = session::body_text(&self.page).await.unwrap_or_default();
            if let Some(why) = REFUSED.iter().find(|r| text.contains(**r)) {
                bail!("not let in: Meet says {why:?}");
            }
            if start.elapsed() > timeout {
                bail!("nobody let the agent in within {}s", timeout.as_secs());
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }

    /// True while the in-call controls are on screen.
    pub async fn in_call(&self) -> bool {
        session::controls(&self.page)
            .await
            .is_ok_and(|cs| cs.iter().any(|c| c.is(&self.labels.leave_call)))
    }

    /// True while Meet shows that we present.
    pub async fn is_presenting(&self) -> bool {
        if session::controls(&self.page)
            .await
            .is_ok_and(|cs| cs.iter().any(|c| c.is(&self.labels.stop_presenting)))
        {
            return true;
        }
        let text = session::body_text(&self.page).await.unwrap_or_default();
        self.labels
            .presenting_texts
            .iter()
            .any(|t| text.contains(t.as_str()))
    }

    /// Clicks Share screen. Chrome's auto-select flag picks the screen, so no picker
    /// needs a click. Waits until Meet shows we're presenting.
    ///
    /// # Errors
    /// Fails when presenting hasn't started after 20 s.
    pub async fn share(&self) -> Result<()> {
        if self.is_presenting().await {
            return Ok(());
        }
        self.dismiss_popups().await;
        session::click(&self.page, &self.labels.present).await?;
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(20) {
            if self.is_presenting().await {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        let seen = session::controls(&self.page).await.unwrap_or_default();
        let seen: Vec<_> = seen
            .iter()
            .map(|c| format!("{:?}/{:?}", c.label, c.text))
            .collect();
        bail!(
            "sharing did not start within 20s (Chrome may lack Screen Recording permission, \
             or share_source doesn't name a screen); visible buttons: {seen:?}"
        )
    }

    /// Stops presenting: Stop presenting if shown, otherwise the share button again.
    ///
    /// # Errors
    /// Fails when Meet still shows we're presenting after 10 s.
    pub async fn stop_sharing(&self) -> Result<()> {
        if !self.is_presenting().await {
            return Ok(());
        }
        if session::click(&self.page, &self.labels.stop_presenting)
            .await
            .is_err()
        {
            session::click(&self.page, &self.labels.present)
                .await
                .context("no Stop presenting button")?;
        }
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(10) {
            if !self.is_presenting().await {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        bail!("Meet still shows we're presenting")
    }

    /// Clicks Rejoin if Meet shows it (after a network drop). True if clicked.
    pub async fn rejoin(&self) -> bool {
        session::click(&self.page, &self.labels.rejoin)
            .await
            .is_ok()
    }

    /// Switches the microphone during the call (e.g. to the user's real mic).
    ///
    /// # Errors
    /// Fails when the device isn't in Meet's audio list.
    pub async fn switch_mic(&self, device: &str) -> Result<String> {
        session::select_device(&self.page, &self.labels.in_call_audio, device).await
    }

    /// Leaves the call and closes the tab.
    ///
    /// # Errors
    /// Fails when the tab can't be closed.
    pub async fn leave(self) -> Result<()> {
        let _ = self.stop_sharing().await;
        let _ = session::click(&self.page, &self.labels.leave_call).await;
        tokio::time::sleep(Duration::from_secs(1)).await;
        self.page.close().await.context("closing the Meet tab")?;
        Ok(())
    }

    /// Closes the tab without the polite steps (for dry runs and failures).
    pub async fn abandon(self) {
        if self.in_call().await {
            let _ = session::click(&self.page, &self.labels.leave_call).await;
        }
        let _ = self.page.close().await;
    }
}
