//! What a chat's tools did, in a few words each, for a model that did not see
//! it happen.
//!
//! A handoff used to carry chat text only, so a model taking over a chat knew
//! what was said but not that `cargo test` had just failed or which files the
//! last turn edited. A fact is the tool and its target (the command's first
//! line, the path, the query) plus how it ended. It never carries tool output:
//! output is what makes a transcript big, and a chat that needs it reads it on
//! demand through `chats_history`.
//!
//! Every adapter writes its own shape. Codex puts the command and exit code on
//! the completed item, OpenCode puts them under `data.state`, and Claude's
//! completed entry is only the tool result, so its command or path is read from
//! the started entry with the same `itemId`.

use std::collections::HashMap;

use serde_json::Value;

use crate::model::SessionEntry;

/// The most a `tools:` line may take, so a turn of forty tool calls costs the
/// handoff a line, not a page.
pub const MAX_LINE_BYTES: usize = 480;
/// The most facts one `tools:` line names before it says how many it left out.
pub const MAX_FACTS_PER_LINE: usize = 6;
/// The most of one target (command line, path, query) a fact keeps.
const MAX_TARGET_BYTES: usize = 160;
/// The most of a raw target a fact scrubs before clipping it.
const SCRUB_WINDOW_BYTES: usize = 4096;
/// The most paths one file-change fact names.
const MAX_PATHS_PER_FACT: usize = 3;
/// Input fields that name what a tool acted on, most specific first.
const TARGET_FIELDS: [&str; 8] = [
    "file_path",
    "filePath",
    "notebook_path",
    "path",
    "url",
    "query",
    "pattern",
    "command",
];

/// Whether `kind` is a completed tool entry, the only kind that makes a fact.
pub fn is_completed_tool(kind: &str) -> bool {
    matches!(kind, "command.completed" | "file_change.completed" | "tool.completed")
}

/// Whether `kind` is a started tool entry: it names what a Claude tool result
/// ran, and makes no fact of its own.
pub fn is_started_tool(kind: &str) -> bool {
    matches!(kind, "command.started" | "file_change.started" | "tool.started")
}

/// One tool call, in raw words: nothing is scrubbed or clipped until
/// [`ToolFact::render`], so a branch of thousands of calls costs a scrub only
/// for the few a handoff actually shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolFact {
    /// A shell command: its first line and how it ended.
    Ran { command: String, status: Option<String> },
    /// Files a tool wrote.
    Edited { paths: Vec<String>, more: usize, failed: bool },
    /// Any other tool, by name, with what it acted on.
    Called { name: String, target: Option<String>, failed: bool },
}

impl ToolFact {
    /// The fact as a handoff shows it: every part scrubbed, then clipped.
    pub fn render(&self) -> String {
        let failed = |failed: bool| if failed { " (failed)" } else { "" };
        match self {
            Self::Ran { command, status: Some(status) } => format!("ran `{}` ({status})", clip(command)),
            Self::Ran { command, status: None } => format!("ran `{}`", clip(command)),
            Self::Edited { paths, more, failed: did_fail } => {
                let more = if *more > 0 { format!(" (+{more})") } else { String::new() };
                let paths = paths.iter().map(|path| clip(path)).collect::<Vec<_>>();
                format!("edited {}{more}{}", paths.join(", "), failed(*did_fail))
            }
            Self::Called { name, target: Some(target), failed: did_fail } => {
                format!("{} {}{}", clip(name), clip(target), failed(*did_fail))
            }
            Self::Called { name, target: None, failed: did_fail } => format!("{}{}", clip(name), failed(*did_fail)),
        }
    }

    /// The paths this fact says were written, raw.
    pub fn written_paths(&self) -> &[String] {
        match self {
            Self::Edited { paths, .. } => paths,
            _ => &[],
        }
    }
}

/// Started tool entries by `itemId`, so a completed entry that carries only
/// its result can name what it ran. A later started entry with input replaces
/// an earlier one: Claude starts a tool twice, first with empty input.
#[derive(Default)]
pub struct StartedIndex<'a> {
    by_item: HashMap<&'a str, &'a SessionEntry>,
}

impl<'a> StartedIndex<'a> {
    pub fn new<I>(entries: I) -> Self
    where
        I: IntoIterator<Item = &'a SessionEntry>,
    {
        let mut by_item = HashMap::new();
        for entry in entries {
            if !is_started_tool(&entry.kind) {
                continue;
            }
            let Some(item) = entry.payload.get("itemId").and_then(Value::as_str) else {
                continue;
            };
            if input(&entry.payload).is_some() || by_item.get(item).is_none() {
                by_item.insert(item, entry);
            }
        }
        Self { by_item }
    }

    fn get(&self, entry: &SessionEntry) -> Option<&'a SessionEntry> {
        let item = entry.payload.get("itemId").and_then(Value::as_str)?;
        self.by_item.get(item).copied()
    }
}

/// The fact a completed tool entry makes, if it makes one.
pub fn fact(entry: &SessionEntry, started: &StartedIndex<'_>) -> Option<ToolFact> {
    if !is_completed_tool(&entry.kind) {
        return None;
    }
    let completed = &entry.payload;
    let started = started.get(entry).map(|entry| &entry.payload);
    let input = input(completed).or_else(|| started.and_then(input));
    let failed = failed(completed);
    match entry.kind.as_str() {
        "command.completed" => {
            let command = string_at(completed, "/data/command")
                .or_else(|| input.and_then(|input| string_at(input, "/command")))
                .or_else(|| started.and_then(|started| string_at(started, "/data/command")))
                .or_else(|| started.and_then(|started| string_at(started, "/title")))
                .or_else(|| string_at(completed, "/title"))?;
            let command = first_line(unwrap_shell(command)).to_owned();
            if command.is_empty() {
                return None;
            }
            let status = exit_code(completed)
                .map(|code| format!("exit {code}"))
                .or_else(|| failed.then(|| "failed".to_owned()));
            Some(ToolFact::Ran { command, status })
        }
        "file_change.completed" => {
            let mut paths = Vec::new();
            for change in completed
                .pointer("/data/changes")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                if let Some(path) = string_at(change, "/path") {
                    paths.push(path);
                }
            }
            if paths.is_empty() {
                if let Some(path) = input.and_then(|input| {
                    ["file_path", "filePath", "notebook_path", "path"]
                        .iter()
                        .find_map(|field| input.get(*field).and_then(Value::as_str))
                        .map(str::trim)
                        .filter(|path| !path.is_empty())
                }) {
                    paths.push(path);
                }
            }
            if paths.is_empty() {
                return called(completed, started, input, failed);
            }
            let mut seen = Vec::<String>::new();
            for path in paths {
                let path = first_line(path).to_owned();
                if !path.is_empty() && !seen.contains(&path) {
                    seen.push(path);
                }
            }
            let more = seen.len().saturating_sub(MAX_PATHS_PER_FACT);
            seen.truncate(MAX_PATHS_PER_FACT);
            Some(ToolFact::Edited { paths: seen, more, failed })
        }
        _ => called(completed, started, input, failed),
    }
}

fn called(completed: &Value, started: Option<&Value>, input: Option<&Value>, failed: bool) -> Option<ToolFact> {
    let name = ["/data/name", "/data/tool", "/title"]
        .iter()
        .find_map(|pointer| {
            string_at(completed, pointer).or_else(|| started.and_then(|started| string_at(started, pointer)))
        })
        // A Claude tool result says only that it is one.
        .or_else(|| string_at(completed, "/data/type").filter(|kind| *kind != "tool_result"))?;
    let name = first_line(name).to_owned();
    if name.is_empty() {
        return None;
    }
    let target = input
        .and_then(|input| {
            TARGET_FIELDS
                .iter()
                .find_map(|field| input.get(*field).and_then(Value::as_str))
        })
        .or_else(|| string_at(completed, "/data/query"))
        .or_else(|| string_at(completed, "/data/command"))
        .map(|target| first_line(unwrap_shell(target)).to_owned())
        .filter(|target| !target.is_empty() && *target != name);
    Some(ToolFact::Called { name, target, failed })
}

/// What a completed tool printed or returned, wherever its adapter keeps it.
pub fn output(payload: &Value) -> Option<String> {
    let text = |value: &Value| -> Option<String> {
        match value {
            Value::String(text) => Some(text.clone()),
            Value::Array(parts) => {
                let joined = parts
                    .iter()
                    .filter_map(|part| part.get("text").and_then(Value::as_str).or_else(|| part.as_str()))
                    .collect::<Vec<_>>()
                    .join("\n");
                Some(joined)
            }
            _ => None,
        }
    };
    ["/text", "/data/aggregatedOutput", "/data/content", "/data/state/output", "/data/output", "/data/result"]
        .iter()
        .filter_map(|pointer| payload.pointer(pointer))
        .filter_map(text)
        .map(|output| output.trim().to_owned())
        .find(|output| !output.is_empty())
}

/// The tool's input object, wherever this adapter keeps it, when it holds
/// anything.
fn input(payload: &Value) -> Option<&Value> {
    ["/data/input", "/data/state/input"]
        .iter()
        .filter_map(|pointer| payload.pointer(pointer))
        .find(|value| value.as_object().is_some_and(|object| !object.is_empty()))
}

fn string_at<'v>(value: &'v Value, pointer: &str) -> Option<&'v str> {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn exit_code(payload: &Value) -> Option<i64> {
    ["/data/exitCode", "/data/exit_code", "/data/state/metadata/exit"]
        .iter()
        .find_map(|pointer| payload.pointer(pointer).and_then(Value::as_i64))
}

fn failed(payload: &Value) -> bool {
    payload.pointer("/data/is_error").and_then(Value::as_bool) == Some(true)
        || matches!(
            payload.get("status").and_then(Value::as_str),
            Some("failed" | "error" | "declined")
        )
        || matches!(
            payload.pointer("/data/state/status").and_then(Value::as_str),
            Some("error" | "failed")
        )
}

/// `cmd` from `/bin/zsh -lc 'cmd'` and its spellings: Codex wraps every
/// command in the login shell, and the wrapper says nothing.
fn unwrap_shell(command: &str) -> &str {
    let command = command.trim();
    let mut rest = command;
    for shell in ["/usr/bin/env ", "/bin/", "/usr/bin/"] {
        rest = rest.strip_prefix(shell).unwrap_or(rest);
    }
    let Some(rest) = ["zsh ", "bash ", "sh "].iter().find_map(|shell| rest.strip_prefix(shell)) else {
        return command;
    };
    let Some(rest) = ["-lc ", "-c ", "-l -c "].iter().find_map(|flag| rest.trim_start().strip_prefix(flag)) else {
        return command;
    };
    let rest = rest.trim();
    for quote in ['\'', '"'] {
        if let Some(inner) = rest.strip_prefix(quote) {
            return inner.strip_suffix(quote).unwrap_or(inner);
        }
    }
    rest
}

fn first_line(text: &str) -> &str {
    text.lines().map(str::trim).find(|line| !line.is_empty()).unwrap_or("")
}

/// Scrubbed, then cut to [`MAX_TARGET_BYTES`] on a char boundary. Scrubbing
/// comes first so a credential is never cut into a fragment no detector knows.
fn clip(text: &str) -> String {
    // Only the first [`MAX_TARGET_BYTES`] of the scrubbed text survive, and
    // scrubbing never lengthens a credential, so nothing past this cut could.
    // It bounds the scrub on a one-line megabyte of minified JSON.
    let mut cut = text.len().min(SCRUB_WINDOW_BYTES);
    while !text.is_char_boundary(cut) {
        cut -= 1;
    }
    let scrubbed = crate::history_tools::scrub(&text[..cut]);
    if scrubbed.len() <= MAX_TARGET_BYTES {
        return scrubbed;
    }
    let mut end = MAX_TARGET_BYTES - '…'.len_utf8();
    while !scrubbed.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &scrubbed[..end])
}

/// One `tools:` line for a run of facts, oldest first. It names the newest
/// facts, at most [`MAX_FACTS_PER_LINE`] in at most [`MAX_LINE_BYTES`], in the
/// order they ran, after a count of the earlier ones it left out: the last
/// calls before a reply (the test run that passed, the PR it opened) are the
/// ones that say where the work stands. `None` for no facts.
pub fn line(facts: &[ToolFact]) -> Option<String> {
    const PREFIX: &str = "tools: ";
    // Room kept for the widest "(+N earlier) " a line can need.
    const COUNT_ROOM: usize = 24;
    if facts.is_empty() {
        return None;
    }
    let room = MAX_LINE_BYTES - PREFIX.len() - COUNT_ROOM;
    let mut shown = Vec::new();
    let mut used = 0usize;
    for fact in facts.iter().rev().take(MAX_FACTS_PER_LINE) {
        let rendered = fact.render();
        let separator = if shown.is_empty() { 0 } else { "; ".len() };
        if used + separator + rendered.len() > room {
            if shown.is_empty() {
                let mut end = room - '…'.len_utf8();
                while !rendered.is_char_boundary(end) {
                    end -= 1;
                }
                shown.push(format!("{}…", &rendered[..end]));
            }
            break;
        }
        used += separator + rendered.len();
        shown.push(rendered);
    }
    shown.reverse();
    let earlier = facts.len() - shown.len();
    let count = if earlier > 0 { format!("(+{earlier} earlier) ") } else { String::new() };
    Some(format!("{PREFIX}{count}{}", shown.join("; ")))
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

    fn facts_of(entries: &[SessionEntry]) -> Vec<ToolFact> {
        let index = StartedIndex::new(entries.iter());
        entries.iter().filter_map(|entry| fact(entry, &index)).collect()
    }

    #[test]
    fn a_codex_command_fact_unwraps_the_shell_and_keeps_the_exit_code() {
        let entries = [entry(1, "command.completed", json!({
            "itemId": "exec-1", "status": "completed", "title": "/bin/zsh -lc 'cargo test -p bridge-core'",
            "data": {"type": "commandExecution", "command": "/bin/zsh -lc 'cargo test -p bridge-core'", "exitCode": 101,
                     "aggregatedOutput": "test result: FAILED. 3 passed; 1 failed"}
        }))];
        assert_eq!(
            facts_of(&entries),
            vec![ToolFact::Ran { command: "cargo test -p bridge-core".into(), status: Some("exit 101".into()) }]
        );
        let heredoc = [entry(1, "command.completed", json!({
            "itemId": "exec-2", "data": {"command": "/bin/zsh -lc \"python3 - <<'PY'\nprint(1)\nPY\"", "exitCode": 0}
        }))];
        assert_eq!(facts_of(&heredoc)[0].render(), "ran `python3 - <<'PY'` (exit 0)");
    }

    #[test]
    fn a_claude_completed_entry_reads_its_command_from_the_started_entry() {
        let entries = [
            entry(1, "command.started", json!({"itemId": "toolu_1", "title": "Bash", "data": {"name": "Bash", "input": {}, "phase": "preparing"}})),
            entry(2, "command.started", json!({"itemId": "toolu_1", "title": "gh pr create --fill", "data": {"name": "Bash", "input": {"command": "gh pr create --fill"}, "phase": "running"}})),
            entry(3, "command.completed", json!({"itemId": "toolu_1", "status": "completed", "title": null, "text": "https://github.com/acme/app/pull/12", "data": {"type": "tool_result", "is_error": false}})),
            entry(4, "command.started", json!({"itemId": "toolu_2", "data": {"name": "Bash", "input": {"command": "false"}}})),
            entry(5, "command.completed", json!({"itemId": "toolu_2", "data": {"type": "tool_result", "is_error": true}})),
        ];
        assert_eq!(
            facts_of(&entries),
            vec![
                ToolFact::Ran { command: "gh pr create --fill".into(), status: None },
                ToolFact::Ran { command: "false".into(), status: Some("failed".into()) },
            ]
        );
    }

    #[test]
    fn an_opencode_edit_and_a_claude_write_name_their_paths() {
        let entries = [
            entry(1, "file_change.completed", json!({"itemId": "call_1", "title": "edit", "data": {"tool": "edit", "state": {"status": "completed", "input": {"filePath": "/w/src/app.ts", "oldString": "a", "newString": "b"}}}})),
            entry(2, "file_change.started", json!({"itemId": "toolu_9", "data": {"name": "Write", "input": {"file_path": "/w/notes.md", "content": "big"}}})),
            entry(3, "file_change.completed", json!({"itemId": "toolu_9", "text": "File created successfully at: /w/notes.md", "data": {"type": "tool_result"}})),
            entry(4, "file_change.completed", json!({"itemId": "exec-3", "data": {"type": "fileChange", "changes": [
                {"path": "/w/a.rs"}, {"path": "/w/b.rs"}, {"path": "/w/a.rs"}, {"path": "/w/c.rs"}, {"path": "/w/d.rs"}
            ]}})),
            entry(5, "tool.completed", json!({"itemId": "call_2", "title": "read", "data": {"tool": "read", "state": {"status": "completed", "input": {"filePath": "/w/README.md"}, "output": "secret contents"}}})),
        ];
        let rendered = facts_of(&entries).iter().map(ToolFact::render).collect::<Vec<_>>();
        assert_eq!(
            rendered,
            vec![
                "edited /w/src/app.ts",
                "edited /w/notes.md",
                "edited /w/a.rs, /w/b.rs, /w/c.rs (+1)",
                "read /w/README.md",
            ]
        );
        assert_eq!(facts_of(&entries)[0].written_paths(), ["/w/src/app.ts".to_owned()]);
    }

    #[test]
    fn started_entries_and_output_never_make_a_fact() {
        let entries = [
            entry(1, "command.started", json!({"itemId": "x", "data": {"command": "ls"}})),
            entry(2, "command.output_delta", json!({"itemId": "x", "text": "PRIVATE OUTPUT"})),
            entry(3, "reasoning", json!({"text": "thinking"})),
            entry(4, "command.completed", json!({"itemId": "x", "text": "PRIVATE OUTPUT", "data": {"command": "ls", "aggregatedOutput": "PRIVATE OUTPUT", "exitCode": 0}})),
            entry(5, "tool.completed", json!({"itemId": "y", "title": "WebFetch", "data": {"name": "WebFetch", "input": {"url": "https://example.com/docs"}, "content": "PRIVATE OUTPUT"}})),
        ];
        let facts = facts_of(&entries);
        assert_eq!(facts.len(), 2);
        let rendered = line(&facts).unwrap();
        assert!(!rendered.contains("PRIVATE"), "{rendered}");
        assert_eq!(rendered, "tools: ran `ls` (exit 0); WebFetch https://example.com/docs");
    }

    #[test]
    fn a_token_in_a_command_is_scrubbed() {
        let token = format!("ghp_{}", "a1B2c3D4e5F6g7H8i9J0".repeat(2));
        let entries = [entry(1, "command.completed", json!({
            "itemId": "e", "data": {"command": format!("GH_TOKEN={token} gh pr list"), "exitCode": 0}
        }))];
        let rendered = facts_of(&entries)[0].render();
        assert!(!rendered.contains(&token[4..20]), "{rendered}");
    }

    #[test]
    fn a_run_of_tools_merges_into_one_bounded_line_that_names_the_newest() {
        let facts = (0..40)
            .map(|index| ToolFact::Ran { command: format!("step-{index:02} {}", "x".repeat(index * 7)), status: Some("exit 0".into()) })
            .collect::<Vec<_>>();
        let rendered = line(&facts).unwrap();
        assert!(rendered.len() <= MAX_LINE_BYTES, "{} bytes", rendered.len());
        assert!(rendered.contains("step-39"), "the newest call is named: {rendered}");
        assert!(!rendered.contains("step-00"), "{rendered}");
        let shown = rendered.matches("ran `").count();
        assert!((1..=MAX_FACTS_PER_LINE).contains(&shown), "{rendered}");
        assert!(rendered.starts_with(&format!("tools: (+{} earlier) ran `", 40 - shown)), "{rendered}");

        let six = (0..9)
            .map(|index| ToolFact::Called { name: format!("T{index}"), target: None, failed: false })
            .collect::<Vec<_>>();
        assert_eq!(line(&six).unwrap(), "tools: (+3 earlier) T3; T4; T5; T6; T7; T8", "in the order they ran");

        let wide = vec![ToolFact::Called { name: "Read".into(), target: Some("界".repeat(400)), failed: false }];
        let rendered = line(&wide).unwrap();
        assert!(rendered.len() <= MAX_LINE_BYTES, "{}", rendered.len());
        assert!(rendered.ends_with('…'), "{rendered}");
        let huge = vec![ToolFact::Ran { command: "y".repeat(2_000_000), status: None }];
        assert!(line(&huge).unwrap().len() <= MAX_LINE_BYTES);
        assert!(line(&[]).is_none());
        assert!(!line(&facts[..2]).unwrap().contains("earlier"), "nothing left out says nothing");
    }

    #[test]
    fn a_claude_tool_result_with_no_started_entry_is_not_named_tool_result() {
        let entries = [entry(1, "tool.completed", json!({"itemId": "toolu_x", "title": null, "data": {"type": "tool_result", "content": "x"}}))];
        assert!(facts_of(&entries).is_empty());
    }

    #[test]
    fn an_unwrapped_shell_keeps_commands_that_are_not_wrapped() {
        assert_eq!(unwrap_shell("/bin/zsh -lc 'ls -la'"), "ls -la");
        assert_eq!(unwrap_shell("bash -c \"make test\""), "make test");
        assert_eq!(unwrap_shell("/usr/bin/env bash -lc 'x'"), "x");
        assert_eq!(unwrap_shell("zshrc-check --all"), "zshrc-check --all");
        assert_eq!(unwrap_shell("shellcheck run.sh"), "shellcheck run.sh");
        assert_eq!(unwrap_shell("sh"), "sh");
    }
}
