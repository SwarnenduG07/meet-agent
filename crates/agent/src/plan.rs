//! The brief (`briefs/*.md`) and the plan the agent follows in the meeting.
//!
//! Brief format:
//! ```text
//! ---
//! link: https://meet.google.com/abc-defg-hij
//! start: 15:00              (optional, local time today)
//! share: at_start | on_request | never
//! ---
//! # Title
//! Free text the agent knows about.
//! ## Sections
//! ### 1. Section title
//! show: app, file or URL to bring forward
//! point: what to point at (screen tools)
//! - talking point
//! ## FAQ / ## Do not discuss
//! ```

use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SharePolicy {
    /// Share and present as soon as the agent is in the call.
    AtStart,
    /// Share only when someone asks.
    #[default]
    OnRequest,
    /// Never share.
    Never,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Section {
    pub title: String,
    pub show: Option<String>,
    pub point: Option<String>,
    pub talk: Vec<String>,
    /// What the agent says, written by the brain before the meeting.
    #[serde(default)]
    pub script: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plan {
    pub title: String,
    pub link: Option<String>,
    pub start: Option<String>,
    pub share: SharePolicy,
    /// The brief without its header: everything the agent may talk about.
    pub context: String,
    pub sections: Vec<Section>,
}

impl Plan {
    /// Reads and parses a brief.
    ///
    /// # Errors
    /// Fails when the file can't be read or the header is malformed.
    pub fn from_brief(path: &Path) -> Result<Self> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        parse(&text)
    }

    /// Writes `plan.json` next to the meeting's other files.
    ///
    /// # Errors
    /// Fails when the file can't be written.
    pub fn save(&self, path: &Path) -> Result<()> {
        std::fs::write(path, serde_json::to_string_pretty(self)?)?;
        Ok(())
    }
}

fn parse(text: &str) -> Result<Plan> {
    let (header, body) = split_front_matter(text);
    let mut plan = Plan {
        title: String::new(),
        link: None,
        start: None,
        share: SharePolicy::default(),
        context: body.trim().to_owned(),
        sections: Vec::new(),
    };
    for line in header.lines().filter(|l| !l.trim().is_empty()) {
        let (key, value) = line
            .split_once(':')
            .with_context(|| format!("brief header line {line:?} has no ':'"))?;
        let value = value.trim().to_owned();
        match key.trim() {
            "link" => plan.link = Some(value),
            "start" => plan.start = Some(value),
            "share" => {
                plan.share = match value.as_str() {
                    "at_start" => SharePolicy::AtStart,
                    "on_request" => SharePolicy::OnRequest,
                    "never" => SharePolicy::Never,
                    other => bail!("share must be at_start, on_request or never, not {other:?}"),
                }
            }
            other => bail!("unknown brief header field {other:?}"),
        }
    }

    let mut in_sections = false;
    for line in body.lines() {
        let trimmed = line.trim();
        if let Some(title) = trimmed.strip_prefix("# ") {
            title.trim().clone_into(&mut plan.title);
        } else if let Some(h2) = trimmed.strip_prefix("## ") {
            in_sections = h2.trim().eq_ignore_ascii_case("sections");
        } else if let Some(h3) = trimmed.strip_prefix("### ").filter(|_| in_sections) {
            let title = h3
                .trim()
                .trim_start_matches(|c: char| c.is_ascii_digit() || c == '.')
                .trim();
            plan.sections.push(Section {
                title: title.to_owned(),
                show: None,
                point: None,
                talk: Vec::new(),
                script: Vec::new(),
            });
        } else if let Some(section) = plan.sections.last_mut().filter(|_| in_sections) {
            if let Some(v) = trimmed.strip_prefix("show:") {
                section.show = Some(v.trim().to_owned());
            } else if let Some(v) = trimmed.strip_prefix("point:") {
                section.point = Some(v.trim().to_owned());
            } else if let Some(v) = trimmed.strip_prefix("- ") {
                section.talk.push(v.trim().to_owned());
            } else if !trimmed.is_empty() {
                section.talk.push(trimmed.to_owned());
            }
        }
    }
    if plan.title.is_empty() {
        plan.title = "Meeting".into();
    }
    Ok(plan)
}

fn split_front_matter(text: &str) -> (&str, &str) {
    let rest = text.trim_start();
    if let Some(after) = rest.strip_prefix("---")
        && let Some(end) = after.find("\n---")
    {
        let header = &after[..end];
        let body = after[end + 4..].trim_start_matches(['-', '\n']);
        return (header, body);
    }
    ("", text)
}

/// Splits text into sentences for speaking one at a time.
pub fn sentences(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let chars: Vec<char> = text.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        current.push(c);
        let end =
            matches!(c, '.' | '!' | '?') && chars.get(i + 1).is_none_or(|n| n.is_whitespace());
        if end && !current.trim().is_empty() {
            out.push(current.trim().to_owned());
            current.clear();
        }
    }
    if !current.trim().is_empty() {
        out.push(current.trim().to_owned());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_example_brief() {
        let plan = Plan::from_brief(Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../briefs/example.md"
        )))
        .unwrap();
        assert_eq!(plan.title, "Flowline architecture walkthrough");
        assert_eq!(plan.share, SharePolicy::AtStart);
        assert_eq!(plan.start.as_deref(), Some("15:00"));
        assert_eq!(plan.sections.len(), 2);
        assert_eq!(plan.sections[1].title, "Architecture");
        assert_eq!(
            plan.sections[1].show.as_deref(),
            Some("docs/architecture.png")
        );
        assert_eq!(plan.sections[1].point.as_deref(), Some("Orchestrator"));
        assert_eq!(plan.sections[1].talk.len(), 4);
        assert!(plan.context.contains("Do not discuss"));
        assert!(!plan.context.contains("link:"));
    }
}
