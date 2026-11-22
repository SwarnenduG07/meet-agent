//! Starts the user's real Chrome with the agent's own profile and a local control
//! port. Nothing automation-related is attached at launch, so Google sees a normal
//! browser and allows signing in. Later steps attach over CDP on that port.

use std::{
    io,
    net::{Ipv4Addr, SocketAddr, TcpStream},
    path::PathBuf,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

pub struct Chrome {
    pub binary: PathBuf,
    /// Holds the agent's Google sign-in. Never the user's personal profile.
    pub profile_dir: PathBuf,
    /// Chrome binds the `DevTools` port to 127.0.0.1 only.
    pub debug_port: u16,
    /// Title of the source Chrome picks when a page asks to share the screen, so
    /// its picker never needs a click: "Screen 1", "Screen 2" with several
    /// displays, "Entire screen" with one.
    pub share_source: String,
}

impl Chrome {
    /// Starts Chrome on `url` with the agent profile and the control port.
    ///
    /// # Errors
    /// Fails when the profile folder can't be created or Chrome can't start.
    pub fn launch(&self, url: &str) -> io::Result<Child> {
        std::fs::create_dir_all(&self.profile_dir)?;
        Command::new(&self.binary)
            .arg(format!("--user-data-dir={}", self.profile_dir.display()))
            .arg(format!("--remote-debugging-port={}", self.debug_port))
            .arg(format!(
                "--auto-select-desktop-capture-source={}",
                self.share_source
            ))
            // Recent Chrome on macOS hands screen picking to the macOS system picker,
            // which the flag above can't answer. Keep Chrome's own picker.
            .arg("--disable-features=UseSCContentSharingPicker")
            .args(["--no-first-run", "--no-default-browser-check"])
            .arg(url)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
    }

    /// Checks the control port at least once, then keeps trying until `timeout`.
    #[must_use]
    pub fn wait_for_port(&self, timeout: Duration) -> bool {
        let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, self.debug_port));
        let start = Instant::now();
        loop {
            if TcpStream::connect_timeout(&addr, Duration::from_millis(200)).is_ok() {
                return true;
            }
            if start.elapsed() >= timeout {
                return false;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    }
}
