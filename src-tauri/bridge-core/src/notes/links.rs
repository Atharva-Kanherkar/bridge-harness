//! Derived full-text and link indexes. Markdown stays authoritative and is never
//! serialized by the parser: renames replace only the exact link byte ranges.
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::sync::OnceLock;

use bridge_protocol::messages::NoteBacklink;
use regex::Regex;
use rusqlite::{params, Connection, OptionalExtension};

use crate::BridgeError;

#[derive(Debug)]
struct Link {
    range: Range<usize>,
    target: String,
    kind: &'static str,
    alias: Option<String>,
    opening: Option<String>,
}

fn pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| Regex::new(r#"\[\[([^\]\r\n]+)\]\]|(<page\s+url="bridge://p/([^"<>\s]+)"\s*>)([^\r\n]*?)</page>"#).unwrap())
}

/// Blockquote markers (`>` after up to three spaces) open each line's
/// containers: returns the quote depth and where block content starts.
fn quote_prefix(line: &str) -> (usize, usize) {
    let bytes = line.as_bytes();
    let (mut depth, mut at) = (0, 0);
    loop {
        let spaces = bytes[at..].iter().take_while(|&&b| b == b' ').count();
        if spaces > 3 || bytes.get(at + spaces) != Some(&b'>') {
            return (depth, at);
        }
        depth += 1;
        at += spaces + 1;
        if bytes.get(at) == Some(&b' ') {
            at += 1;
        }
    }
}

/// Leading indentation in columns, with tabs to the next multiple of four.
fn indent_width(content: &str) -> (usize, usize) {
    let mut width = 0;
    for (i, b) in content.bytes().enumerate() {
        match b {
            b' ' => width += 1,
            b'\t' => width += 4 - width % 4,
            _ => return (width, i),
        }
    }
    (width, content.len())
}

fn list_marker(text: &str) -> bool {
    let digits = text.bytes().take_while(u8::is_ascii_digit).count();
    let rest = match text.as_bytes().first() {
        Some(b'-' | b'*' | b'+') => &text[1..],
        _ if (1..=9).contains(&digits) && matches!(text.as_bytes().get(digits), Some(b'.' | b')')) => &text[digits + 1..],
        _ => return false,
    };
    rest.is_empty() || rest.starts_with([' ', '\t', '\r', '\n'])
}

/// Is the line starting after the newline at `at` blank? Code spans never
/// cross a blank line, the end of a paragraph.
fn blank_line_after(masked: &[u8], at: usize) -> bool {
    masked[at + 1..].iter().take_while(|&&b| b != b'\n').all(|b| b.is_ascii_whitespace())
}

/// Mask code with spaces, retaining byte offsets and newlines for replacements.
/// Fences are found inside blockquotes too, and indented code is masked where
/// it can start a block: after a blank line, outside a list.
fn outside_code(body: &str) -> String {
    let mut masked = body.as_bytes().to_vec();
    let mut fence: Option<(u8, usize, usize)> = None;
    let (mut block_start, mut in_code, mut in_list) = (true, false, false);
    let mut offset = 0;
    for line in body.split_inclusive('\n') {
        let (depth, start) = quote_prefix(line);
        let content = &line[start..];
        let (width, at) = indent_width(content);
        let trimmed = &content[at..];
        let blank = trimmed.trim().is_empty();
        let marker = trimmed.as_bytes().first().copied().unwrap_or(0);
        let count = trimmed.bytes().take_while(|&b| b == marker).count();
        let fence_line = width <= 3 && matches!(marker, b'`' | b'~') && count >= 3;
        if matches!(fence, Some((_, _, open)) if depth < open) {
            // A fence ends with the container that holds it.
            fence = None;
        }
        let code = if let Some((ch, n, _)) = fence {
            if fence_line && marker == ch && count >= n && trimmed[count..].trim().is_empty() {
                fence = None;
            }
            true
        } else if fence_line && (marker == b'~' || !trimmed[count..].contains('`')) {
            fence = Some((marker, count, depth));
            true
        } else {
            !blank && width >= 4 && (block_start || in_code) && !in_list
        };
        if code {
            for b in &mut masked[offset..offset + line.len()] {
                if *b != b'\n' {
                    *b = b' ';
                }
            }
        } else if !blank && width < 4 {
            in_list = list_marker(trimmed);
        }
        in_code = code && fence.is_none() && !fence_line;
        block_start = blank || fence_line || trimmed.starts_with('#');
        offset += line.len();
    }
    let mut i = 0;
    while i < masked.len() {
        if masked[i] != b'`' || escaped(body, i) {
            i += 1;
            continue;
        }
        let start = i;
        while i < masked.len() && masked[i] == b'`' {
            i += 1;
        }
        let n = i - start;
        let mut end = i;
        let mut close = None;
        while end < masked.len() {
            if masked[end] == b'\n' && blank_line_after(&masked, end) {
                break;
            }
            if masked[end] != b'`' {
                end += 1;
                continue;
            }
            let at = end;
            while end < masked.len() && masked[end] == b'`' {
                end += 1;
            }
            if end - at == n {
                close = Some(end);
                break;
            }
        }
        if let Some(end) = close {
            for b in &mut masked[start..end] {
                if *b != b'\n' {
                    *b = b' ';
                }
            }
            i = end;
        }
    }
    String::from_utf8(masked).expect("code masking preserves UTF-8")
}

fn escaped(body: &str, at: usize) -> bool {
    body.as_bytes()[..at].iter().rev().take_while(|&&b| b == b'\\').count() % 2 == 1
}

fn parse(body: &str) -> Vec<Link> {
    let masked = outside_code(body);
    pattern()
        .captures_iter(&masked)
        .filter_map(|caps| {
            let found = caps.get(0)?;
            if escaped(body, found.start()) {
                return None;
            }
            // Read link text from the original bytes: masking is only for
            // locating links, and must not erase code-formatted aliases.
            let caps = pattern().captures(&body[found.range()])?;
            let (target, kind, alias, opening) = if let Some(wiki) = caps.get(1) {
                let (target, alias) = wiki.as_str().split_once('|').map_or((wiki.as_str(), None), |(t, a)| (t, Some(a.to_owned())));
                (target.trim().to_owned(), "wiki", alias, None)
            } else {
                (caps[3].to_owned(), "page", None, Some(caps[2].to_owned()))
            };
            (!target.is_empty()).then_some(Link {
                range: found.range(),
                target,
                kind,
                alias,
                opening,
            })
        })
        .collect()
}

pub(crate) fn display_title(title: Option<&str>, body: &str) -> String {
    if let Some(title) = title.map(str::trim).filter(|title| !title.is_empty()) {
        return title.to_owned();
    }
    // Match noteDisplayTitle in notesDraft.ts, including its ellipsis.
    let line = body
        .lines()
        .map(|line| line.trim_start_matches('#').trim())
        .find(|line| !line.is_empty())
        .unwrap_or("Untitled");
    if line.chars().count() > 80 {
        format!("{}…", line.chars().take(79).collect::<String>())
    } else {
        line.to_owned()
    }
}

fn excerpt(body: &str, link: &Link) -> String {
    let start = body[..link.range.start].rfind('\n').map_or(0, |i| i + 1);
    let end = body[link.range.end..].find('\n').map_or(body.len(), |i| link.range.end + i);
    let line = &body[start..end];
    let clean = pattern().replace_all(line, |caps: &regex::Captures<'_>| {
        if let Some(wiki) = caps.get(1) {
            wiki.as_str().split_once('|').map_or(wiki.as_str(), |(_, alias)| alias).to_owned()
        } else {
            caps[4].to_owned()
        }
    });
    clean.trim().chars().filter(|ch| !ch.is_control()).take(200).collect()
}

fn escape_text(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\r', "&#13;").replace('\n', "&#10;")
}

/// Rewrite links to a renamed page. Page tags name it by id; wiki links by
/// title, and only when `wiki` says this source's `[[old]]` resolved to it. A
/// title the wiki grammar cannot hold becomes a page tag, keeping the alias.
pub(crate) fn rewrite_links_on_rename(body: &str, id: &str, old: &str, new: &str, wiki: bool) -> String {
    let mut rewritten = body.to_owned();
    let safe = !new.contains(['[', ']', '|', '\r', '\n']);
    for link in parse(body).into_iter().rev() {
        let replacement = if wiki && link.kind == "wiki" && link_key(&link.target) == link_key(old) {
            Some(match (link.alias, safe) {
                (Some(alias), true) => format!("[[{new}|{alias}]]"),
                (None, true) => format!("[[{new}]]"),
                (alias, false) => format!("<page url=\"bridge://p/{id}\">{}</page>", escape_text(alias.as_deref().unwrap_or(new))),
            })
        } else if link.kind == "page" && link.target == id {
            Some(format!("{}{}</page>", link.opening.unwrap(), escape_text(new)))
        } else {
            None
        };
        if let Some(replacement) = replacement {
            rewritten.replace_range(link.range, &replacement);
        }
    }
    rewritten
}

/// The case-folded title a wiki link resolves by.
pub(crate) fn link_key(title: &str) -> String {
    title.trim().to_lowercase()
}

fn resolve_one(db: &Connection, space: &str, kind: &str, target: &str) -> Result<Option<String>, BridgeError> {
    let sql = if kind == "page" {
        "SELECT id FROM pages WHERE space_id=?1 AND id=?2 AND deleted_at IS NULL"
    } else {
        "SELECT id FROM pages WHERE space_id=?1 AND link_key=?2 AND deleted_at IS NULL ORDER BY created_at, id LIMIT 1"
    };
    let key = if kind == "page" { target.to_owned() } else { link_key(target) };
    Ok(db.query_row(sql, params![space, key], |r| r.get(0)).optional()?)
}

fn set_target(db: &Connection, space: &str, from: &str, kind: &str, target: &str, to: Option<&str>) -> Result<(), BridgeError> {
    db.execute(
        "UPDATE page_links SET to_page_id=?5 WHERE space_id=?1 AND from_page_id=?2 AND kind=?3 AND target_title=?4",
        params![space, from, kind, target, to],
    )?;
    Ok(())
}

/// Resolve every link in a space in one pass, for rebuilds and migration.
pub(crate) fn resolve_space(db: &Connection, space: &str) -> Result<(), BridgeError> {
    let mut ids = HashSet::new();
    let mut titles = HashMap::new();
    {
        let mut stmt = db.prepare("SELECT id, link_key FROM pages WHERE space_id=?1 AND deleted_at IS NULL ORDER BY created_at, id")?;
        let mut rows = stmt.query([space])?;
        while let Some(row) = rows.next()? {
            let id: String = row.get(0)?;
            if let Some(key) = row.get::<_, Option<String>>(1)? {
                titles.entry(key).or_insert_with(|| id.clone());
            }
            ids.insert(id);
        }
    }
    let links: Vec<(String, String, String, Option<String>)> = {
        let mut stmt = db.prepare("SELECT from_page_id, kind, target_title, to_page_id FROM page_links WHERE space_id=?1")?;
        let rows = stmt.query_map([space], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?.collect::<Result<_, _>>()?;
        rows
    };
    for (from, kind, target, previous) in links {
        let to = if kind == "page" { ids.get(&target).map(String::as_str) } else { titles.get(&link_key(&target)).map(String::as_str) };
        if previous.as_deref() != to {
            set_target(db, space, &from, &kind, &target, to)?;
        }
    }
    Ok(())
}

/// Record a page's resolvable title and replace its outbound edges, unresolved.
pub(crate) fn write_outbound(db: &Connection, id: &str) -> Result<(String, String), BridgeError> {
    let (space, title, body, deleted): (String, Option<String>, String, bool) =
        db.query_row("SELECT space_id, title, body, deleted_at IS NOT NULL FROM pages WHERE id=?1", [id], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })?;
    let key = link_key(&display_title(title.as_deref(), &body));
    db.execute("UPDATE pages SET link_key=?2 WHERE id=?1", params![id, key])?;
    db.execute("DELETE FROM page_links WHERE space_id=?1 AND from_page_id=?2", params![space, id])?;
    if !deleted {
        for link in parse(&body) {
            let target_key = if link.kind == "page" { link.target.clone() } else { link_key(&link.target) };
            db.execute(
                "INSERT OR IGNORE INTO page_links(space_id, from_page_id, target_title, target_key, kind, excerpt) VALUES(?1,?2,?3,?4,?5,?6)",
                params![space, id, link.target, target_key, link.kind, excerpt(&body, &link)],
            )?;
        }
    }
    Ok((space, key))
}

/// Refresh one page's links and only the edges its change can move: its own
/// outbound links, links resolved to it, and links naming its title or id.
pub(crate) fn sync_page_links(db: &Connection, id: &str) -> Result<(), BridgeError> {
    let (space, key) = write_outbound(db, id)?;
    let affected: Vec<(String, String, String, Option<String>)> = {
        let mut stmt = db.prepare(
            "SELECT from_page_id, kind, target_title, to_page_id FROM page_links WHERE space_id=?1 AND from_page_id=?2
             UNION SELECT from_page_id, kind, target_title, to_page_id FROM page_links WHERE space_id=?1 AND to_page_id=?2
             UNION SELECT from_page_id, kind, target_title, to_page_id FROM page_links WHERE space_id=?1 AND target_key=?3 AND kind='wiki'
             UNION SELECT from_page_id, kind, target_title, to_page_id FROM page_links WHERE space_id=?1 AND target_key=?2 AND kind='page'",
        )?;
        let rows = stmt.query_map(params![space, id, key], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?.collect::<Result<_, _>>()?;
        rows
    };
    for (from, kind, target, previous) in affected {
        let to = resolve_one(db, &space, &kind, &target)?;
        if previous != to {
            set_target(db, &space, &from, &kind, &target, to.as_deref())?;
        }
    }
    Ok(())
}

/// Replace a page's search row. The map gives each page a stable FTS rowid,
/// so replacement is a rowid lookup instead of a scan of the UNINDEXED id.
pub(crate) fn sync_fts(db: &Connection, id: &str) -> Result<(), BridgeError> {
    db.execute("INSERT OR IGNORE INTO pages_fts_map(page_id) VALUES(?1)", [id])?;
    db.execute("DELETE FROM pages_fts WHERE rowid=(SELECT fts_rowid FROM pages_fts_map WHERE page_id=?1)", [id])?;
    db.execute(
        "INSERT INTO pages_fts(rowid, page_id, space_id, title, body, tags) SELECT m.fts_rowid, p.id, p.space_id, p.title, p.body, p.tags
         FROM pages p JOIN pages_fts_map m ON m.page_id=p.id WHERE p.id=?1 AND p.deleted_at IS NULL",
        [id],
    )?;
    Ok(())
}

/// Populate the empty indexes in bulk: search rows in one statement, then
/// every page's edges, then one resolution per space.
pub(crate) fn backfill(db: &Connection) -> Result<(), BridgeError> {
    db.execute("INSERT OR IGNORE INTO pages_fts_map(page_id) SELECT id FROM pages", [])?;
    db.execute(
        "INSERT INTO pages_fts(rowid, page_id, space_id, title, body, tags) SELECT m.fts_rowid, p.id, p.space_id, p.title, p.body, p.tags
         FROM pages p JOIN pages_fts_map m ON m.page_id=p.id WHERE p.deleted_at IS NULL",
        [],
    )?;
    let ids: Vec<String> = {
        let mut stmt = db.prepare("SELECT id FROM pages ORDER BY id")?;
        let ids = stmt.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?;
        ids
    };
    let mut spaces = HashSet::new();
    for id in ids {
        spaces.insert(write_outbound(db, &id)?.0);
    }
    for space in spaces {
        resolve_space(db, &space)?;
    }
    Ok(())
}

pub(crate) fn read_backlinks(db: &Connection, space: &str, id: &str) -> Result<Vec<NoteBacklink>, BridgeError> {
    let mut stmt = db.prepare("SELECT p.id, p.title, substr(p.body, 1, 280), l.kind, l.excerpt, p.updated_at FROM page_links l JOIN pages p ON p.id=l.from_page_id AND p.space_id=l.space_id WHERE l.space_id=?1 AND l.to_page_id=?2 AND p.deleted_at IS NULL AND p.id<>?2 ORDER BY p.updated_at DESC, p.id, l.kind, l.target_title")?;
    let rows = stmt
        .query_map(params![space, id], |r| {
            Ok(NoteBacklink {
                page_id: r.get(0)?,
                title: display_title(r.get::<_, Option<String>>(1)?.as_deref(), &r.get::<_, String>(2)?),
                kind: r.get(3)?,
                excerpt: r.get(4)?,
                updated_at: r.get(5)?,
            })
        })?
        .collect::<Result<_, _>>()?;
    Ok(rows)
}

/// unicode61 keeps combining marks inside a token (and folds them away), so a
/// decomposed `e\u{301}cole` is one word there and must be one here.
fn combining_mark(ch: char) -> bool {
    matches!(ch, '\u{300}'..='\u{36f}' | '\u{483}'..='\u{489}' | '\u{591}'..='\u{5bd}' | '\u{1ab0}'..='\u{1aff}' | '\u{1dc0}'..='\u{1dff}' | '\u{20d0}'..='\u{20ff}' | '\u{fe20}'..='\u{fe2f}')
}

pub(crate) fn fts_query(query: &str) -> Option<String> {
    let tokens: Vec<String> = query
        .split(|ch: char| !ch.is_alphanumeric() && !combining_mark(ch))
        .filter(|token| !token.is_empty())
        .map(|token| format!("\"{token}\"*"))
        .collect();
    (!tokens.is_empty()).then(|| tokens.join(" AND "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_aliases_page_tags_and_excludes_code() {
        let body = "See [[ Plan |the plan]] and <page url=\"bridge://p/id\">Plan</page>.\n`[[inline]]` ``a ` [[nested]]``\n```md\n[[fenced]]\n```\n~~~\n[[tilde]]\n~~~\n[[ ]] [[broken\nlink]] \\[[escaped]]\n[[Visible]]";
        let links = parse(body);
        assert_eq!(
            links.iter().map(|l| (l.kind, l.target.as_str())).collect::<Vec<_>>(),
            [("wiki", "Plan"), ("page", "id"), ("wiki", "Visible")]
        );
        assert_eq!(links[0].alias.as_deref(), Some("the plan"));
        assert_eq!(excerpt(body, &links[0]), "See the plan and Plan.");
    }

    #[test]
    fn rename_preserves_aliases_code_and_surrounding_bytes() {
        let body = "  [[ old ]] / [[OLD|my alias]]\r\n<page url=\"bridge://p/id\">old</page>\r\n`[[old]]`\n```\n[[old]]\n```\n[[other]]";
        assert_eq!(
            rewrite_links_on_rename(body, "id", "Old", "New & <Title>", true),
            "  [[New & <Title>]] / [[New & <Title>|my alias]]\r\n<page url=\"bridge://p/id\">New &amp; &lt;Title&gt;</page>\r\n`[[old]]`\n```\n[[old]]\n```\n[[other]]"
        );
    }

    #[test]
    fn rename_preserves_code_formatted_aliases_and_long_fences() {
        assert_eq!(rewrite_links_on_rename("[[Old|`literal alias`]]", "id", "Old", "New", true), "[[New|`literal alias`]]");
        assert!(parse("````md\n```\n[[Hidden]]\n````").is_empty());
        assert!(parse("~~~md\n[[Hidden]]").is_empty());
        assert_eq!(display_title(None, "#\n## Derived"), "Derived");
        assert_eq!(display_title(None, &"a".repeat(100)), format!("{}…", "a".repeat(79)));
    }

    #[test]
    fn unicode_excerpts_are_clipped_and_queries_are_safe() {
        let body = format!("{} [[目標]]", "界".repeat(220));
        assert_eq!(excerpt(&body, &parse(&body)[0]).chars().count(), 200);
        assert_eq!(fts_query("launch OR \"plan\"*"), Some("\"launch\"* AND \"OR\"* AND \"plan\"*".into()));
        assert_eq!(fts_query("%_!"), None);
        assert_eq!(parse("`unclosed [[visible]]").len(), 1);
        assert!(parse("``code\n[[hidden]]\n``").is_empty());
    }
}
