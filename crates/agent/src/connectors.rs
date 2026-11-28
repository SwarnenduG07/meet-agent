//! Turns a planning thread (Codex or Claude Code, read from local files) into a
//! draft brief. Only the conversation is used: tool calls, tool output and
//! reasoning are skipped, code blocks are dropped, and anything that looks like
//! a key or token is redacted. The draft is for you to review before a meeting.

use std::{
    path::{Path, PathBuf},
    sync::LazyLock,
    time::SystemTime,
};

use anyhow::{Context, Result, bail};
use brain_client::Brain;
use regex::Regex;
use serde_json::Value;

use crate::conversation::log;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Codex,
    Claude,
}

pub struct Thread {
    pub path: PathBuf,
    pub title: String,
    /// `(speaker, text)`, speaker is "USER" or "AGENT".
    pub messages: Vec<(&'static str, String)>,
}

/// Thread files, newest first.
///
/// # Errors
/// Fails when the sessions folder can't be read.
pub fn files(source: Source) -> Result<Vec<PathBuf>> {
    let home = PathBuf::from(std::env::var_os("HOME").context("HOME is not set")?);
    let root = match source {
        Source::Codex => home.join(".codex/sessions"),
        Source::Claude => home.join(".claude/projects"),
    };
    let mut found = Vec::new();
    collect_jsonl(&root, &mut found, 0)?;
    found.sort_by_key(|(_, modified)| std::cmp::Reverse(*modified));
    Ok(found.into_iter().map(|(p, _)| p).collect())
}

fn collect_jsonl(dir: &Path, out: &mut Vec<(PathBuf, SystemTime)>, depth: usize) -> Result<()> {
    if depth > 4 {
        return Ok(());
    }
    for entry in std::fs::read_dir(dir)
        .with_context(|| format!("reading {}", dir.display()))?
        .flatten()
    {
        let path = entry.path();
        if path.is_dir() {
            collect_jsonl(&path, out, depth + 1)?;
        } else if path.extension().is_some_and(|e| e == "jsonl") {
            let modified = entry
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            out.push((path, modified));
        }
    }
    Ok(())
}

/// `latest`, a path, or part of a file name (e.g. a session id).
///
/// # Errors
/// Fails when nothing matches.
pub fn find(source: Source, which: &str) -> Result<PathBuf> {
    if Path::new(which).is_file() {
        return Ok(which.into());
    }
    let all = files(source)?;
    let found = if which == "latest" {
        all.first()
    } else {
        all.iter().find(|p| p.to_string_lossy().contains(which))
    };
    found
        .cloned()
        .with_context(|| format!("no {source:?} thread matches {which:?}"))
}

/// Reads a thread's messages.
///
/// # Errors
/// Fails when the file can't be read.
pub fn read(source: Source, path: &Path) -> Result<Thread> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let mut thread = Thread {
        path: path.to_owned(),
        title: String::new(),
        messages: Vec::new(),
    };
    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        match source {
            Source::Codex => read_codex_line(&v, &mut thread),
            Source::Claude => read_claude_line(&v, &mut thread),
        }
    }
    if thread.title.is_empty() {
        thread.title = thread
            .messages
            .iter()
            .find(|(who, _)| *who == "USER")
            .map_or_else(
                || "Planning thread".to_owned(),
                |(_, t)| t.chars().take(60).collect(),
            );
    }
    Ok(thread)
}

fn read_codex_line(v: &Value, thread: &mut Thread) {
    let payload = &v["payload"];
    if v["type"] != "response_item" || payload["type"] != "message" {
        return;
    }
    let who = match payload["role"].as_str() {
        Some("user") => "USER",
        Some("assistant") => "AGENT",
        _ => return,
    };
    let text: Vec<&str> = payload["content"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|item| matches!(item["type"].as_str(), Some("input_text" | "output_text")))
        .filter_map(|item| item["text"].as_str())
        .collect();
    push(thread, who, &text.join("\n"));
}

fn read_claude_line(v: &Value, thread: &mut Thread) {
    if v["type"] == "custom-title" {
        if let Some(t) = v["customTitle"].as_str() {
            t.clone_into(&mut thread.title);
        }
        return;
    }
    let who = match v["type"].as_str() {
        Some("user") => "USER",
        Some("assistant") => "AGENT",
        _ => return,
    };
    let content = &v["message"]["content"];
    let text = if let Some(s) = content.as_str() {
        s.to_owned()
    } else {
        content
            .as_array()
            .into_iter()
            .flatten()
            .filter(|b| b["type"] == "text")
            .filter_map(|b| b["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n")
    };
    push(thread, who, &text);
}

/// Keeps real conversation: drops injected context blocks and empty messages.
fn push(thread: &mut Thread, who: &'static str, text: &str) {
    let t = text.trim();
    let injected = t.starts_with('<')
        && t.contains("</")
        && t.find('>').is_some_and(|i| !t[1..i].contains(' '));
    if t.is_empty() || injected || t.starts_with("[Request interrupted") {
        return;
    }
    thread.messages.push((who, clean(t)));
}

static CODE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)```.*?```").expect("valid regex"));
static SECRET: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?x)
        sk-[A-Za-z0-9_-]{16,} | sk-ant-[A-Za-z0-9_-]{16,} | gh[pousr]_[A-Za-z0-9]{20,} | github_pat_[A-Za-z0-9_]{20,}
        | AKIA[0-9A-Z]{16} | xox[abprs]-[A-Za-z0-9-]{10,} | hf_[A-Za-z0-9]{20,}
        | (?i:bearer)\s+[A-Za-z0-9._~+/-]{16,}=*
        | \b[A-Za-z0-9+/_-]{40,}={0,2}",
    )
    .expect("valid regex")
});

/// Drops code blocks, redacts likely secrets, and caps the length.
#[must_use]
pub fn clean(text: &str) -> String {
    let no_code = CODE.replace_all(text, "[code omitted]");
    let redacted = SECRET.replace_all(&no_code, "[redacted]");
    redacted.chars().take(1500).collect()
}

/// Condenses the thread with the local brain and writes a draft brief.
///
/// # Errors
/// Fails when the brain fails or the file can't be written.
pub async fn draft_brief(brain: &Brain, thread: &Thread, owner: &str, out: &Path) -> Result<()> {
    if thread.messages.is_empty() {
        bail!("{} has no conversation to summarize", thread.path.display());
    }
    let transcript: Vec<String> = thread
        .messages
        .iter()
        .map(|(who, t)| format!("{who}: {t}"))
        .collect();
    let chunks = chunk(&transcript, 6000);
    // Long threads: the start (the idea) and the most recent part (the current plan).
    let picked: Vec<&String> = if chunks.len() > 24 {
        chunks
            .iter()
            .take(2)
            .chain(chunks.iter().skip(chunks.len() - 22))
            .collect()
    } else {
        chunks.iter().collect()
    };
    log(
        "brief",
        &format!(
            "{} messages, {} chunks to read",
            thread.messages.len(),
            picked.len()
        ),
    );

    let mut notes = Vec::new();
    for (i, part) in picked.iter().enumerate() {
        let prompt = format!(
            "Below is part {} of {} of a planning conversation between {owner} and an AI assistant.\n\
             List up to 8 short facts about what is being built: purpose, how it works, key decisions, \
             status, open questions. Only what was decided or done, not options that were merely \
             mentioned, compared or rejected. Plain bullet points, no code, no file paths, nothing \
             private.\n\n{part}",
            i + 1,
            picked.len()
        );
        let facts = brain.complete(&prompt, 300, None).await?;
        notes.push(format!(
            "NOTES FROM PART {} OF {}:\n{facts}",
            i + 1,
            picked.len()
        ));
        log("brief", &format!("read chunk {}/{}", i + 1, picked.len()));
    }

    let prompt = format!(
        "From these notes, write a meeting brief so an agent can present the project on {owner}'s behalf.\n\
         Use EXACTLY this markdown format, nothing before or after:\n\n\
         # <short project title>\n\n## About\n<3-5 sentences>\n\n## Sections\n\n### 1. <title>\n- <talking point>\n- <talking point>\n\n\
         ### 2. <title>\n- <talking point>\n\n## FAQ\n- <question>? <answer>\n\n## Do not discuss\n- Pricing, deadlines, hiring. Say {owner} will follow up.\n\n\
         Use 2 to 4 sections, 2 to 4 talking points each, only facts from the notes. The notes are in \
         order: when they disagree, later parts win, because plans changed over time. Leave out \
         tools or approaches that later notes replaced.\n\nNOTES:\n{}",
        notes.join("\n")
    );
    let body = brain.complete(&prompt, 900, None).await?;
    let draft = format!(
        "---\nshare: on_request\n---\n{}\n",
        body.trim()
            .trim_start_matches("```markdown")
            .trim_end_matches("```")
            .trim()
    );
    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(out, draft).with_context(|| format!("writing {}", out.display()))?;
    Ok(())
}

fn chunk(lines: &[String], max_chars: usize) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current = String::new();
    for line in lines {
        if !current.is_empty() && current.len() + line.len() > max_chars {
            chunks.push(std::mem::take(&mut current));
        }
        current.push_str(line);
        current.push('\n');
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

#[cfg(test)]
mod tests {
    use super::clean;

    #[test]
    fn cleaning_removes_code_and_secrets() {
        let text =
            "Use key sk-ant-REDACTEDREDACTEDREDACTED now.\n```rust\nfn main() {}\n```\nDone.";
        let out = clean(text);
        assert!(!out.contains("sk-ant-REDACTED"));
        assert!(out.contains("[redacted]"));
        assert!(out.contains("[code omitted]"));
        assert!(!out.contains("fn main"));
    }
}
