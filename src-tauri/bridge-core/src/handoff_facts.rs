//! The part of a handoff Bridge writes itself, from stored history, with no
//! model involved.
//!
//! A cold start used to be headed by the outgoing model's summary, when there
//! was one. Most switches have none: the old process has been reaped, or the
//! provider is out of quota and cannot answer a summary prompt at all, which is
//! exactly when a person switches. These facts are there either way: what the
//! person last asked, the first ask when the tail no longer reaches it, which
//! files the chat wrote, and which pull requests and issues it touched. A model
//! summary, when one lands, still heads the handoff; these follow it.
//!
//! Deterministic: the same branch gives the same facts, in the same order.

use std::borrow::Cow;
use std::collections::HashSet;
use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;

use crate::model::SessionEntry;
use crate::tool_facts::{self, StartedIndex};

/// The most of the latest (or first) user message a facts line keeps.
pub const GOAL_BYTES: usize = 400;
/// The most files a handoff names.
pub const MAX_FILES: usize = 15;
/// The most pull request and issue links a handoff names.
pub const MAX_LINKS: usize = 8;
/// The most the whole facts block may take.
pub const MAX_FACTS_BYTES: usize = 4096;
/// The most text a link scan reads across the branch, newest first. A chat of
/// a hundred thousand command outputs is not read end to end at a switch.
const LINK_SCAN_BYTES: usize = 2 * 1024 * 1024;
/// The most of one entry's text a link scan reads, from each end.
const LINK_SCAN_ENTRY_BYTES: usize = 32 * 1024;
/// The most of one path a facts line keeps.
const PATH_BYTES: usize = 200;

/// GitHub pull requests and issues, GitLab merge requests and issues. The
/// match ends at the number, so a query string or fragment is never part of it.
static LINK: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(
        r"https://(?:github\.com/[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+/(?:pull|issues)/[0-9]+|gitlab\.com/[A-Za-z0-9_.-]+(?:/[A-Za-z0-9_.-]+)+/-/(?:merge_requests|issues)/[0-9]+)",
    )
    .ok()
});

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HandoffFacts {
    /// The latest user message, clipped.
    pub goal: Option<String>,
    /// The first user message and its sequence, when it is not the latest.
    pub first_ask: Option<(i64, String)>,
    /// Files the chat wrote, newest first.
    pub files: Vec<String>,
    /// Files past [`MAX_FILES`].
    pub more_files: usize,
    /// Pull request and issue links, newest first.
    pub links: Vec<String>,
}

impl HandoffFacts {
    /// Files named or counted.
    pub fn file_count(&self) -> usize {
        self.files.len() + self.more_files
    }

    /// The facts as handoff lines. `first_ask_carried` drops the first ask when
    /// the tail already holds it; `earlier` is the pointer line, when the tail
    /// left messages out and the chat can read them.
    pub fn lines(&self, first_ask_carried: bool, earlier: Option<String>) -> Vec<String> {
        let mut lines = Vec::new();
        if let Some(goal) = &self.goal {
            lines.push(format!("goal: {goal}"));
        }
        if let Some((_, first)) = self.first_ask.as_ref().filter(|_| !first_ask_carried) {
            lines.push(format!("started with: {first}"));
        }
        if !self.files.is_empty() {
            let more = if self.more_files > 0 { format!(" (+{} more)", self.more_files) } else { String::new() };
            lines.push(format!("files touched: {}{more}", self.files.join(", ")));
        }
        if !self.links.is_empty() {
            lines.push(format!("links: {}", self.links.join(", ")));
        }
        lines.extend(earlier);
        lines
    }
}

/// The pointer a handoff gives a chat that can read what it left out.
pub fn earlier_line(older_messages: usize, before: i64) -> String {
    format!(
        "earlier: {older_messages} older message{} not in this handoff; call {} with before={before} to read them, tool output included.",
        if older_messages == 1 { " is" } else { "s are" },
        crate::history_tools::HISTORY,
    )
}

/// The facts of a branch, oldest entry first.
pub fn collect(branch: &[SessionEntry]) -> HandoffFacts {
    let users = branch
        .iter()
        .filter(|entry| entry.kind == "user.message")
        .filter_map(|entry| crate::restoration::entry_text(entry).map(|text| (entry, text)))
        .collect::<Vec<_>>();
    let goal = users.last().map(|(_, text)| one_line(text, GOAL_BYTES));
    let first_ask = (users.len() > 1)
        .then(|| users.first())
        .flatten()
        .map(|(entry, text)| (entry.sequence, one_line(text, GOAL_BYTES)));

    let started = StartedIndex::new(branch.iter());
    let mut files = Vec::new();
    let mut seen = HashSet::new();
    let mut add = |path: &str| {
        let path = path.trim();
        if !path.is_empty() && seen.insert(path.to_owned()) {
            files.push(path.to_owned());
        }
    };
    // The newest checkpoint's list stands for everything up to it, so the
    // walk stops there after taking it.
    for entry in branch.iter().rev() {
        match entry.kind.as_str() {
            "worker.result" => strings(&entry.payload, "filesChanged").for_each(&mut add),
            "artifact.created" => {
                if let Some(path) = entry.payload.get("path").and_then(Value::as_str) {
                    add(path);
                }
            }
            "checkpoint" | "compaction" => {
                strings(&entry.payload, "filesTouched").for_each(&mut add);
                break;
            }
            kind if tool_facts::is_completed_tool(kind) => {
                if let Some(fact) = tool_facts::fact(entry, &started) {
                    fact.written_paths().iter().map(String::as_str).for_each(&mut add);
                }
            }
            _ => {}
        }
    }
    let more_files = files.len().saturating_sub(MAX_FILES);
    files.truncate(MAX_FILES);
    let files = files.iter().map(|path| one_line(path, PATH_BYTES)).collect();

    HandoffFacts { goal, first_ask, files, more_files, links: links(branch) }
}

/// The newest [`MAX_LINKS`] distinct pull request and issue links, reading
/// messages, worker results and tool calls newest first, within
/// [`LINK_SCAN_BYTES`].
fn links(branch: &[SessionEntry]) -> Vec<String> {
    let Some(pattern) = LINK.as_ref() else {
        return Vec::new();
    };
    let mut found = Vec::new();
    let mut seen = HashSet::new();
    let mut budget = LINK_SCAN_BYTES;
    for entry in branch.iter().rev() {
        let texts: Vec<Cow<'_, str>> = if let Some(text) = crate::restoration::entry_text(entry) {
            vec![Cow::Borrowed(text)]
        } else if tool_facts::is_completed_tool(&entry.kind) || tool_facts::is_started_tool(&entry.kind) {
            ["/data/command", "/data/input/command", "/data/state/input/command", "/title"]
                .iter()
                .filter_map(|pointer| entry.payload.pointer(pointer).and_then(Value::as_str))
                .map(Cow::Borrowed)
                .chain(tool_facts::output(&entry.payload).map(Cow::Owned))
                .collect()
        } else {
            continue;
        };
        // Newest first within the entry too, so the cut keeps the newest links.
        let mut in_entry = Vec::new();
        for text in texts {
            for window in scan_windows(&text) {
                let cost = window.len().min(budget);
                budget -= cost;
                for link in pattern.find_iter(&window[..floor_boundary(window, cost)]) {
                    in_entry.push(link.as_str().to_owned());
                }
                if budget == 0 {
                    break;
                }
            }
        }
        for link in in_entry.into_iter().rev() {
            if seen.insert(link.clone()) {
                found.push(link);
                if found.len() == MAX_LINKS {
                    return found;
                }
            }
        }
        if budget == 0 {
            break;
        }
    }
    found
}

/// The parts of `text` a link scan reads: all of it when short, else its
/// first and last [`LINK_SCAN_ENTRY_BYTES`].
fn scan_windows(text: &str) -> Vec<&str> {
    if text.len() <= 2 * LINK_SCAN_ENTRY_BYTES {
        return vec![text];
    }
    let head = floor_boundary(text, LINK_SCAN_ENTRY_BYTES);
    let mut tail = text.len() - LINK_SCAN_ENTRY_BYTES;
    while !text.is_char_boundary(tail) {
        tail += 1;
    }
    vec![&text[..head], &text[tail..]]
}

fn floor_boundary(text: &str, at: usize) -> usize {
    let mut at = at.min(text.len());
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

fn strings<'a>(payload: &'a Value, field: &str) -> impl Iterator<Item = &'a str> {
    payload
        .get(field)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
}

/// `text` on one line (runs of whitespace become one space), scrubbed, and
/// clipped to `max_bytes` on a char boundary.
fn one_line(text: &str, max_bytes: usize) -> String {
    let window = &text[..floor_boundary(text, max_bytes.saturating_mul(4).max(4096))];
    let flat = window.split_whitespace().collect::<Vec<_>>().join(" ");
    let scrubbed = crate::history_tools::scrub(&flat);
    if scrubbed.len() <= max_bytes {
        return scrubbed;
    }
    let end = floor_boundary(&scrubbed, max_bytes - '…'.len_utf8());
    format!("{}…", &scrubbed[..end])
}

/// Bound a facts block to `max_bytes`: whole lines in order, the line that
/// overflows dropped with everything after it. The goal line is first, so it
/// is the last to go.
pub fn bound(lines: Vec<String>, max_bytes: usize) -> Vec<String> {
    let mut used = 0usize;
    lines
        .into_iter()
        .take_while(|line| {
            used += line.len() + 1;
            used <= max_bytes
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn entry(sequence: i64, kind: &str, payload: Value) -> SessionEntry {
        SessionEntry {
            id: format!("e{sequence}"),
            session_id: "s".into(),
            parent_entry_id: None,
            sequence,
            semantic_schema_version: crate::model::SEMANTIC_EVENT_SCHEMA_VERSION,
            kind: kind.into(),
            payload,
            provider_event_id: None,
            context_visibility: "eligible".into(),
            token_estimate: None,
            created_at: "now".into(),
        }
    }

    #[test]
    fn links_and_files_are_newest_first_deduplicated_and_capped() {
        let mut branch = vec![
            entry(1, "user.message", json!({"text": "fix https://github.com/acme/app/issues/1 please"})),
            entry(2, "checkpoint", json!({"schemaVersion": 1, "summary": "s", "filesTouched": ["old/before-checkpoint.rs"]})),
        ];
        for index in 0..20 {
            branch.push(entry(10 + index, "file_change.completed", json!({"itemId": format!("x{index}"), "data": {"changes": [{"path": format!("src/f{index:02}.rs")}]}})));
        }
        branch.push(entry(40, "worker.result", json!({"status": "done", "summary": "opened https://github.com/acme/app/pull/12?tab=files and https://gitlab.com/g/sub/proj/-/merge_requests/3#note_1", "filesChanged": ["src/f19.rs", "docs/worker.md"]})));
        branch.push(entry(41, "command.completed", json!({"itemId": "c", "text": "https://github.com/acme/app/pull/13\nhttps://github.com/acme/app/pull/12", "data": {"command": "gh pr create", "exitCode": 0}})));
        for index in 0..10 {
            branch.push(entry(50 + index, "assistant.message", json!({"text": format!("see https://github.com/acme/other/issues/{index}")})));
        }
        let facts = collect(&branch);
        assert_eq!(facts.files[..3], ["src/f19.rs", "docs/worker.md", "src/f18.rs"], "newest entry first, its own order kept, deduplicated");
        assert_eq!(facts.files.len(), MAX_FILES);
        assert_eq!(facts.file_count(), 22, "20 edits + worker doc + the checkpoint's file");
        assert_eq!(facts.links.len(), MAX_LINKS);
        assert_eq!(facts.links[0], "https://github.com/acme/other/issues/9");
        assert!(!facts.links.iter().any(|link| link.contains('?') || link.contains('#')), "{:?}", facts.links);

        let fewer = collect(&branch[..branch.len() - 10]);
        assert_eq!(
            fewer.links,
            vec![
                "https://github.com/acme/app/pull/12",
                "https://github.com/acme/app/pull/13",
                "https://gitlab.com/g/sub/proj/-/merge_requests/3",
                "https://github.com/acme/app/issues/1",
            ],
            "newest entry first, newest link in an entry first, each once"
        );
        let line = fewer.lines(false, None).join("\n");
        assert!(line.contains("files touched: src/f19.rs, docs/worker.md, src/f18.rs"), "{line}");
        assert!(line.contains(" (+7 more)"), "{line}");
    }

    #[test]
    fn the_goal_is_the_latest_ask_and_the_first_ask_is_kept_apart() {
        let branch = vec![
            entry(1, "user.message", json!({"text": "build the importer\n\nfor   Conductor"})),
            entry(2, "assistant.message", json!({"text": "done"})),
            entry(3, "user.message", json!({"text": format!("now add tests {}", "ü".repeat(500))})),
        ];
        let facts = collect(&branch);
        assert!(facts.goal.as_deref().unwrap().starts_with("now add tests"));
        assert!(facts.goal.as_deref().unwrap().len() <= GOAL_BYTES);
        assert_eq!(facts.first_ask, Some((1, "build the importer for Conductor".to_owned())));
        assert!(facts.lines(false, None).iter().any(|line| line == "started with: build the importer for Conductor"));
        assert!(!facts.lines(true, None).iter().any(|line| line.starts_with("started with")));
        let one = collect(&branch[..2]);
        assert!(one.first_ask.is_none(), "one ask is only a goal");
        assert!(collect(&[]).lines(false, None).is_empty());
    }

    #[test]
    fn the_earlier_line_names_the_tool_and_the_sequence() {
        assert_eq!(
            earlier_line(107, 1203),
            "earlier: 107 older messages are not in this handoff; call chats_history with before=1203 to read them, tool output included."
        );
        assert!(earlier_line(1, 5).starts_with("earlier: 1 older message is not"));
    }

    #[test]
    fn a_token_in_a_goal_is_scrubbed_and_the_block_is_bounded() {
        let token = format!("sk-ant-api03-{}", "Qx7".repeat(30));
        let facts = collect(&[entry(1, "user.message", json!({"text": format!("use {token} for the call")}))]);
        assert!(!facts.goal.as_deref().unwrap().contains(&token[13..30]), "{:?}", facts.goal);
        let lines = vec!["a".repeat(3000), "b".repeat(2000), "c".into()];
        assert_eq!(bound(lines, MAX_FACTS_BYTES), vec!["a".repeat(3000)]);
    }

    #[test]
    fn a_huge_output_is_scanned_at_both_ends_only() {
        let middle = format!("{} https://github.com/acme/app/pull/77 {}", "x".repeat(100_000), "y".repeat(100_000));
        let ends = format!("https://github.com/acme/app/pull/1 {middle} https://github.com/acme/app/pull/2");
        let facts = collect(&[entry(1, "command.completed", json!({"itemId": "c", "text": ends, "data": {"command": "gh pr list"}}))]);
        assert_eq!(facts.links, vec!["https://github.com/acme/app/pull/2", "https://github.com/acme/app/pull/1"]);
    }
}
