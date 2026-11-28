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
 "findings":[{"path":"<a changed file, exactly as in files.json>","line":<the line the problem is on>,
   "startLine":<first line, for a range; omit for one line>,"side":"RIGHT",
   "severity":"critical|major|minor|nit","category":"security|correctness|error-handling|tests|performance|naming|docs|style|other",
   "title":"<one line, at most 120 characters>",
   "body":"<markdown: what is wrong, why it matters, how to fix it>","confidence":<0.0 to 1.0>}]}
```

- `side` is "RIGHT" for a line in the new version of the file and "LEFT" for a line only in the old version (a deleted line). Line numbers are those of that side.
- `line` is the line the problem is on: the faulty condition, call or statement itself, never the closing brace or last line of the block, function or hunk around it. A problem spanning several lines gives its first line as `startLine` and its last as `line`.
- `category` is what the finding is about; use `other` when none fits.
- Comment only on lines inside the diff's hunks; anything else goes in the summary.
- Use `"findings":[]` when you found nothing worth raising. Never invent a problem to fill the list.
- On a re-review only, add `"fixed":["<id>"]` with the `id` of each earlier finding in prior_findings.json that the new commits fixed. Leave it out otherwise."#;

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawReview {
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub findings: Vec<RawFinding>,
    /// On a re-review, the earlier findings the new commits fixed.
    #[serde(default)]
    pub fixed: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawFinding {
    pub path: String,
    #[serde(deserialize_with = "line_number")]
    pub line: u32,
    #[serde(default, deserialize_with = "optional_line_number")]
    pub start_line: Option<u32>,
    #[serde(default = "right", deserialize_with = "side")]
    pub side: ReviewSide,
    #[serde(deserialize_with = "severity")]
    pub severity: ReviewSeverity,
    pub title: String,
    #[serde(default)]
    pub body: String,
    #[serde(default = "middling", deserialize_with = "confidence")]
    pub confidence: f64,
    /// Read leniently: anything not one of the known categories is dropped
    /// in validation, so a model's own word never fails a pass.
    #[serde(default, deserialize_with = "loose_text")]
    pub category: Option<String>,
    /// The taste rules that shaped the finding. Read leniently; only ids the
    /// pass was given survive.
    #[serde(default, deserialize_with = "loose_ids")]
    pub rule_ids: Vec<String>,
}

fn loose_ids<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Vec<String>, D::Error> {
    Ok(Option::<serde_json::Value>::deserialize(deserializer)?
        .and_then(|value| value.as_array().cloned())
        .unwrap_or_default()
        .into_iter()
        .filter_map(|value| value.as_str().map(str::trim).filter(|id| !id.is_empty()).map(str::to_owned))
        .collect())
}

fn loose_text<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Option<String>, D::Error> {
    Ok(Option::<serde_json::Value>::deserialize(deserializer)?.and_then(|value| value.as_str().map(str::to_owned)))
}

fn right() -> ReviewSide {
    ReviewSide::Right
}

/// A finding that gave no confidence is neither sure nor a guess.
fn middling() -> f64 {
    0.5
}

/// A number, or a number written as text (`"42"`): models write both.
#[derive(Deserialize)]
#[serde(untagged)]
enum Numeric {
    Number(serde_json::Number),
    Text(String),
}

impl Numeric {
    fn as_f64(&self) -> Option<f64> {
        match self {
            Numeric::Number(number) => number.as_f64(),
            Numeric::Text(text) => text.trim().parse().ok(),
        }
    }
}

fn line_number<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<u32, D::Error> {
    let value = Numeric::deserialize(deserializer)?;
    value
        .as_f64()
        .filter(|line| line.fract() == 0.0 && *line >= 1.0 && *line <= f64::from(u32::MAX))
        .map(|line| line as u32)
        .ok_or_else(|| serde::de::Error::custom("invalid value: a line number must be a whole number from 1"))
}

fn optional_line_number<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Option<u32>, D::Error> {
    let value = Option::<Numeric>::deserialize(deserializer)?;
    Ok(value.and_then(|value| value.as_f64()).filter(|line| line.fract() == 0.0 && *line >= 1.0).map(|line| line as u32))
}

fn confidence<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<f64, D::Error> {
    Ok(Option::<Numeric>::deserialize(deserializer)?.and_then(|value| value.as_f64()).unwrap_or_else(middling))
}

fn side<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<ReviewSide, D::Error> {
    let text = Option::<String>::deserialize(deserializer)?.unwrap_or_default();
    match text.trim().to_ascii_lowercase().as_str() {
        "" | "right" | "new" => Ok(ReviewSide::Right),
        "left" | "old" => Ok(ReviewSide::Left),
        other => Err(serde::de::Error::custom(format!("unknown variant `{other}`, expected `RIGHT` or `LEFT`"))),
    }
}

/// The contract's four severities, and the words models use for them.
fn severity<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<ReviewSeverity, D::Error> {
    let text = String::deserialize(deserializer)?;
    match text.trim().to_ascii_lowercase().as_str() {
        "critical" | "blocker" | "severe" => Ok(ReviewSeverity::Critical),
        "major" | "high" | "error" | "important" => Ok(ReviewSeverity::Major),
        "minor" | "medium" | "moderate" | "warning" => Ok(ReviewSeverity::Minor),
        "nit" | "low" | "info" | "trivial" | "style" | "suggestion" => Ok(ReviewSeverity::Nit),
        other => Err(serde::de::Error::custom(format!("unknown variant `{other}`, expected one of critical, major, minor, nit"))),
    }
}

/// Read the review out of a final message.
///
/// The contract asks for one fenced `json` block, and that is what is read
/// first. Models drift from it in harmless ways — a bare JSON object, a fence
/// labelled `JSON` or nothing, an example block before the real one, a
/// field the contract does not name — and a review lost to any of those is
/// a review the person waited for and did not get. So every JSON object the
/// message holds is a candidate, the last first, and the first that reads as
/// a review wins. An error means no candidate did, worded so a repair turn
/// can quote it back.
pub fn parse(message: &str) -> Result<RawReview, String> {
    let mut candidates = fenced_blocks(message);
    if candidates.is_empty() {
        candidates = bare_objects(message);
    }
    let mut mismatch = None;
    for candidate in candidates.iter().rev() {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(candidate) else {
            mismatch.get_or_insert_with(|| {
                let error = serde_json::from_str::<serde_json::Value>(candidate).err().map(|error| error.to_string()).unwrap_or_default();
                format!("The ```json block does not match the contract: {error}.")
            });
            continue;
        };
        let Some(object) = value.as_object() else { continue };
        if !object.contains_key("findings") && !object.contains_key("summary") {
            continue;
        }
        match serde_json::from_value::<RawReview>(value) {
            Ok(review) => return Ok(review),
            Err(error) => {
                mismatch.get_or_insert_with(|| format!("The ```json block does not match the contract: {error}."));
            }
        }
    }
    Err(mismatch.unwrap_or_else(|| "The final message has no fenced ```json block.".into()))
}

/// The bodies of every fence that could hold JSON (labelled `json`, or not
/// labelled at all), in order. A fence still open when the message ends runs
/// to its end.
fn fenced_blocks(message: &str) -> Vec<&str> {
    let mut blocks = Vec::new();
    // The open fence's marker, its length, where its body starts, and
    // whether it can hold JSON. Every fence is tracked, so the end of a
    // ```rust block is never read as the start of another.
    let mut open: Option<(char, usize, usize, bool)> = None;
    let mut offset = 0;
    for line in message.split_inclusive('\n') {
        let start = offset;
        offset += line.len();
        let trimmed = line.trim();
        match open {
            Some((marker, length, body, json)) => {
                let run = trimmed.chars().take_while(|character| *character == marker).count();
                if run >= length && trimmed.len() == run {
                    if json {
                        blocks.push(message[body..start].trim());
                    }
                    open = None;
                }
            }
            None => {
                let at_start = trimmed.starts_with("```") || trimmed.starts_with("~~~");
                // "Here it is: ```json" opens a block too.
                let lower = trimmed.to_ascii_lowercase();
                let trailing = ["```json", "~~~json"].iter().find_map(|fence| lower.ends_with(fence).then(|| trimmed.len() - fence.len()));
                let Some(at) = (if at_start { Some(0) } else { trailing }) else { continue };
                let marker = trimmed[at..].chars().next().unwrap_or('`');
                let length = trimmed[at..].chars().take_while(|character| *character == marker).count();
                let info = trimmed[at + length..].trim().to_ascii_lowercase();
                if at_start && info.contains(marker) {
                    // A one-line span, ```like this```: not a block.
                    continue;
                }
                let json = info.is_empty() || matches!(info.as_str(), "json" | "jsonc" | "json5");
                open = Some((marker, length, offset, json));
            }
        }
    }
    if let Some((_, _, body, true)) = open {
        blocks.push(message[body..].trim());
    }
    blocks
}

/// Every top-level JSON object written straight into the message, in order.
fn bare_objects(message: &str) -> Vec<&str> {
    let mut objects = Vec::new();
    let mut from = 0;
    while let Some(found) = message[from..].find('{') {
        let start = from + found;
        let mut stream = serde_json::Deserializer::from_str(&message[start..]).into_iter::<serde_json::Value>();
        match stream.next() {
            Some(Ok(serde_json::Value::Object(_))) => {
                let end = start + stream.byte_offset();
                objects.push(&message[start..end]);
                from = end;
            }
            _ => from = start + 1,
        }
    }
    objects
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

    /// One side's lines with their numbers, as the diff shows them.
    fn side_lines(&self, side: ReviewSide) -> Vec<(u32, &str)> {
        let numbers = match side {
            ReviewSide::Left => &self.old,
            ReviewSide::Right => &self.new,
        };
        let body = self.text.lines().skip(1).filter_map(|line| match (line.as_bytes().first(), side) {
            (Some(b'\\'), _) | (Some(b'+'), ReviewSide::Left) | (Some(b'-'), ReviewSide::Right) => None,
            (Some(b'+' | b'-' | b' '), _) => Some(&line[1..]),
            _ => Some(line),
        });
        numbers.iter().copied().zip(body).collect()
    }
}

/// Where the block closed on `line` opens, when `line` holds nothing but
/// closing brackets (`}`, `});`) and the opening sits earlier in the same
/// hunk. A reviewer that names a block's closing line means the block: Claude
/// reports a bug in an `if` condition on the `if`'s closing brace (#1151).
/// A continuation such as `} else {` belongs to the block it continues, so the
/// start is the `if`, not the `else`.
fn block_start(hunk: &Hunk, side: ReviewSide, line: u32) -> Option<u32> {
    let lines = hunk.side_lines(side);
    let at = lines.iter().position(|(number, _)| *number == line)?;
    let closing = lines[at].1.trim();
    if !closing.contains(['}', ')', ']']) || !closing.chars().all(|ch| matches!(ch, '}' | ')' | ']' | ';' | ',') || ch.is_whitespace()) {
        return None;
    }
    let mut depth = 0i64;
    for (number, text) in lines[..=at].iter().rev() {
        for ch in text.chars() {
            match ch {
                '}' | ')' | ']' => depth += 1,
                '{' | '(' | '[' => depth -= 1,
                _ => {}
            }
        }
        if *number != line && depth <= 0 {
            return Some(*number);
        }
    }
    None
}

/// An open hunk: what it holds so far, the next line on each side, and how
/// many lines of each side its header said are still to come.
struct OpenHunk {
    hunk: Hunk,
    old_line: u32,
    new_line: u32,
    old_left: u32,
    new_left: u32,
}

/// The hunks of every file in a unified diff, keyed by the file's path on the
/// new side (the old side for a deletion).
///
/// A hunk runs for exactly the line counts its header gives, so a content line
/// that looks like a header (a deleted `-- comment` reads `--- comment`) is
/// still content. Headers are read only between hunks.
pub fn hunks(patch: &str) -> BTreeMap<String, Vec<Hunk>> {
    let mut files: BTreeMap<String, Vec<Hunk>> = BTreeMap::new();
    let mut current: Option<String> = None;
    let mut old_path: Option<String> = None;
    let mut open: Option<OpenHunk> = None;
    let close = |files: &mut BTreeMap<String, Vec<Hunk>>, current: &Option<String>, open: &mut Option<OpenHunk>| {
        if let (Some(path), Some(done)) = (current, open.take()) {
            files.entry(path.clone()).or_default().push(done.hunk);
        }
    };
    for line in patch.lines() {
        if let Some(hunk) = open.as_mut() {
            if hunk.old_left > 0 || hunk.new_left > 0 {
                hunk.hunk.text.push_str(line);
                hunk.hunk.text.push('\n');
                match line.as_bytes().first() {
                    Some(b'+') => {
                        hunk.hunk.new.push(hunk.new_line);
                        hunk.new_line += 1;
                        hunk.new_left = hunk.new_left.saturating_sub(1);
                    }
                    Some(b'-') => {
                        hunk.hunk.old.push(hunk.old_line);
                        hunk.old_line += 1;
                        hunk.old_left = hunk.old_left.saturating_sub(1);
                    }
                    // "\ No newline at end of file" belongs to the line before.
                    Some(b'\\') => {}
                    _ => {
                        hunk.hunk.old.push(hunk.old_line);
                        hunk.hunk.new.push(hunk.new_line);
                        hunk.old_line += 1;
                        hunk.new_line += 1;
                        hunk.old_left = hunk.old_left.saturating_sub(1);
                        hunk.new_left = hunk.new_left.saturating_sub(1);
                    }
                }
                continue;
            }
            if line.starts_with("\\") {
                hunk.hunk.text.push_str(line);
                hunk.hunk.text.push('\n');
                continue;
            }
            close(&mut files, &current, &mut open);
        }
        if line.starts_with("diff --git ") {
            current = None;
            old_path = None;
        } else if let Some(path) = line.strip_prefix("--- ") {
            old_path = side_path(path, "a/");
        } else if let Some(path) = line.strip_prefix("+++ ") {
            current = side_path(path, "b/").or_else(|| old_path.clone());
        } else if line.starts_with("@@ ") {
            if let Some((old_line, old_left, new_line, new_left)) = hunk_header(line) {
                open = Some(OpenHunk {
                    hunk: Hunk { text: format!("{line}\n"), ..Hunk::default() },
                    old_line,
                    new_line,
                    old_left,
                    new_left,
                });
            }
        }
    }
    close(&mut files, &current, &mut open);
    files
}

/// `a/src/x.rs` or `"a/sp\tace"` to `src/x.rs`; `/dev/null` to nothing.
fn side_path(raw: &str, prefix: &str) -> Option<String> {
    let raw = raw.trim_end_matches('\t').trim();
    if raw == "/dev/null" {
        return None;
    }
    let unquoted = raw.strip_prefix('"').and_then(|rest| rest.strip_suffix('"')).map(unquote).unwrap_or_else(|| raw.to_owned());
    Some(unquoted.strip_prefix(prefix).unwrap_or(&unquoted).to_owned())
}

/// Git's C-style quoting: `\t`, `\n`, `\"`, `\\` and octal escapes, the
/// octal ones being the bytes of a UTF-8 name (`\346\227\245` is `日`).
fn unquote(quoted: &str) -> String {
    let mut bytes = Vec::with_capacity(quoted.len());
    let mut chars = quoted.chars().peekable();
    while let Some(character) = chars.next() {
        if character != '\\' {
            let mut buffer = [0; 4];
            bytes.extend_from_slice(character.encode_utf8(&mut buffer).as_bytes());
            continue;
        }
        match chars.next() {
            Some(digit @ '0'..='7') => {
                let mut value = digit.to_digit(8).unwrap_or(0);
                for _ in 0..2 {
                    match chars.peek().and_then(|next| next.to_digit(8)) {
                        Some(next) => {
                            value = value * 8 + next;
                            chars.next();
                        }
                        None => break,
                    }
                }
                bytes.push((value & 0xff) as u8);
            }
            Some('t') => bytes.push(b'\t'),
            Some('n') => bytes.push(b'\n'),
            Some('r') => bytes.push(b'\r'),
            Some('a') => bytes.push(0x07),
            Some('b') => bytes.push(0x08),
            Some('f') => bytes.push(0x0c),
            Some('v') => bytes.push(0x0b),
            Some(other) => {
                let mut buffer = [0; 4];
                bytes.extend_from_slice(other.encode_utf8(&mut buffer).as_bytes());
            }
            None => {}
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

/// `@@ -12,3 +14,5 @@` to (12, 3, 14, 5). A side without a count has one line.
fn hunk_header(header: &str) -> Option<(u32, u32, u32, u32)> {
    let mut parts = header.split_whitespace().skip(1);
    let side = |part: Option<&str>, sign: char| -> Option<(u32, u32)> {
        let mut numbers = part?.strip_prefix(sign)?.split(',');
        let start = numbers.next()?.parse().ok()?;
        let count = match numbers.next() {
            Some(count) => count.parse().ok()?,
            None => 1,
        };
        Some((start, count))
    };
    let (old_start, old_count) = side(parts.next(), '-')?;
    let (new_start, new_count) = side(parts.next(), '+')?;
    Some((old_start, old_count, new_start, new_count))
}

/// A title for comparison: lowercase words, punctuation dropped.
pub fn normalized(title: &str) -> String {
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

pub(crate) fn severity_rank(severity: ReviewSeverity) -> u8 {
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

/// `raw` as one of [`crate::taste::signals::CATEGORIES`], if it is one.
fn known_category(raw: &str) -> Option<&'static str> {
    let raw = raw.trim();
    crate::taste::signals::CATEGORIES.iter().copied().find(|known| known.eq_ignore_ascii_case(raw))
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
    /// Earlier findings the reviewer says the new commits fixed, as given.
    pub fixed: Vec<String>,
}

/// Check a parsed review against the files that changed and the diff's
/// hunks. A finding on a changed file whose lines all sit in one hunk on its
/// side is inline; anything else is kept, marked `outside_diff`. A one-line
/// finding on a block's closing line becomes a range over the block.
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
        let start_line = start_line.or_else(|| hunk.and_then(|hunk| block_start(hunk, raw.side, raw.line)));
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
            category: raw.category.as_deref().and_then(known_category).map(str::to_owned),
            rule_ids: raw.rule_ids,
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
    Validated { summary: review.summary.trim().to_owned(), findings: kept, duplicates, over_limit, fixed: review.fixed }
}

#[cfg(test)]
#[path = "contract_tests.rs"]
mod tests;
