//! Text → phonemes for Kokoro. espeak-ng gives IPA; Kokoro was trained on
//! misaki's variant of it (single letters for diphthongs, no length marks), so
//! the IPA is converted to that style.

use std::process::Command;

use anyhow::{Context, Result, bail};

/// espeak IPA → misaki, longest patterns first.
const TO_MISAKI: [(&str, &str); 19] = [
    ("ɚɹ", "əɹ"),
    ("ɜː", "ɜɹ"),
    ("aɪ", "I"),
    ("aʊ", "W"),
    ("eɪ", "A"),
    ("ɔɪ", "Y"),
    ("oʊ", "O"),
    ("dʒ", "ʤ"),
    ("tʃ", "ʧ"),
    ("ɚ", "əɹ"),
    ("ɾ", "T"),
    ("ᵻ", "ə"),
    ("ɐ", "ə"),
    ("ː", ""),
    ("r", "ɹ"),
    ("x", "k"),
    ("ç", "k"),
    ("ɬ", "l"),
    ("ʔ", "t"),
];

const PUNCTUATION: &[char] = &[',', '.', '!', '?', ';', ':'];

/// Phonemes for `text`, keeping punctuation so Kokoro pauses in the right places.
///
/// # Errors
/// Fails when espeak-ng isn't installed or fails.
pub fn phonemize(text: &str) -> Result<String> {
    let mut out = String::new();
    let mut segment = String::new();
    for ch in text.chars() {
        if PUNCTUATION.contains(&ch) {
            push_segment(&mut out, &segment)?;
            segment.clear();
            out.push(ch);
        } else {
            segment.push(ch);
        }
    }
    push_segment(&mut out, &segment)?;
    Ok(out.trim().to_owned())
}

fn push_segment(out: &mut String, segment: &str) -> Result<()> {
    if segment.trim().is_empty() {
        return Ok(());
    }
    if !out.is_empty() && !out.ends_with(' ') {
        out.push(' ');
    }
    out.push_str(&to_misaki(&espeak_ipa(segment)?));
    Ok(())
}

fn espeak_ipa(text: &str) -> Result<String> {
    let output = Command::new("espeak-ng")
        .args(["-q", "--ipa", "-v", "en-us", "--", text])
        .output()
        .context("running espeak-ng (brew install espeak-ng)")?;
    if !output.status.success() {
        bail!(
            "espeak-ng failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" "))
}

fn to_misaki(ipa: &str) -> String {
    let mut s = ipa.to_owned();
    for (from, to) in TO_MISAKI {
        s = s.replace(from, to);
    }
    s
}
