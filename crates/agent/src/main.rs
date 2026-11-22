//! `meet-agent`: joins Google Meet for you, presents from a brief and answers questions.

mod config;
mod doctor;

use std::path::PathBuf;

use anyhow::{Result, bail};
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "meet-agent", version, about)]
struct Cli {
    /// Path to the agent config.
    #[arg(long, global = true, default_value = "config/agent.toml")]
    config: PathBuf,
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Check tools, audio devices, models and the Chrome profile.
    Doctor,
    /// Open the agent's Chrome profile so you can sign in to Google once.
    Login,
    /// Join a meeting, share the screen, then leave. No AI (Phase 1 test).
    Join { link: String },
    /// Voice loop with your mic and speakers, no Meet (Phase 2b test).
    Listen,
    /// Run a full meeting from a brief.
    Run { brief: PathBuf },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let cfg = config::Config::load(&cli.config)?;
    match cli.command {
        Cmd::Doctor => {
            if doctor::run(&cfg) > 0 {
                std::process::exit(1);
            }
            Ok(())
        }
        Cmd::Login | Cmd::Join { .. } => bail!("not built yet: Phase 1 (meet crate)"),
        Cmd::Listen => bail!("not built yet: Phase 2b (audio + speech crates)"),
        Cmd::Run { .. } => bail!("not built yet: Phase 3 (orchestrator)"),
    }
}
