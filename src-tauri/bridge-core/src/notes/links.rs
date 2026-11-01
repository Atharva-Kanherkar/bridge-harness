//! Derived full-text and link indexes. Markdown stays authoritative and is never
//! serialized by the parser: renames replace only the exact link byte ranges.
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::sync::OnceLock;

use bridge_protocol::messages::NoteBacklink;
use regex::Regex;
use rusqlite::{params, Connection};

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

/// Mask code with spaces, retaining byte offsets and newlines for replacements.
fn outside_code(body: &str) -> String {
    let mut masked = body.as_bytes().to_vec();
    let mut fence: Option<(u8, usize)> = None;
    let mut offset = 0;
    for line in body.split_inclusive('\n') {
        let trimmed = line.trim_start_matches(' ');
        let indent = line.len() - trimmed.len();
        let marker = trimmed.as_bytes().first().copied().unwrap_or(0);
        let count = trimmed.bytes().take_while(|&b| b == marker).count();
        let was_fenced = fence.is_some();
        if indent <= 3 && matches!(marker, b'`' | b'~') && count >= 3 {
            match fence {
                Some((ch, n)) if marker == ch && count >= n && trimmed[count..].trim().is_empty() => fence = None,
                None if marker == b'~' || !trimmed[count..].contains('`') => fence = Some((marker, count)),
                _ => {}
            }
        }
        if was_fenced || fence.is_some() {
            for b in &mut masked[offset..offset + line.len()] {
                if *b != b'\n' {
                    *b = b' ';
                }
            }
        }
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

pub(crate) fn rewrite_links_on_rename(body: &str, id: &str, old: &str, new: &str) -> String {
    let mut rewritten = body.to_owned();
    for link in parse(body).into_iter().rev() {
        let replacement = if link.kind == "wiki" && link.target.to_lowercase() == old.trim().to_lowercase() {
            Some(match link.alias {
                Some(alias) => format!("[[{new}|{alias}]]"),
                None => format!("[[{new}]]"),
            })
        } else if link.kind == "page" && link.target == id {
            let escaped = new.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;");
            Some(format!("{}{escaped}</page>", link.opening.unwrap()))
        } else {
            None
        };
        if let Some(replacement) = replacement {
            rewritten.replace_range(link.range, &replacement);
        }
    }
    rewritten
}

/// Resolve both newly inserted links and incoming links whose target changed.
/// Re-evaluating resolved links also removes stale edges after deletion/retitling.
fn resolve(db: &Connection, space: &str) -> Result<(), BridgeError> {
    let pages: Vec<(String, String)> = {
        let mut stmt = db.prepare("SELECT id, title, substr(body, 1, 280) FROM pages WHERE space_id=?1 AND deleted_at IS NULL ORDER BY created_at, id")?;
        let rows = stmt
            .query_map([space], |r| {
                Ok((r.get(0)?, display_title(r.get::<_, Option<String>>(1)?.as_deref(), &r.get::<_, String>(2)?)))
            })?
            .collect::<Result<_, _>>()?;
        rows
    };
    let ids: HashSet<&str> = pages.iter().map(|(id, _)| id.as_str()).collect();
    let mut titles = HashMap::new();
    for (id, title) in &pages {
        titles.entry(title.trim().to_lowercase()).or_insert(id.as_str());
    }
    let links: Vec<(String, String, String, Option<String>)> = {
        let mut stmt = db.prepare("SELECT from_page_id, kind, target_title, to_page_id FROM page_links WHERE space_id=?1")?;
        let rows = stmt.query_map([space], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?.collect::<Result<_, _>>()?;
        rows
    };
    for (from, kind, target, previous) in links {
        let to = if kind == "page" {
            ids.get(target.as_str()).copied()
        } else {
            titles.get(&target.to_lowercase()).copied()
        };
        if previous.as_deref() == to {
            continue;
        }
        db.execute(
            "UPDATE page_links SET to_page_id=?4 WHERE space_id=?1 AND from_page_id=?2 AND kind=?3 AND target_title=?5",
            params![space, from, kind, to, target],
        )?;
    }
    Ok(())
}

fn insert_outbound(db: &Connection, id: &str) -> Result<String, BridgeError> {
    db.execute("DELETE FROM page_links WHERE from_page_id=?1", [id])?;
    let row: (String, String, bool) = db.query_row("SELECT space_id, body, deleted_at IS NOT NULL FROM pages WHERE id=?1", [id], |r| {
        Ok((r.get(0)?, r.get(1)?, r.get(2)?))
    })?;
    if !row.2 {
        for link in parse(&row.1) {
            db.execute(
                "INSERT OR IGNORE INTO page_links(space_id, from_page_id, target_title, kind, excerpt) VALUES(?1,?2,?3,?4,?5)",
                params![row.0, id, link.target, link.kind, excerpt(&row.1, &link)],
            )?;
        }
    }
    Ok(row.0)
}

pub(crate) fn sync_page_links(db: &Connection, id: &str) -> Result<(), BridgeError> {
    let space = insert_outbound(db, id)?;
    resolve(db, &space)
}

pub(crate) fn sync_fts(db: &Connection, id: &str) -> Result<(), BridgeError> {
    db.execute("DELETE FROM pages_fts WHERE page_id=?1", [id])?;
    db.execute(
        "INSERT INTO pages_fts(page_id, space_id, title, body, tags) SELECT id, space_id, title, body, tags FROM pages WHERE id=?1 AND deleted_at IS NULL",
        [id],
    )?;
    Ok(())
}

pub(crate) fn backfill(db: &Connection) -> Result<(), BridgeError> {
    let ids: Vec<String> = {
        let mut stmt = db.prepare("SELECT id FROM pages ORDER BY id")?;
        let ids = stmt.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?;
        ids
    };
    let mut spaces = HashSet::new();
    for id in ids {
        sync_fts(db, &id)?;
        spaces.insert(insert_outbound(db, &id)?);
    }
    for space in spaces {
        resolve(db, &space)?;
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

pub(crate) fn fts_query(query: &str) -> Option<String> {
    let tokens: Vec<String> = query
        .split(|ch: char| !ch.is_alphanumeric())
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
            rewrite_links_on_rename(body, "id", "Old", "New & <Title>"),
            "  [[New & <Title>]] / [[New & <Title>|my alias]]\r\n<page url=\"bridge://p/id\">New &amp; &lt;Title&gt;</page>\r\n`[[old]]`\n```\n[[old]]\n```\n[[other]]"
        );
    }

    #[test]
    fn rename_preserves_code_formatted_aliases_and_long_fences() {
        assert_eq!(rewrite_links_on_rename("[[Old|`literal alias`]]", "id", "Old", "New"), "[[New|`literal alias`]]");
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
