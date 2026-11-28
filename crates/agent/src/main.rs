//! `meet-agent`: joins Google Meet for you, presents from a brief and answers questions.

mod ask;
mod config;
mod connectors;
mod conversation;
mod demo;
mod doctor;
mod join;
mod listen;
mod login;
mod orchestrator;
mod plan;
mod run;
mod screen_tools;
mod timeline;
mod voice;

use std::path::PathBuf;

use anyhow::Result;
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
    },
    /// Find something on screen and ring it (or, with --image, just locate it there).
    Point {
        target: String,
        /// Look in this image instead of taking a screenshot.
        #[arg(long)]
        image: Option<PathBuf>,
    },
    /// Draft a brief from a Codex or Claude Code planning thread (local files).
    Brief {
        /// Codex thread: `latest`, a session id, or a path.
        #[arg(long, conflicts_with = "from_claude")]
        from_codex: Option<String>,
        /// Claude Code session: `latest`, a session id, or a path.
        #[arg(long)]
        from_claude: Option<String>,
        /// Where to write the draft (default: briefs/drafts/<title>.md).
        #[arg(long)]
        out: Option<PathBuf>,
        /// List recent threads instead.
        #[arg(long)]
        list: bool,
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
    /// Run a full meeting from a brief: prepare, join, present, answer, leave, summary.
    Run {
        brief: PathBuf,
        /// Meet link (overrides the brief's).
        #[arg(long)]
        link: Option<String>,
        /// Join now instead of waiting for the brief's start time.
        #[arg(long)]
        now: bool,
        /// Run everything locally (mic and speakers) without joining Meet.
        #[arg(long)]
        rehearse: bool,
        /// Rehearsal: listen to a WAV recording instead of the mic.
        #[arg(long, requires = "rehearse")]
        file: Option<PathBuf>,
        /// Rehearsal: seconds before the recording starts.
        #[arg(long, default_value_t = 0)]
        file_delay: u32,
        /// Rehearsal: play no sound.
        #[arg(long)]
        mute: bool,
    },
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
        Cmd::Talk { link, brief } => {
            let opts = run::Options {
                brief,
                link: Some(link),
                share: Some(plan::SharePolicy::OnRequest),
                now: true,
                rehearse: false,
                file: None,
                file_delay: 0,
                mute: false,
            };
            tokio::runtime::Runtime::new()?.block_on(run::run(&cfg, opts))
        }
        Cmd::Point { target, image } => {
            tokio::runtime::Runtime::new()?.block_on(point(&cfg, &target, image))
        }
        Cmd::Brief {
            from_codex,
            from_claude,
            out,
            list,
        } => tokio::runtime::Runtime::new()?.block_on(brief(
            &cfg,
            from_codex,
            from_claude,
            out,
            list,
        )),
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
        Cmd::Run {
            brief,
            link,
            now,
            rehearse,
            file,
            file_delay,
            mute,
        } => {
            let opts = run::Options {
                brief,
                link,
                share: None,
                now,
                rehearse,
                file,
                file_delay,
                mute,
            };
            tokio::runtime::Runtime::new()?.block_on(run::run(&cfg, opts))
        }
    }
}

async fn point(cfg: &config::Config, target: &str, image: Option<PathBuf>) -> Result<()> {
    let brain = brain_client::Brain::start(&cfg.brain()).await?;
    let t = std::time::Instant::now();
    let result = if let Some(image) = image {
        screen_tools::Pointer::locate(&brain, &image, target)
            .await
            .map(|b| {
                println!(
                    "{target:?} is at {b:?} on the 0-1000 grid ({:.1}s)",
                    t.elapsed().as_secs_f32()
                );
            })
    } else {
        let shots = std::env::temp_dir().join("meet-agent");
        let pointer = screen_tools::Pointer::new(cfg.screen.display, &shots)?;
        pointer
            .point_at(&brain, target, cfg.screen.spotlight_secs)
            .await
            .map(|r| {
                println!(
                    "ringed {target:?} at {r:?} ({:.1}s)",
                    t.elapsed().as_secs_f32()
                );
            })
    };
    brain.shutdown().await;
    result
}

async fn brief(
    cfg: &config::Config,
    from_codex: Option<String>,
    from_claude: Option<String>,
    out: Option<PathBuf>,
    list: bool,
) -> Result<()> {
    use connectors::Source;
    let (source, which) = match (from_codex, from_claude) {
        (Some(w), _) => (Source::Codex, w),
        (None, Some(w)) => (Source::Claude, w),
        (None, None) => (Source::Codex, "latest".to_owned()),
    };
    if list {
        for path in connectors::files(source)?.into_iter().take(15) {
            let t = connectors::read(source, &path)?;
            println!(
                "{:>4} msgs  {}  {}",
                t.messages.len(),
                t.title,
                path.display()
            );
        }
        return Ok(());
    }
    let path = connectors::find(source, &which)?;
    let thread = connectors::read(source, &path)?;
    println!(
        "Thread: {} ({} messages)",
        thread.title,
        thread.messages.len()
    );
    let slug: String = thread
        .title
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .take(6)
        .collect::<Vec<_>>()
        .join("-");
    let out = out.unwrap_or_else(|| PathBuf::from(format!("briefs/drafts/{slug}.md")));
    let brain = brain_client::Brain::start(&cfg.brain()).await?;
    let result = connectors::draft_brief(&brain, &thread, &cfg.brain.owner, &out).await;
    brain.shutdown().await;
    result?;
    println!("Draft brief: {}", out.display());
    println!(
        "Review it before use: check every fact, remove anything private, add `link:` to the header."
    );
    Ok(())
}
