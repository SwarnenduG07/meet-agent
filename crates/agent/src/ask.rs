//! `meet-agent ask "<question>"`: one question to the brain, with timings.

use std::{path::Path, time::Instant};

use anyhow::{Context, Result};
use brain_client::{Brain, protocol::Event};

use crate::config::Config;

pub async fn run(cfg: &Config, question: &str, brief: &Path) -> Result<()> {
    let context =
        std::fs::read_to_string(brief).with_context(|| format!("reading {}", brief.display()))?;
    let t = Instant::now();
    let brain = Brain::start(&cfg.brain()).await?;
    println!(
        "brain ready ({}) in {:.1}s",
        brain.model,
        t.elapsed().as_secs_f32()
    );

    let allowed = ["answer", "share_screen", "defer_to_sunny", "wait", "leave"];
    let t = Instant::now();
    let (_, mut events) = brain.think(question, "LISTENING", &allowed, &context, None)?;
    while let Some(ev) = events.recv().await {
        match ev {
            Event::Sentence { text, .. } => {
                println!("  +{:.2}s  {text}", t.elapsed().as_secs_f32());
            }
            Event::Reply { action, .. } => {
                println!("  +{:.2}s  action: {action}", t.elapsed().as_secs_f32());
                break;
            }
            other => {
                println!("  {other:?}");
                break;
            }
        }
    }
    brain.shutdown().await;
    Ok(())
}
