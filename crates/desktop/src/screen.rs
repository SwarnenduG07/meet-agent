//! Displays, screenshots and the spotlight ring.

use std::{
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

/// A display in points, top-left origin (the primary display starts at 0,0).
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct Display {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub scale: f64,
}

/// A rectangle on a display, in points from the display's top-left corner.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

const OVERLAY_SOURCE: &str = include_str!("../swift/overlay.swift");

/// Builds the Swift overlay helper once (into `dir`) and returns its path.
///
/// # Errors
/// Fails when `swiftc` (Xcode command line tools) isn't installed.
pub fn overlay_binary(dir: &Path) -> Result<PathBuf> {
    let bin = dir.join("meet-agent-overlay");
    let src = dir.join("overlay.swift");
    let fresh = std::fs::read_to_string(&src).is_ok_and(|s| s == OVERLAY_SOURCE) && bin.is_file();
    if !fresh {
        std::fs::create_dir_all(dir)?;
        std::fs::write(&src, OVERLAY_SOURCE)?;
        let out = Command::new("swiftc")
            .args(["-O", "-o"])
            .arg(&bin)
            .arg(&src)
            .output()
            .context("running swiftc (install Xcode command line tools)")?;
        if !out.status.success() {
            bail!(
                "building the overlay failed: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
    }
    Ok(bin)
}

/// All displays, primary first.
///
/// # Errors
/// Fails when the helper can't run.
pub fn displays(overlay: &Path) -> Result<Vec<Display>> {
    let out = Command::new(overlay)
        .arg("screens")
        .output()
        .context("listing displays")?;
    Ok(serde_json::from_slice(&out.stdout)?)
}

/// Saves a screenshot of display `n` (1-based, like `screencapture -D`).
///
/// # Errors
/// Fails when macOS refuses (the terminal needs Screen Recording permission).
pub fn screenshot(display: usize, path: &Path) -> Result<()> {
    let out = Command::new("screencapture")
        .args(["-x", "-t", "png", "-D", &display.to_string()])
        .arg(path)
        .output()
        .context("running screencapture")?;
    if !out.status.success() || !path.is_file() {
        bail!(
            "screenshot failed ({}); allow your terminal in System Settings > Privacy & Security > Screen Recording",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

/// Shrinks an image in place so its longest side is at most `max_px`. Vision
/// models work much faster on smaller images, and text stays readable at 1280.
///
/// # Errors
/// Fails when `sips` (built into macOS) fails.
pub fn shrink(path: &Path, max_px: u32) -> Result<()> {
    let out = Command::new("sips")
        .args(["-Z", &max_px.to_string()])
        .arg(path)
        .output()
        .context("running sips")?;
    if !out.status.success() {
        bail!(
            "resizing the screenshot failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

/// Draws a ring around `rect` on display `n` (1-based) for `seconds`. Returns at once.
///
/// # Errors
/// Fails when the helper can't start.
pub fn spotlight(overlay: &Path, display: usize, rect: Rect, seconds: f64) -> Result<()> {
    Command::new(overlay)
        .arg("ring")
        .args([rect.x, rect.y, rect.w, rect.h, seconds].map(|v| format!("{v:.1}")))
        .arg(display.to_string())
        .spawn()
        .context("starting the spotlight")?;
    Ok(())
}

/// Turns a box on Qwen-VL's 0-1000 grid into points on a display.
#[must_use]
pub fn from_grid(grid: [f64; 4], display: &Display) -> Rect {
    let [x1, y1, x2, y2] = grid.map(|v| v.clamp(0.0, 1000.0) / 1000.0);
    Rect {
        x: x1 * display.w,
        y: y1 * display.h,
        w: ((x2 - x1) * display.w).abs(),
        h: ((y2 - y1) * display.h).abs(),
    }
}

/// True when a found box is plausible: big enough to see and not most of the screen.
#[must_use]
pub fn plausible(rect: Rect, display: &Display) -> bool {
    let area = rect.w * rect.h;
    rect.w >= 8.0 && rect.h >= 8.0 && area < 0.6 * display.w * display.h
}

/// Finds a `[x1, y1, x2, y2]` box in the model's answer.
#[must_use]
pub fn parse_box(answer: &str) -> Option<[f64; 4]> {
    let start = answer.find('[')?;
    let end = start + answer[start..].find(']')?;
    let nums: Vec<f64> = answer[start + 1..end]
        .split(',')
        .filter_map(|n| n.trim().parse().ok())
        .collect();
    <[f64; 4]>::try_from(nums.get(..4)?).ok()
}
