//! The talk loop shared by every mode: hear a turn, transcribe it, ask the brain,
//! speak the streamed reply, and let a [`Director`] decide what it may do.

use std::time::{Duration, Instant};

use anyhow::Result;
use audio::listener::Heard;
use brain_client::{Brain, protocol::Event};
use tokio::{io::AsyncBufReadExt, sync::mpsc};

use crate::voice::{Ears, Voice};

/// Whether the conversation goes on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    Continue,
    Stop,
}

/// Keyboard commands typed into the terminal while the agent runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Control {
    /// `t`: you take over the call (your mic), or hand it back to the agent.
    TakeOver,
    /// `s`: share the screen, or stop sharing.
    Share,
    /// `n`: go to the next part of the plan.
    Next,
    /// `l`: say goodbye and leave.
    Leave,
}

/// What a director can use while acting.
pub struct Tools<'a> {
    pub voice: &'a Voice,
}

/// Decides what the agent may do; the conversation loop does the talking.
pub trait Director {
    /// Current state name, sent to the brain (e.g. `LISTENING`).
    fn state(&self) -> &'static str;
    /// Actions the brain may choose right now.
    fn allowed(&self) -> Vec<&'static str>;
    /// Brief, current section and anything else the brain should know.
    fn context(&self) -> String;
    /// False while the user has taken over; turns are then ignored.
    fn listening(&self) -> bool {
        true
    }
    /// Runs the brain's chosen action once its words are queued.
    async fn act(
        &mut self,
        _question: &str,
        _action: &str,
        _say: &str,
        _tools: &Tools<'_>,
    ) -> Result<Flow> {
        Ok(Flow::Continue)
    }
    /// Called about once a second (e.g. to notice the meeting ended).
    async fn tick(&mut self, _tools: &Tools<'_>) -> Result<Flow> {
        Ok(Flow::Continue)
    }
    /// A keyboard command.
    async fn control(&mut self, _control: Control, _tools: &Tools<'_>) -> Result<Flow> {
        Ok(Flow::Continue)
    }
    /// Ctrl+C or end of input: wrap up (e.g. leave the call politely).
    async fn finish(&mut self, _tools: &Tools<'_>) -> Result<()> {
        Ok(())
    }
}

/// Whisper's usual output for noise or silence.
const NOISE: [&str; 5] = ["Thank you.", "Thanks for watching!", "you", "Bye.", "."];

struct Turn {
    asked: Instant,
    id: u64,
    events: mpsc::UnboundedReceiver<Event>,
    ended: Instant,
    question: String,
    spoke: bool,
    said: String,
}

/// Prints a log line with seconds since the agent started.
pub fn log(stage: &str, msg: &str) {
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    let t = START.get_or_init(Instant::now).elapsed().as_secs_f32();
    println!("[{t:>7.2}s] [{stage:<7}] {msg}");
}

/// Reads single-letter commands from the terminal.
pub fn keyboard() -> mpsc::UnboundedReceiver<Control> {
    let (tx, rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        let mut lines = tokio::io::BufReader::new(tokio::io::stdin()).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let control = match line.trim() {
                "t" => Control::TakeOver,
                "s" => Control::Share,
                "n" => Control::Next,
                "l" => Control::Leave,
                _ => continue,
            };
            if tx.send(control).is_err() {
                break;
            }
        }
    });
    rx
}

pub struct Options {
    /// Stop talking when someone else starts (only safe when the agent can't
    /// hear itself: headphones or `BlackHole`).
    pub barge_in: bool,
    /// Stop once the input ends (recordings).
    pub stop_at_end_of_input: bool,
}

/// Runs until the director says stop, Ctrl+C, or (optionally) the input ends.
pub async fn run<D: Director>(
    brain: &Brain,
    voice: &Voice,
    ears: &mut Ears,
    director: &mut D,
    mut controls: mpsc::UnboundedReceiver<Control>,
    opts: Options,
) -> Result<()> {
    let tools = Tools { voice };
    let mut current: Option<Turn> = None;
    let mut pending: Option<(String, Instant)> = None;
    let mut input_done = false;
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    let ctrl_c = tokio::signal::ctrl_c();
    tokio::pin!(ctrl_c);

    loop {
        let flow = tokio::select! {
            _ = &mut ctrl_c => {
                log("agent", "Ctrl+C: wrapping up");
                Flow::Stop
            }
            heard = ears.heard.recv(), if !input_done => match heard {
                None => {
                    input_done = true;
                    Flow::Continue
                }
                Some(h) if !director.listening() => {
                    drop(h);
                    Flow::Continue
                }
                Some(Heard::SpeechStarted) => {
                    if opts.barge_in && (voice.is_speaking() || current.is_some()) {
                        voice.stop();
                        if let Some(c) = current.take() {
                            brain.cancel(c.id);
                        }
                        log("barge", "someone started talking; stopped");
                    }
                    Flow::Continue
                }
                Some(Heard::Turn { audio, ended, .. }) => {
                    let t = Instant::now();
                    let text = voice.transcribe(audio).await?;
                    log("heard", &format!("({:.2}s) {text}", t.elapsed().as_secs_f32()));
                    if !text.is_empty() && !NOISE.contains(&text.as_str()) {
                        let question = match pending.take() {
                            Some((before, at)) if at.elapsed().as_secs() < 10 => format!("{before} {text}"),
                            _ => text,
                        };
                        let allowed = director.allowed();
                        let (id, events) = brain.think(&question, director.state(), &allowed, &director.context(), None)?;
                        current = Some(Turn { asked: Instant::now(), id, events, ended, question, spoke: false, said: String::new() });
                    }
                    Flow::Continue
                }
            },
            event = next_event(&mut current) => on_brain_event(&tools, director, &mut current, &mut pending, event).await?,
            _ = tick.tick() => director.tick(&tools).await?,
            Some(control) = controls.recv() => director.control(control, &tools).await?,
        };
        if flow == Flow::Stop || (opts.stop_at_end_of_input && input_done && current.is_none()) {
            break;
        }
    }
    if let Some(c) = current.take() {
        brain.cancel(c.id);
    }
    director.finish(&tools).await?;
    voice.finish().await;
    Ok(())
}

/// Speaks streamed sentences as they arrive; on the reply, lets the director act.
async fn on_brain_event<D: Director>(
    tools: &Tools<'_>,
    director: &mut D,
    current: &mut Option<Turn>,
    pending: &mut Option<(String, Instant)>,
    event: Option<Event>,
) -> Result<Flow> {
    let Some(turn) = current.as_mut() else {
        return Ok(Flow::Continue);
    };
    match event {
        Some(Event::Sentence { text, .. }) => {
            let since_end = turn.ended.elapsed();
            let to_audio = tools.voice.say(&text).await?;
            if !turn.spoke {
                turn.spoke = true;
                log(
                    "timing",
                    &format!(
                        "brain {:.2}s, first audio {:.2}s after the question ended",
                        turn.asked.elapsed().as_secs_f32() - to_audio.as_secs_f32(),
                        (since_end + to_audio).as_secs_f32()
                    ),
                );
            }
            log("say", &text);
            turn.said.push_str(&text);
            turn.said.push(' ');
            Ok(Flow::Continue)
        }
        Some(Event::Reply { action, .. }) => {
            let Some(turn) = current.take() else {
                return Ok(Flow::Continue);
            };
            log("action", &action);
            if action == "wait" {
                *pending = Some((turn.question, Instant::now()));
                return Ok(Flow::Continue);
            }
            director
                .act(&turn.question, &action, turn.said.trim(), tools)
                .await
        }
        other => {
            log("brain", &format!("{other:?}"));
            *current = None;
            Ok(Flow::Continue)
        }
    }
}

async fn next_event(current: &mut Option<Turn>) -> Option<Event> {
    match current {
        Some(turn) => turn.events.recv().await,
        None => std::future::pending().await,
    }
}
