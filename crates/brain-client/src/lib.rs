//! Rust side of the connection to the Python brain server: start it, talk to it,
//! stop it.

pub mod protocol;

use std::{
    collections::HashMap,
    path::PathBuf,
    process::Stdio,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use futures::{SinkExt, StreamExt};
use protocol::{Command, Event};
use tokio::{
    process::{Child, Command as Process},
    sync::mpsc,
    task::JoinHandle,
};
use tokio_tungstenite::tungstenite::Message;

/// How to start the Python brain server.
#[derive(Debug, Clone)]
pub struct Settings {
    /// The `brain/` uv project.
    pub project_dir: PathBuf,
    pub model: String,
    pub port: u16,
    pub startup_timeout: Duration,
    /// How the agent introduces itself, e.g. "Sunny's meet agent".
    pub name: String,
    /// Whose meeting it is, e.g. "Sunny".
    pub owner: String,
}

type Waiters = Arc<Mutex<HashMap<u64, mpsc::UnboundedSender<Event>>>>;

/// The server process and the live connection to it; replaced on restart.
struct Link {
    child: Child,
    outgoing: mpsc::UnboundedSender<Command>,
    tasks: [JoinHandle<()>; 2],
}

/// A running brain server and the connection to it.
pub struct Brain {
    link: Mutex<Option<Link>>,
    settings: Settings,
    waiters: Waiters,
    next_id: AtomicU64,
    pub model: String,
}

impl Brain {
    /// Starts the server, connects, and waits until the model is loaded and warm.
    ///
    /// # Errors
    /// Fails when the server exits early or isn't ready within the timeout.
    pub async fn start(s: &Settings) -> Result<Self> {
        let waiters: Waiters = Arc::default();
        let (link, model) = connect(s, Arc::clone(&waiters)).await?;
        Ok(Self {
            link: Mutex::new(Some(link)),
            settings: s.clone(),
            waiters,
            next_id: AtomicU64::new(1),
            model,
        })
    }

    /// Stops the server and starts a fresh one (after a crash). Requests that were
    /// waiting get an error.
    ///
    /// # Errors
    /// Fails when the new server doesn't come up.
    pub async fn restart(&self) -> Result<()> {
        self.shutdown().await;
        for (id, tx) in self
            .waiters
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .drain()
        {
            let _ = tx.send(Event::Error {
                id: Some(id),
                message: "brain server restarted".into(),
            });
        }
        let (link, _) = connect(&self.settings, Arc::clone(&self.waiters)).await?;
        *self
            .link
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(link);
        Ok(())
    }

    fn register(&self) -> (u64, mpsc::UnboundedReceiver<Event>) {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = mpsc::unbounded_channel();
        self.waiters
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(id, tx);
        (id, rx)
    }

    fn send(&self, cmd: Command) -> Result<()> {
        let link = self
            .link
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let link = link
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("brain server is stopped"))?;
        link.outgoing
            .send(cmd)
            .map_err(|_| anyhow::anyhow!("brain server connection closed"))
    }

    /// Asks for the next action. Returns the request id and a stream of
    /// `Sentence` events ending with `Reply`, `Cancelled` or `Error`.
    ///
    /// # Errors
    /// Fails when the connection is closed.
    pub fn think(
        &self,
        text: &str,
        state: &str,
        allowed: &[&str],
        context: &str,
        image: Option<String>,
    ) -> Result<(u64, mpsc::UnboundedReceiver<Event>)> {
        let (id, rx) = self.register();
        self.send(Command::Think {
            id,
            text: text.to_owned(),
            state: state.to_owned(),
            allowed: allowed.iter().map(|a| (*a).to_owned()).collect(),
            context: context.to_owned(),
            image,
        })?;
        Ok((id, rx))
    }

    /// Plain completion (briefs, summaries, looking at a screenshot).
    ///
    /// # Errors
    /// Fails when the brain reports an error or the connection drops.
    pub async fn complete(
        &self,
        prompt: &str,
        max_tokens: u32,
        image: Option<String>,
    ) -> Result<String> {
        let (id, mut rx) = self.register();
        self.send(Command::Complete {
            id,
            prompt: prompt.to_owned(),
            max_tokens,
            image,
        })?;
        while let Some(ev) = rx.recv().await {
            match ev {
                Event::Text { text, .. } => return Ok(text),
                Event::Error { message, .. } => bail!("brain: {message}"),
                Event::Cancelled { .. } => bail!("brain request cancelled"),
                _ => {}
            }
        }
        bail!("brain server disconnected")
    }

    /// Stops a running request.
    pub fn cancel(&self, id: u64) {
        let _ = self.send(Command::Cancel { id });
    }

    /// Runs one realistic request with `context` so the first real question is
    /// as fast as the rest, then forgets it.
    ///
    /// # Errors
    /// Fails when the brain reports an error.
    pub async fn warm_up(&self, context: &str) -> Result<()> {
        let (_, mut rx) = self.think(
            "Hello, can you hear me?",
            "LISTENING",
            &["answer"],
            context,
            None,
        )?;
        while let Some(ev) = rx.recv().await {
            match ev {
                Event::Reply { .. } | Event::Cancelled { .. } => break,
                Event::Error { message, .. } => bail!("brain warm-up: {message}"),
                _ => {}
            }
        }
        self.reset();
        Ok(())
    }

    /// Forgets the conversation.
    pub fn reset(&self) {
        let _ = self.send(Command::Reset);
    }

    /// True while the server process is running.
    pub fn is_alive(&self) -> bool {
        let mut link = self
            .link
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        link.as_mut()
            .is_some_and(|l| matches!(l.child.try_wait(), Ok(None)))
    }

    /// Stops the server; its memory is freed. Safe to call from a shared handle.
    pub async fn shutdown(&self) {
        let link = self
            .link
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        let Some(mut link) = link else { return };
        for t in &link.tasks {
            t.abort();
        }
        if let Some(pid) = link.child.id() {
            let _ = tokio::process::Command::new("kill")
                .args(["-TERM", &format!("-{pid}")])
                .status()
                .await;
        }
        if tokio::time::timeout(Duration::from_secs(3), link.child.wait())
            .await
            .is_err()
        {
            let _ = link.child.kill().await;
        }
    }
}

/// Starts the server process, connects, and waits for `ready`.
async fn connect(s: &Settings, waiters: Waiters) -> Result<(Link, String)> {
    let mut child = spawn_server(s)?;
    let url = format!("ws://127.0.0.1:{}", s.port);
    let start = Instant::now();
    let ws = loop {
        if let Some(status) = child.try_wait()? {
            bail!("the brain server exited during startup ({status})");
        }
        if let Ok((ws, _)) = tokio_tungstenite::connect_async(&url).await {
            break ws;
        }
        if start.elapsed() > s.startup_timeout {
            bail!(
                "the brain server wasn't reachable within {:?}",
                s.startup_timeout
            );
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    };
    let (mut sink, mut stream) = ws.split();
    let model = match tokio::time::timeout(s.startup_timeout, stream.next()).await {
        Ok(Some(Ok(Message::Text(t)))) => match serde_json::from_str(&t)? {
            Event::Ready { model } => model,
            other => bail!("expected ready from the brain server, got {other:?}"),
        },
        _ => bail!("the brain server never said it was ready"),
    };
    let (outgoing, mut rx) = mpsc::unbounded_channel::<Command>();
    let writer = tokio::spawn(async move {
        while let Some(cmd) = rx.recv().await {
            let Ok(text) = serde_json::to_string(&cmd) else {
                continue;
            };
            if sink.send(Message::text(text)).await.is_err() {
                break;
            }
        }
    });
    let reader = tokio::spawn(route_events(stream, waiters));
    Ok((
        Link {
            child,
            outgoing,
            tasks: [writer, reader],
        },
        model,
    ))
}

fn spawn_server(s: &Settings) -> Result<Child> {
    Process::new("uv")
        .args(["run", "-q", "--project"])
        .arg(&s.project_dir)
        .args([
            "brain-server",
            "--port",
            &s.port.to_string(),
            "--model",
            &s.model,
        ])
        .args(["--name", &s.name, "--owner", &s.owner])
        .args(["--parent-pid", &std::process::id().to_string()])
        // Models are already downloaded; never reach out to Hugging Face.
        .env("HF_HUB_OFFLINE", "1")
        .env("TRANSFORMERS_OFFLINE", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        // Own process group, so shutdown stops uv and the Python process under it.
        .process_group(0)
        .spawn()
        .context("starting the Python brain server (is uv installed?)")
}

/// Sends every event to the request that's waiting for it.
async fn route_events<S>(mut stream: S, routes: Waiters)
where
    S: futures::Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    while let Some(Ok(msg)) = stream.next().await {
        let Message::Text(text) = msg else { continue };
        let Ok(event) = serde_json::from_str::<Event>(&text) else {
            continue;
        };
        let done = matches!(
            event,
            Event::Reply { .. }
                | Event::Text { .. }
                | Event::Cancelled { .. }
                | Event::Error { .. }
        );
        let mut map = routes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(id) = event.id() {
            if let Some(tx) = map.get(&id) {
                let _ = tx.send(event);
            }
            if done {
                map.remove(&id);
            }
        }
    }
    // Connection lost: wake everyone who is waiting.
    let mut map = routes
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    for (id, tx) in map.drain() {
        let _ = tx.send(Event::Error {
            id: Some(id),
            message: "brain server disconnected".into(),
        });
    }
}
