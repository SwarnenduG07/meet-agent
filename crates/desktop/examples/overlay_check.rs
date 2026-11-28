//! Builds the overlay, lists displays, and flashes a ring in the middle of display 1.
fn main() -> anyhow::Result<()> {
    let dir = std::env::temp_dir().join("meet-agent");
    let bin = desktop::screen::overlay_binary(&dir)?;
    let displays = desktop::screen::displays(&bin)?;
    println!("overlay: {}\ndisplays: {displays:?}", bin.display());
    let d = displays[0];
    let rect = desktop::screen::from_grid([400.0, 400.0, 600.0, 600.0], &d);
    println!("ring at {rect:?}");
    desktop::screen::spotlight(&bin, 1, rect, 1.5)?;
    std::thread::sleep(std::time::Duration::from_secs(2));
    Ok(())
}
