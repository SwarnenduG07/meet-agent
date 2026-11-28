//! Pointing at things on the shared screen: screenshot → brain finds the target
//! (0-1000 grid) → points on the display → spotlight ring.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use brain_client::Brain;
use desktop::screen::{self, Display, Rect};

pub struct Pointer {
    overlay: PathBuf,
    /// Display that Meet shares (1-based).
    pub display: usize,
    pub info: Display,
    shots: PathBuf,
}

impl Pointer {
    /// Builds the overlay helper (first run only) and reads the display size.
    ///
    /// # Errors
    /// Fails when the helper can't be built or the display doesn't exist.
    pub fn new(display: usize, shots: &Path) -> Result<Self> {
        let overlay = screen::overlay_binary(&std::env::temp_dir().join("meet-agent"))?;
        let displays = screen::displays(&overlay)?;
        let info = *displays
            .get(display.saturating_sub(1))
            .with_context(|| format!("no display {display}"))?;
        Ok(Self {
            overlay,
            display,
            info,
            shots: shots.to_owned(),
        })
    }

    /// Where `target` is in `image`, on the 0-1000 grid.
    ///
    /// # Errors
    /// Fails when the brain can't find it.
    pub async fn locate(brain: &Brain, image: &Path, target: &str) -> Result<[f64; 4]> {
        let prompt = format!(
            "Find \"{target}\" in this screenshot. Answer with its bounding box on a 0-1000 grid, \
             exactly as: BOX: [x1, y1, x2, y2]"
        );
        let answer = brain
            .complete(&prompt, 60, Some(image.display().to_string()))
            .await?;
        screen::parse_box(&answer)
            .with_context(|| format!("no box in the brain's answer: {answer:?}"))
    }

    /// Screenshot, find `target`, draw the ring for `seconds`. Returns where.
    ///
    /// # Errors
    /// Fails when the screenshot or the lookup fails.
    pub async fn point_at(&self, brain: &Brain, target: &str, seconds: f64) -> Result<Rect> {
        std::fs::create_dir_all(&self.shots)?;
        let shot = self
            .shots
            .join(format!("screen-{}.png", std::process::id()));
        screen::screenshot(self.display, &shot)?;
        screen::shrink(&shot, 1280)?;
        let grid = Self::locate(brain, &shot, target).await?;
        let rect = screen::from_grid(grid, &self.info);
        if !screen::plausible(rect, &self.info) {
            bail!("{target:?} not found on screen (got an implausible box {grid:?})");
        }
        screen::spotlight(&self.overlay, self.display, rect, seconds)?;
        Ok(rect)
    }
}
