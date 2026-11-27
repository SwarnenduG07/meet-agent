//! Messages between the Rust orchestrator and the Python brain server.
//! Must match `protocol/messages.json`, which the Python side follows.

use serde::{Deserialize, Serialize};

/// Rust → Python.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Command {
    /// Ask for the next action and what to say. Sentences stream back first.
    Think {
        id: u64,
        text: String,
        state: String,
        allowed: Vec<String>,
        /// Brief, current plan section and running summary.
        #[serde(default)]
        context: String,
        /// Path to a screenshot when the question is about the screen.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        image: Option<String>,
    },
    /// Plain completion without conversation memory: briefs, summaries, looking.
    Complete {
        id: u64,
        prompt: String,
        max_tokens: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        image: Option<String>,
    },
    /// Stop working on a request, for example because someone interrupted.
    Cancel { id: u64 },
    /// Forget the conversation (new meeting).
    Reset,
}

/// Python → Rust.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    /// The model is loaded and warm; the server accepts commands.
    Ready { model: String },
    /// One finished sentence of a `think` reply, sent early so speech can start.
    Sentence { id: u64, text: String },
    /// The complete `think` reply.
    Reply {
        id: u64,
        action: String,
        say: String,
    },
    /// The result of a `complete`.
    Text { id: u64, text: String },
    /// The request was cancelled.
    Cancelled { id: u64 },
    /// The request failed; the orchestrator falls back to a safe line.
    Error { id: Option<u64>, message: String },
}

impl Event {
    /// The request this event belongs to, if any.
    #[must_use]
    pub fn id(&self) -> Option<u64> {
        match self {
            Self::Ready { .. } => None,
            Self::Sentence { id, .. }
            | Self::Reply { id, .. }
            | Self::Text { id, .. }
            | Self::Cancelled { id } => Some(*id),
            Self::Error { id, .. } => *id,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_format_matches_the_contract() {
        let cmd = Command::Think {
            id: 7,
            text: "Can you share your screen?".into(),
            state: "LISTENING".into(),
            allowed: vec!["answer".into(), "share_screen".into()],
            context: String::new(),
            image: None,
        };
        assert_eq!(
            serde_json::to_string(&cmd).unwrap(),
            r#"{"cmd":"think","id":7,"text":"Can you share your screen?","state":"LISTENING","allowed":["answer","share_screen"],"context":""}"#
        );
        assert_eq!(
            serde_json::to_string(&Command::Reset).unwrap(),
            r#"{"cmd":"reset"}"#
        );
        let ev: Event = serde_json::from_str(
            r#"{"event":"reply","id":7,"action":"share_screen","say":"Sure."}"#,
        )
        .unwrap();
        assert_eq!(
            ev,
            Event::Reply {
                id: 7,
                action: "share_screen".into(),
                say: "Sure.".into()
            }
        );
        let ev: Event = serde_json::from_str(r#"{"event":"cancelled","id":3}"#).unwrap();
        assert_eq!(ev.id(), Some(3));
    }
}
