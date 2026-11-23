//! The reviewer's output contract: what its final message must contain, how
//! Bridge reads it, and how each finding is checked against the diff.
//!
//! The final message holds exactly one fenced `json` block:
//!
//! ```json
//! {"summary":"…","findings":[{"path":"src/a.rs","line":42,"startLine":40,"side":"RIGHT",
//!   "severity":"major","title":"…","body":"markdown","confidence":0.8}]}
//! ```
//!
//! Reading is tolerant of prose around the block and strict on the fields
//! inside it, so a parse error is something a repair turn can quote back.
//! Validation never fails a pass: a finding whose path or lines are not in the
//! diff moves out of the inline comments into the summary (`outside_diff`).

use std::collections::{BTreeMap, HashSet};

use bridge_protocol::messages::{ReviewSeverity, ReviewSide};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use super::steps::DraftFinding;

/// Findings kept per pass, the most severe and most confident first.
pub const MAX_FINDINGS: usize = 50;
/// Longest title kept; longer ones are cut at a character boundary.
pub const MAX_TITLE_CHARS: usize = 120;

/// The contract, as the prompt states it.
pub const OUTPUT_CONTRACT: &str = r#"End your final message with exactly one fenced ```json block and nothing after it:

```json
{"summary":"<what the change does and your overall verdict, in a few sentences>",
 "findings":[{"path":"<a changed file, exactly as in files.json>","line":<line number>,
   "startLine":<first line, for a range; omit for one line>,"side":"RIGHT",
   "severity":"critical|major|minor|nit","title":"<one line, at most 120 characters>",
   "body":"<markdown: what is wrong, why it matters, how to fix it>","confidence":<0.0 to 1.0>}]}
```

- `side` is "RIGHT" for a line in the new version of the file and "LEFT" for a line only in the old version (a deleted line). Line numbers are those of that side.
- Comment only on lines inside the diff's hunks; anything else goes in the summary.
- Use `"findings":[]` when you found nothing worth raising. Never invent a problem to fill the list."#;

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RawReview {
    pub summary: String,
    pub findings: Vec<RawFinding>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RawFinding {
    pub path: String,
    pub line: u32,
    #[serde(default)]
    pub start_line: Option<u32>,
    #[serde(default = "right")]
    pub side: ReviewSide,
    pub severity: ReviewSeverity,
    pub title: String,
    pub body: String,
    pub confidence: f64,
}

fn right() -> ReviewSide {
    ReviewSide::Right
}

/// Read the one fenced `json` block out of a final message.
pub fn parse(message: &str) -> Result<RawReview, String> {
    let blocks = json_blocks(message);
    match blocks.len() {
        0 => Err("The final message has no fenced ```json block.".into()),
        1 => serde_json::from_str::<RawReview>(blocks[0]).map_err(|error| format!("The ```json block does not match the contract: {error}.")),
        count => Err(format!("The final message has {count} fenced ```json blocks; it must have exactly one.")),
    }
}

/// The bodies of every ```json fence, in order.
fn json_blocks(message: &str) -> Vec<&str> {
    let mut blocks = Vec::new();
    let mut rest = message;
    while let Some(open) = rest.find("```json") {
        let after = &rest[open + "```json".len()..];
        // The info string ends the line; the body starts on the next.
        let Some(newline) = after.find('\n') else { break };
        if !after[..newline].trim().is_empty() {
            rest = &after[newline..];
            continue;
        }
        let body = &after[newline + 1..];
        let Some(close) = find_closing_fence(body) else { break };
        blocks.push(body[..close].trim());
        rest = &body[close + 3..];
    }
    blocks
}

/// Where the fence closing a block starts: three backticks at the start of a
/// line.
fn find_closing_fence(body: &str) -> Option<usize> {
    let mut offset = 0;
    for line in body.split_inclusive('\n') {
        if line.trim_start().starts_with("```") {
            return Some(offset + (line.len() - line.trim_start().len()));
        }
        offset += line.len();
    }
    None
}

/// One hunk of one file, with the line numbers each side covers.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Hunk {
    pub old: Vec<u32>,
    pub new: Vec<u32>,
    /// The hunk's text, header included: what a fingerprint hashes.
    pub text: String,
}

impl Hunk {
    fn covers(&self, side: ReviewSide, line: u32) -> bool {
        match side {
            ReviewSide::Left => self.old.contains(&line),
            ReviewSide::Right => self.new.contains(&line),
        }
    }
}

/// The hunks of every file in a unified diff, keyed by the file's path on the
/// new side (the old side for a deletion).
pub fn hunks(patch: &str) -> BTreeMap<String, Vec<Hunk>> {
    let mut files: BTreeMap<String, Vec<Hunk>> = BTreeMap::new();
    let mut current: Option<String> = None;
    let mut old_path: Option<String> = None;
    let mut hunk: Option<(Hunk, u32, u32)> = None;
    let mut flush = |files: &mut BTreeMap<String, Vec<Hunk>>, current: &Option<String>, hunk: &mut Option<(Hunk, u32, u32)>| {
        if let (Some(path), Some((done, _, _))) = (current, hunk.take()) {
            files.entry(path.clone()).or_default().push(done);
        }
    };
    for line in patch.lines() {
        if line.starts_with("diff --git ") {
            flush(&mut files, &current, &mut hunk);
            current = None;
            old_path = None;
        } else if let Some(path) = line.strip_prefix("--- ") {
            if hunk.is_none() {
                old_path = side_path(path, "a/");
            }
        } else if let Some(path) = line.strip_prefix("+++ ") {
            if hunk.is_none() {
                current = side_path(path, "b/").or_else(|| old_path.clone());
            }
        } else if line.starts_with("@@ ") {
            flush(&mut files, &current, &mut hunk);
            if let Some((old_start, new_start)) = hunk_starts(line) {
                hunk = Some((Hunk { text: format!("{line}\n"), ..Hunk::default() }, old_start, new_start));
            }
        } else if let Some((open, old_line, new_line)) = hunk.as_mut() {
            open.text.push_str(line);
            open.text.push('\n');
            match line.as_bytes().first() {
                Some(b'+') => {
                    open.new.push(*new_line);
                    *new_line += 1;
                }
                Some(b'-') => {
                    open.old.push(*old_line);
                    *old_line += 1;
                }
                Some(b'\\') => {}
                _ => {
                    open.old.push(*old_line);
                    open.new.push(*new_line);
                    *old_line += 1;
                    *new_line += 1;
                }
            }
        }
    }
    flush(&mut files, &current, &mut hunk);
    files
}

/// `a/src/x.rs` or `"a/sp\tace"` to `src/x.rs`; `/dev/null` to nothing.
fn side_path(raw: &str, prefix: &str) -> Option<String> {
    let raw = raw.trim_end_matches('\t').trim();
    if raw == "/dev/null" {
        return None;
    }
    let unquoted = raw.strip_prefix('"').and_then(|rest| rest.strip_suffix('"')).map(unescape).unwrap_or_else(|| raw.to_owned());
    Some(unquoted.strip_prefix(prefix).unwrap_or(&unquoted).to_owned())
}

fn unescape(quoted: &str) -> String {
    let mut out = String::new();
    let mut chars = quoted.chars();
    while let Some(character) = chars.next() {
        if character != '\\' {
            out.push(character);
            continue;
        }
        match chars.next() {
            Some('t') => out.push('\t'),
            Some('n') => out.push('\n'),
            Some(other) => out.push(other),
            None => {}
        }
    }
    out
}

/// `@@ -12,3 +14,5 @@` to (12, 14).
fn hunk_starts(header: &str) -> Option<(u32, u32)> {
    let mut parts = header.split_whitespace().skip(1);
    let start = |part: Option<&str>, sign: char| -> Option<u32> {
        part?.strip_prefix(sign)?.split(',').next()?.parse().ok()
    };
    Some((start(parts.next(), '-')?, start(parts.next(), '+')?))
}

/// A title for comparison: lowercase words, punctuation dropped.
fn normalized(title: &str) -> String {
    title
        .to_lowercase()
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// `sha256(path + normalized title + hunk text)`, hex.
pub fn fingerprint(path: &str, title: &str, hunk_text: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(path.as_bytes());
    digest.update([0]);
    digest.update(normalized(title).as_bytes());
    digest.update([0]);
    digest.update(hunk_text.as_bytes());
    format!("{:x}", digest.finalize())
}

fn severity_rank(severity: ReviewSeverity) -> u8 {
    match severity {
        ReviewSeverity::Critical => 0,
        ReviewSeverity::Major => 1,
        ReviewSeverity::Minor => 2,
        ReviewSeverity::Nit => 3,
    }
}

fn cut(title: &str) -> String {
    let title = title.trim();
    match title.char_indices().nth(MAX_TITLE_CHARS) {
        Some((index, _)) => title[..index].trim_end().to_owned(),
        None => title.to_owned(),
    }
}

/// What a pass produced, checked against the diff.
#[derive(Debug, Clone, PartialEq)]
pub struct Validated {
    pub summary: String,
    /// Inline findings first, then those outside the diff; ranked within each.
    pub findings: Vec<DraftFinding>,
    /// Findings dropped as repeats of one already kept.
    pub duplicates: usize,
    /// Findings beyond [`MAX_FINDINGS`].
    pub over_limit: usize,
}

/// Check a parsed review against the files that changed and the diff's
/// hunks. A finding on a changed file whose lines all sit in one hunk on its
/// side is inline; anything else is kept, marked `outside_diff`.
pub fn validate(review: RawReview, changed: &HashSet<String>, hunks: &BTreeMap<String, Vec<Hunk>>) -> Validated {
    let mut seen = HashSet::new();
    let mut kept = Vec::new();
    let mut duplicates = 0;
    for raw in review.findings {
        let title = cut(&raw.title);
        if title.is_empty() {
            continue;
        }
        let key = (raw.path.clone(), raw.line, normalized(&title));
        if !seen.insert(key) {
            duplicates += 1;
            continue;
        }
        let start_line = raw.start_line.filter(|start| *start < raw.line);
        let hunk = changed
            .contains(&raw.path)
            .then(|| hunks.get(&raw.path))
            .flatten()
            .and_then(|file| {
                file.iter().find(|hunk| {
                    hunk.covers(raw.side, raw.line) && start_line.is_none_or(|start| hunk.covers(raw.side, start))
                })
            });
        let confidence = if raw.confidence.is_finite() { raw.confidence.clamp(0.0, 1.0) } else { 0.0 };
        kept.push(DraftFinding {
            fingerprint: fingerprint(&raw.path, &title, hunk.map_or("", |hunk| hunk.text.as_str())),
            path: raw.path,
            line: raw.line,
            start_line,
            side: raw.side,
            severity: raw.severity,
            title,
            body: raw.body.trim().to_owned(),
            confidence,
            outside_diff: hunk.is_none(),
        });
    }
    kept.sort_by(|left, right| {
        severity_rank(left.severity)
            .cmp(&severity_rank(right.severity))
            .then(right.confidence.total_cmp(&left.confidence))
    });
    let over_limit = kept.len().saturating_sub(MAX_FINDINGS);
    kept.truncate(MAX_FINDINGS);
    // Inline first, so the review's comments lead and the rest follows.
    kept.sort_by_key(|finding| finding.outside_diff);
    Validated { summary: review.summary.trim().to_owned(), findings: kept, duplicates, over_limit }
}

#[cfg(test)]
#[path = "contract_tests.rs"]
mod tests;
