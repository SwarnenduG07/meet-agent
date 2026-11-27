//! `meet-agent`: joins Google Meet for you, presents from a brief and answers questions.

mod ask;
mod config;
mod conversation;
mod doctor;
mod join;
mod listen;
mod login;
mod meeting;
mod voice;

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
    /// Join a meeting and talk: answers questions, shares when asked, leaves on request.
    Talk {
        link: String,
        #[arg(long, default_value = "briefs/example.md")]
        brief: PathBuf,
        /// App, file or URL to bring forward when sharing starts.
        #[arg(long)]
        show: Option<String>,
    },
    /// Ask the brain one question using a brief (developer test, no audio).
    Ask {
        question: String,
        #[arg(long, default_value = "briefs/example.md")]
        brief: PathBuf,
    },
    /// Talk to the agent with your mic and speakers, no Meet.
    Listen {
        #[arg(long, default_value = "briefs/example.md")]
        brief: PathBuf,
        /// Stop talking when you talk (wear headphones).
        #[arg(long)]
        barge_in: bool,
        /// Listen to a WAV recording instead of the mic.
        #[arg(long)]
        file: Option<PathBuf>,
        /// Don't play any sound.
        #[arg(long)]
        mute: bool,
    },
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
        Cmd::Talk { link, brief, show } => {
            let opts = meeting::Options { link, brief, show };
            tokio::runtime::Runtime::new()?.block_on(meeting::run(&cfg, opts))
        }
        Cmd::Ask { question, brief } => {
            tokio::runtime::Runtime::new()?.block_on(ask::run(&cfg, &question, &brief))
        }
        Cmd::Listen {
            brief,
            barge_in,
            file,
            mute,
        } => {
            let opts = listen::Options {
                brief,
                barge_in,
                file,
                mute,
            };
            tokio::runtime::Runtime::new()?.block_on(listen::run(&cfg, opts))
        }
        Cmd::Run { .. } => bail!("not built yet: Phase 3 (orchestrator)"),
    }
}
