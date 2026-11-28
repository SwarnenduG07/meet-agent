//! Everything that happens in a meeting, as JSON lines in
//! `runs/<date>_<name>/timeline.jsonl`, and the summary written afterwards.

use std::{
    fmt::Write as _,
    fs::{File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};
use serde::Serialize;
use serde_json::json;

pub struct Timeline {
    pub dir: PathBuf,
    file: File,
    pub questions: Vec<(String, String)>,
    pub deferred: Vec<String>,
    pub covered: Vec<String>,
}

impl Timeline {
    /// Creates `runs/<YYYYmmdd-HHMMSS>_<slug>/`.
    ///
    /// # Errors
    /// Fails when the folder can't be created.
    pub fn create(runs: &Path, title: &str) -> Result<Self> {
        let slug: String = title
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
        let stamp = std::process::Command::new("date")
            .arg("+%Y%m%d-%H%M%S")
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
            .unwrap_or_default();
        let dir = runs.join(format!("{stamp}_{slug}"));
        std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join("timeline.jsonl"))?;
        Ok(Self {
            dir,
            file,
            questions: Vec::new(),
            deferred: Vec::new(),
            covered: Vec::new(),
        })
    }

    /// Appends one event.
    pub fn record(&mut self, kind: &str, data: impl Serialize) {
        let at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_millis());
        let line = json!({ "at_ms": at, "kind": kind, "data": data });
        let _ = writeln!(self.file, "{line}");
    }

    /// Writes `summary.md`: sections covered, questions, and follow-ups for the owner.
    ///
    /// # Errors
    /// Fails when the file can't be written.
    pub fn write_summary(&self, title: &str, owner: &str, recap: Option<&str>) -> Result<PathBuf> {
        let mut md = format!("# {title}: meeting summary\n\n");
        if let Some(recap) = recap {
            md.push_str(recap.trim());
            md.push_str("\n\n");
        }
        md.push_str("## Covered\n\n");
        if self.covered.is_empty() {
            md.push_str("- (no planned sections presented)\n");
        }
        for s in &self.covered {
            let _ = writeln!(md, "- {s}");
        }
        let _ = writeln!(md, "\n## Follow up ({owner})\n");
        if self.deferred.is_empty() {
            md.push_str("- Nothing was deferred.\n");
        }
        for q in &self.deferred {
            let _ = writeln!(md, "- {q}");
        }
        md.push_str("\n## Questions and answers\n\n");
        if self.questions.is_empty() {
            md.push_str("- No questions.\n");
        }
        for (q, a) in &self.questions {
            let _ = writeln!(md, "- **{q}**\n  {a}");
        }
        let path = self.dir.join("summary.md");
        std::fs::write(&path, md)?;
        Ok(path)
    }
}
