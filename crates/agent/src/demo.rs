//! Runs a section's demo steps (`do:` lines) one after another.

use std::{sync::Arc, time::Duration};

use brain_client::Brain;

use crate::{conversation::log, plan::Step, screen_tools::Pointer};

/// Runs `steps` in order. A failed step is logged and the rest still run, so
/// one missing button never stops the presentation.
pub async fn run(steps: Vec<Step>, brain: Arc<Brain>, pointer: Option<Arc<Pointer>>) {
    for step in steps {
        let result = match &step {
            Step::Open(target) => desktop::windows::show(target),
            Step::Keys(combo) => desktop::windows::keys(combo),
            Step::Type(text) => desktop::windows::type_text(text),
            Step::Wait(secs) => {
                tokio::time::sleep(Duration::from_secs_f64(secs.max(0.0))).await;
                Ok(())
            }
            Step::Click(label) => match &pointer {
                Some(p) => p.click_on(&brain, label).await.map(|_| ()),
                None => Err(anyhow::anyhow!("screen tools are unavailable")),
            },
        };
        match result {
            Ok(()) => log("demo", &format!("{step:?}")),
            Err(e) => log("demo", &format!("{step:?} failed: {e}")),
        }
    }
}
