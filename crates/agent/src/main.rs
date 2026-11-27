//! `meet-agent`: joins Google Meet for you, presents from a brief and answers questions.

mod ask;
mod config;
mod doctor;
mod join;
mod login;

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
    Join {
        /// Meet link, e.g. <https://meet.google.com/abc-defg-hij>.
        link: String,
        /// Open the green room and list Meet's buttons, without joining.
        #[arg(long)]
        dry_run: bool,
        /// Share the entire screen once in the call, then stop before leaving.
        #[arg(long)]
        share: bool,
        /// App, file or URL to bring forward while sharing.
        #[arg(long, requires = "share")]
        show: Option<String>,
        /// Seconds to stay in the call (or to keep sharing) before leaving.
        #[arg(long, default_value_t = 20)]
        stay: u64,
    },
    /// Ask the brain one question using a brief (developer test, no audio).
    Ask {
        question: String,
        #[arg(long, default_value = "briefs/example.md")]
        brief: PathBuf,
    },
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
        Cmd::Login => login::run(&cfg),
        Cmd::Join {
            link,
            dry_run,
            share,
            show,
            stay,
        } => {
            let opts = join::Options {
                dry_run,
                share,
                show,
                stay: std::time::Duration::from_secs(stay),
            };
            tokio::runtime::Runtime::new()?.block_on(join::run(&cfg, &link, opts))
        }
        Cmd::Ask { question, brief } => {
            tokio::runtime::Runtime::new()?.block_on(ask::run(&cfg, &question, &brief))
        }
        Cmd::Listen => bail!("not built yet: Phase 2b (audio + speech crates)"),
        Cmd::Run { .. } => bail!("not built yet: Phase 3 (orchestrator)"),
    }
}
