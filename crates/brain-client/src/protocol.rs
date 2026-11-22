//! Messages between the Rust orchestrator and the Python brain server.
//! Must match `protocol/messages.json`, which the Python side follows.

use serde::{Deserialize, Serialize};

/// Rust → Python.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Command {
    /// Ask the brain for the next action and what to say.
    Think {
        id: u64,
        text: String,
        state: String,
        allowed: Vec<String>,
        /// Path to a screenshot when the question is about the screen.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        image: Option<String>,
    },
    /// Stop working on a request, for example because someone interrupted.
    Cancel { id: u64 },
}

/// Python → Rust.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    /// The model is loaded and the server accepts commands.
    Ready { model: String },
    /// One finished sentence of the reply, sent early so speech can start.
    Sentence { id: u64, text: String },
    /// The complete reply.
    Reply {
        id: u64,
        action: String,
        say: String,
    },
    /// The request failed; the orchestrator falls back to a safe line.
    Error { id: Option<u64>, message: String },
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
            image: None,
        };
        assert_eq!(
            serde_json::to_string(&cmd).unwrap(),
            r#"{"cmd":"think","id":7,"text":"Can you share your screen?","state":"LISTENING","allowed":["answer","share_screen"]}"#
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
    }
}
