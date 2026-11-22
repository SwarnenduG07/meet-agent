//! Attaches to the agent's running Chrome over CDP and works with a Meet tab.

use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use chromiumoxide::{
    Browser, Page,
    cdp::browser_protocol::browser::{
        PermissionDescriptor, PermissionSetting, SetPermissionParams,
    },
};
use futures::StreamExt;
use serde::Deserialize;
use tokio::task::JoinHandle;

const MEET_ORIGIN: &str = "https://meet.google.com";

/// A visible button on the page: its accessible label and its text.
#[derive(Debug, Clone, Deserialize)]
pub struct Control {
    pub label: String,
    pub text: String,
}

impl Control {
    /// True when the label or the visible text matches `name` (case-insensitive).
    #[must_use]
    pub fn is(&self, name: &str) -> bool {
        self.label.eq_ignore_ascii_case(name) || self.text.eq_ignore_ascii_case(name)
    }
}

/// Lists visible buttons, with `aria-label` and trimmed text.
const LIST_CONTROLS_JS: &str = r"
[...document.querySelectorAll('button, [role=button]')]
  .filter(e => e.offsetParent !== null)
  .map(e => ({
    label: (e.getAttribute('aria-label') || '').trim(),
    text: (e.innerText || '').trim().split('\n')[0].slice(0, 60),
  }))
";

pub struct Session {
    browser: Browser,
    handler: JoinHandle<()>,
}

impl Session {
    /// Connects to Chrome's control port on 127.0.0.1.
    ///
    /// # Errors
    /// Fails when nothing answers on the port.
    pub async fn attach(port: u16) -> Result<Self> {
        let (browser, mut events) = Browser::connect(format!("http://127.0.0.1:{port}"))
            .await
            .with_context(|| format!("attaching to Chrome on port {port}"))?;
        let handler = tokio::spawn(async move { while events.next().await.is_some() {} });
        Ok(Self { browser, handler })
    }

    /// Lets Meet use the microphone and camera without a permission popup.
    ///
    /// # Errors
    /// Fails when Chrome rejects the command.
    pub async fn grant_media(&self) -> Result<()> {
        for name in ["microphone", "camera"] {
            let params = SetPermissionParams::builder()
                .permission(PermissionDescriptor::new(name))
                .setting(PermissionSetting::Granted)
                .origin(MEET_ORIGIN)
                .build()
                .map_err(anyhow::Error::msg)?;
            self.browser
                .execute(params)
                .await
                .with_context(|| format!("allowing {name} for Meet"))?;
        }
        Ok(())
    }

    /// Opens `link` in a new tab and returns it.
    ///
    /// # Errors
    /// Fails when the tab can't be opened.
    pub async fn open(&self, link: &str) -> Result<Page> {
        let page = self
            .browser
            .new_page(link)
            .await
            .with_context(|| format!("opening {link}"))?;
        Ok(page)
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        // Detaches from Chrome; Chrome itself keeps running.
        self.handler.abort();
    }
}

/// Visible buttons on the page right now.
///
/// # Errors
/// Fails when the page can't run the script.
pub async fn controls(page: &Page) -> Result<Vec<Control>> {
    page.evaluate(LIST_CONTROLS_JS)
        .await
        .context("listing buttons")?
        .into_value()
        .context("reading button list")
}

/// Waits until one of `names` is visible and returns all visible buttons.
///
/// # Errors
/// Fails on timeout, or when Google shows its sign-in page instead of Meet.
pub async fn wait_for_any(page: &Page, names: &[&str], timeout: Duration) -> Result<Vec<Control>> {
    let start = Instant::now();
    loop {
        let url = page.url().await?.unwrap_or_default();
        if url.starts_with("https://accounts.google.com") {
            bail!("Google asks the agent to sign in again; run `just login`");
        }
        let found = controls(page).await.unwrap_or_default();
        if found.iter().any(|c| names.iter().any(|n| c.is(n))) {
            return Ok(found);
        }
        if start.elapsed() > timeout {
            let seen: Vec<_> = found
                .iter()
                .map(|c| format!("{:?}/{:?}", c.label, c.text))
                .collect();
            bail!(
                "none of {names:?} appeared within {timeout:?} on {url}; visible buttons: {seen:?}"
            );
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

/// Clicks the visible button whose label or text is `name`.
///
/// # Errors
/// Fails when no such button is visible.
pub async fn click(page: &Page, name: &str) -> Result<()> {
    let js = format!(
        r"(() => {{
            const want = {name}.toLowerCase();
            const b = [...document.querySelectorAll('button, [role=button]')]
              .filter(e => e.offsetParent !== null)
              .find(e => (e.getAttribute('aria-label') || '').trim().toLowerCase() === want
                      || (e.innerText || '').trim().split('\n')[0].toLowerCase() === want);
            if (!b) return false;
            b.click();
            return true;
        }})()",
        name = serde_json::to_string(name)?
    );
    let clicked: bool = page.evaluate(js).await.context("clicking")?.into_value()?;
    if !clicked {
        bail!("button {name:?} is not on the screen");
    }
    Ok(())
}

/// Result of trying to choose a device in an open picker menu.
#[derive(Deserialize)]
struct Pick {
    chosen: Option<String>,
    names: Vec<String>,
}

/// Opens the device picker whose label starts with `picker` (e.g. `"Microphone:"`)
/// and chooses the device whose name starts with `device`. Returns the chosen entry.
///
/// # Errors
/// Fails when the picker never appears or the device isn't in its list.
pub async fn select_device(page: &Page, picker: &str, device: &str) -> Result<String> {
    let open_js = format!(
        r"(() => {{
            const b = [...document.querySelectorAll('button, [role=button]')]
              .filter(e => e.offsetParent !== null)
              .find(e => (e.getAttribute('aria-label') || '').startsWith({picker}));
            if (!b) return false;
            b.click();
            return true;
        }})()",
        picker = serde_json::to_string(picker)?
    );
    let pick_js = format!(
        r"(() => {{
            const items = [...document.querySelectorAll('[role=menuitemradio]')].filter(e => e.offsetParent !== null);
            const names = items.map(e => (e.innerText || '').trim().split('\n')[0]);
            const i = names.findIndex(n => n.startsWith({device}));
            if (i < 0) return {{ chosen: null, names }};
            items[i].click();
            return {{ chosen: names[i], names }};
        }})()",
        device = serde_json::to_string(device)?
    );

    let start = Instant::now();
    loop {
        if page
            .evaluate(open_js.as_str())
            .await?
            .into_value::<bool>()?
        {
            break;
        }
        if start.elapsed() > Duration::from_secs(10) {
            bail!("the {picker:?} picker never appeared");
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }

    let mut last = Vec::new();
    for _ in 0..10 {
        tokio::time::sleep(Duration::from_millis(300)).await;
        let pick: Pick = page.evaluate(pick_js.as_str()).await?.into_value()?;
        if let Some(chosen) = pick.chosen {
            return Ok(chosen);
        }
        last = pick.names;
    }
    // Close the menu so it doesn't cover the Join button.
    page.evaluate("document.activeElement && document.activeElement.dispatchEvent(new KeyboardEvent('keydown', {key: 'Escape', bubbles: true}))")
        .await?;
    bail!("{device:?} is not in the {picker:?} list; options: {last:?}")
}

/// The page's visible text, for spotting messages like a denied join request.
///
/// # Errors
/// Fails when the page can't run the script.
pub async fn body_text(page: &Page) -> Result<String> {
    Ok(page
        .evaluate("document.body.innerText")
        .await?
        .into_value()?)
}
