//! Bridge Markdown files: YAML frontmatter, then Your words.
//!
//! Phase 1 needs only the frontmatter: the body is stored and mirrored as the
//! exact text a person or agent wrote. Values are written as JSON scalars and
//! arrays, which are valid YAML, so any editor or YAML parser reads them and
//! this module needs no YAML dependency. Keys Bridge does not know are kept,
//! in order, and written back unchanged.

use serde_json::Value;

/// Keys Bridge writes and reads, in the order it writes them.
pub const KNOWN: [&str; 9] = ["id", "title", "kind", "tags", "project", "state", "parent", "created", "updated"];

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Frontmatter {
    pub id: Option<String>,
    pub title: Option<String>,
    pub kind: Option<String>,
    pub tags: Vec<String>,
    pub project: Option<String>,
    pub state: Option<String>,
    pub parent: Option<String>,
    pub created: Option<String>,
    pub updated: Option<String>,
    /// Lines for keys Bridge does not know, verbatim.
    pub unknown: Vec<String>,
}

fn scalar(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "\"\"".into())
}

fn read_scalar(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() || raw == "null" || raw == "~" {
        return None;
    }
    if raw.starts_with('"') {
        return serde_json::from_str::<String>(raw).ok();
    }
    if raw.starts_with('\'') && raw.ends_with('\'') && raw.len() >= 2 {
        return Some(raw[1..raw.len() - 1].replace("''", "'"));
    }
    Some(raw.to_owned())
}

fn read_list(raw: &str) -> Vec<String> {
    let raw = raw.trim();
    if let Ok(Value::Array(items)) = serde_json::from_str::<Value>(raw) {
        return items.into_iter().filter_map(|item| item.as_str().map(str::to_owned)).collect();
    }
    // `[a, b]` without quotes, as people type it.
    raw.trim_start_matches('[')
        .trim_end_matches(']')
        .split(',')
        .filter_map(read_scalar)
        .collect()
}

/// Group header lines into top-level entries: a key line plus every indented
/// or list line under it, so nested YAML never reads as a page field.
fn top_level_blocks(header: &str) -> Vec<Vec<&str>> {
    let mut blocks: Vec<Vec<&str>> = Vec::new();
    for line in header.lines() {
        let continuation = line.starts_with(' ') || line.starts_with('\t') || line.starts_with("- ") || line == "-" || line.trim().is_empty();
        match blocks.last_mut() {
            Some(block) if continuation => block.push(line),
            _ if line.trim().is_empty() => {}
            _ => blocks.push(vec![line]),
        }
    }
    for block in &mut blocks {
        while block.len() > 1 && block.last().is_some_and(|line| line.trim().is_empty()) {
            block.pop();
        }
    }
    blocks
}

/// Split a file into its frontmatter and body. A file with no frontmatter is
/// all body.
pub fn parse(text: &str) -> (Frontmatter, String) {
    let mut front = Frontmatter::default();
    // Either line ending: editors on Windows, and some sync tools, save CRLF.
    let opening = if text.starts_with("---\r\n") {
        5
    } else if text.starts_with("---\n") {
        4
    } else {
        return (front, text.to_owned());
    };
    let mut position = opening;
    let mut close = None;
    for line in text[opening..].split_inclusive('\n') {
        if line.trim_end_matches('\n').trim_end_matches('\r') == "---" {
            close = Some((position, position + line.len()));
            break;
        }
        position += line.len();
    }
    let Some((header_end, body_start)) = close else {
        return (front, text.to_owned());
    };
    let header = &text[opening..header_end];
    for block in top_level_blocks(header) {
        let first = block[0];
        let continued = block.len() > 1;
        let Some((key, value)) = first.split_once(':').filter(|_| !first.starts_with('#')) else {
            front.unknown.push(block.join("\n"));
            continue;
        };
        let list_items = || -> Vec<String> {
            block[1..].iter().filter_map(|line| line.trim_start().strip_prefix('-')).filter_map(read_scalar).collect()
        };
        match (key.trim(), continued) {
            ("tags", true) if value.trim().is_empty() => front.tags = list_items(),
            ("id", false) => front.id = read_scalar(value),
            ("title", false) => front.title = read_scalar(value),
            ("kind", false) => front.kind = read_scalar(value),
            ("tags", false) => front.tags = read_list(value),
            ("project", false) => front.project = read_scalar(value),
            ("state", false) => front.state = read_scalar(value),
            ("parent", false) => front.parent = read_scalar(value),
            ("created", false) => front.created = read_scalar(value),
            ("updated", false) => front.updated = read_scalar(value),
            // Anything else, including a known key written in a form this
            // reader does not interpret, is kept verbatim rather than guessed at.
            _ => front.unknown.push(block.join("\n")),
        }
    }
    (front, text[body_start..].to_owned())
}

/// Render frontmatter and body back into a file.
pub fn render(front: &Frontmatter, body: &str) -> String {
    let mut out = String::from("---\n");
    let mut line = |key: &str, value: String| {
        out.push_str(key);
        out.push_str(": ");
        out.push_str(&value);
        out.push('\n');
    };
    let optional = |value: &Option<String>| value.as_deref().map(scalar).unwrap_or_else(|| "null".into());
    line("id", optional(&front.id));
    line("title", optional(&front.title));
    line("kind", optional(&front.kind));
    line("tags", serde_json::to_string(&front.tags).unwrap_or_else(|_| "[]".into()));
    line("project", optional(&front.project));
    line("state", optional(&front.state));
    line("parent", optional(&front.parent));
    line("created", optional(&front.created));
    line("updated", optional(&front.updated));
    for unknown in &front.unknown {
        out.push_str(unknown);
        out.push('\n');
    }
    out.push_str("---\n");
    out.push_str(body);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frontmatter_round_trips_and_keeps_unknown_keys() {
        let front = Frontmatter {
            id: Some("0190-abc".into()),
            title: Some("Launch: \"v1\" plan".into()),
            kind: Some("idea".into()),
            tags: vec!["launch".into(), "q4".into()],
            project: None,
            state: Some("open".into()),
            parent: None,
            created: Some("2026-10-30T10:00:00.000Z".into()),
            updated: Some("2026-10-30T11:00:00.000Z".into()),
            unknown: vec![],
        };
        let body = "Your words\n\n- with a typo: teh\n---\nnot frontmatter\n";
        let file = render(&front, body);
        assert_eq!(parse(&file), (front.clone(), body.to_owned()));

        // An editor added its own keys and typed tags by hand.
        let edited = file
            .replace("tags: [\"launch\",\"q4\"]\n", "tags: [launch, ops]\naliases: [\"lp\"]\n")
            .replace("---\nYour words", "obsidian-cssclass: wide\n---\nYour words");
        let (parsed, parsed_body) = parse(&edited);
        assert_eq!(parsed.tags, vec!["launch", "ops"]);
        assert_eq!(parsed.unknown, vec!["aliases: [\"lp\"]", "obsidian-cssclass: wide"]);
        assert_eq!(parsed_body, body);
        let rewritten = render(&parsed, &parsed_body);
        assert!(rewritten.contains("aliases: [\"lp\"]\nobsidian-cssclass: wide\n---\n"));
        assert_eq!(parse(&rewritten), (parsed, parsed_body));
    }

    #[test]
    fn crlf_frontmatter_is_recognized_and_the_body_kept_exactly() {
        let front = Frontmatter { id: Some("x".into()), title: Some("Original".into()), tags: vec!["a".into()], ..Default::default() };
        let crlf = render(&front, "line one\nline two\n").replace('\n', "\r\n");
        let (parsed, body) = parse(&crlf);
        assert_eq!(parsed.title.as_deref(), Some("Original"));
        assert_eq!(parsed.tags, vec!["a"]);
        assert!(parsed.unknown.is_empty(), "{:?}", parsed.unknown);
        assert_eq!(body, "line one\r\nline two\r\n");
        // No body at all, closing delimiter at the very end.
        let (parsed, body) = parse("---\r\ntitle: \"T\"\r\n---");
        assert_eq!((parsed.title.as_deref(), body.as_str()), (Some("T"), ""));
    }

    #[test]
    fn nested_and_block_yaml_stays_with_its_key() {
        let file = "---\nid: \"x\"\ntitle: \"Original\"\ncustom:\n  title: Nested\n  list:\n    - a\naliases:\n- lp\n- launch\ntags:\n  - one\n  - \"two\"\n---\nbody\n";
        let (parsed, body) = parse(file);
        assert_eq!(parsed.title.as_deref(), Some("Original"), "a nested title is not the page title");
        assert_eq!(parsed.tags, vec!["one", "two"], "block-style lists are read");
        assert_eq!(parsed.unknown, vec!["custom:\n  title: Nested\n  list:\n    - a", "aliases:\n- lp\n- launch"]);
        assert_eq!(body, "body\n");
        let rendered = render(&parsed, &body);
        assert!(rendered.contains("custom:\n  title: Nested\n  list:\n    - a\naliases:\n- lp\n- launch\n---\n"), "{rendered}");
        assert_eq!(parse(&rendered), (parsed, body));
    }

    #[test]
    fn a_file_without_frontmatter_is_all_body() {
        let (front, body) = parse("just words\n");
        assert_eq!(front, Frontmatter::default());
        assert_eq!(body, "just words\n");
    }
}
