//! Taste rules out to plain instruction files and back (#1206).
//!
//! Export writes a section Bridge manages, between two markers, into a
//! repository's `AGENTS.md` and `CLAUDE.md` (code rules, for any coding agent)
//! and `.cursor/BUGBOT.md` (review rules, for Cursor's reviewer). Text outside
//! the markers is never read on export or changed; a file without them gets
//! the section at its end. Import reads the section back: a bullet that names
//! a rule updates it, a new bullet becomes a rule the person wrote, and a rule
//! missing from the file is reported, never deleted.

use std::path::{Path, PathBuf};

use bridge_protocol::messages::MemoryRecord;
use rusqlite::Connection;
use serde::Serialize;

use crate::BridgeError;

pub const START: &str = "<!-- bridge:taste:start v1 -->";
pub const END: &str = "<!-- bridge:taste:end -->";

/// A file the rules go to, and which kinds it carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Target {
    #[serde(rename = "AGENTS.md")]
    Agents,
    #[serde(rename = "CLAUDE.md")]
    Claude,
    #[serde(rename = ".cursor/BUGBOT.md")]
    Bugbot,
}

impl Target {
    pub const ALL: [Target; 3] = [Target::Agents, Target::Claude, Target::Bugbot];

    pub fn path(self) -> &'static str {
        match self {
            Target::Agents => "AGENTS.md",
            Target::Claude => "CLAUDE.md",
            Target::Bugbot => ".cursor/BUGBOT.md",
        }
    }

    pub fn parse(raw: &str) -> Option<Target> {
        Target::ALL.into_iter().find(|target| target.path() == raw.trim())
    }

    fn carries(self, kind: &str) -> bool {
        match self {
            Target::Agents | Target::Claude => kind == "code_prefer",
            Target::Bugbot => kind == "review_avoid" || kind == "review_prefer",
        }
    }

    /// The kind a new bullet in this file is read as.
    fn default_kind(self) -> &'static str {
        match self {
            Target::Agents | Target::Claude => "code_prefer",
            Target::Bugbot => "review_avoid",
        }
    }

    fn heading(self) -> &'static str {
        match self {
            Target::Agents | Target::Claude => "## How code is written here (from Bridge)",
            Target::Bugbot => "## Review rules (from Bridge)",
        }
    }
}

/// The managed section for `rules`, markers included, ending in a newline.
pub fn render(target: Target, rules: &[MemoryRecord]) -> String {
    let mut section = format!("{START}\n{}\n\n", target.heading());
    let mut carried = rules.iter().filter(|rule| target.carries(&rule.kind)).peekable();
    if carried.peek().is_none() {
        section.push_str("_No rules yet._\n");
    }
    for rule in carried {
        let paths = rule.path_globs.as_ref().filter(|globs| !globs.is_empty()).map(|globs| format!(" (paths: {})", globs.join(", "))).unwrap_or_default();
        section.push_str(&format!(
            "- {}{paths} <!-- id:{} kind:{} -->\n",
            crate::memory_packet::render_body(&rule.body),
            rule.id,
            rule.kind
        ));
    }
    section.push_str(END);
    section.push('\n');
    section
}

/// Where the managed section sits in `text`, as a byte range, when both
/// markers are there in order.
fn section_range(text: &str) -> Option<std::ops::Range<usize>> {
    let start = text.find(START)?;
    let end = text[start..].find(END)? + start + END.len();
    let end = if text[end..].starts_with('\n') { end + 1 } else { end };
    Some(start..end)
}

/// `existing` with its managed section replaced by `section`, or `section`
/// appended after a blank line when it has none.
pub fn merge(existing: &str, section: &str) -> String {
    match section_range(existing) {
        Some(range) => format!("{}{section}{}", &existing[..range.start], &existing[range.end..]),
        None if existing.trim().is_empty() => section.to_string(),
        None => format!("{}\n\n{section}", existing.trim_end_matches('\n')),
    }
}

/// One bullet of a managed section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bullet {
    pub id: Option<String>,
    pub kind: Option<String>,
    pub body: String,
    pub path_globs: Option<Vec<String>>,
}

/// The bullets of `text`'s managed section. A section with a start and no
/// end is refused with the line it starts on, so nothing is half-read.
pub fn parse(text: &str) -> Result<Vec<Bullet>, String> {
    let Some(start) = text.find(START) else { return Ok(Vec::new()) };
    let Some(range) = section_range(text) else {
        let line = text[..start].lines().count() + 1;
        return Err(format!("The section that starts on line {line} has no end marker ({END})."));
    };
    let inner = &text[range.start + START.len()..range.end].trim_end().trim_end_matches(END);
    Ok(inner
        .lines()
        .filter_map(|line| line.trim().strip_prefix("- "))
        .map(|line| {
            let (text, meta) = match line.rsplit_once("<!--") {
                Some((text, meta)) if meta.trim_end().ends_with("-->") => (text.trim(), meta.trim().trim_end_matches("-->").trim()),
                _ => (line.trim(), ""),
            };
            let field = |name: &str| meta.split_whitespace().find_map(|part| part.strip_prefix(name)).map(str::to_owned);
            let (body, path_globs) = match text.rsplit_once(" (paths: ") {
                Some((body, paths)) if paths.ends_with(')') => (
                    body.trim().to_string(),
                    Some(paths.trim_end_matches(')').split(',').map(|glob| glob.trim().to_string()).filter(|glob| !glob.is_empty()).collect()),
                ),
                _ => (text.to_string(), None),
            };
            Bullet { id: field("id:"), kind: field("kind:"), body, path_globs }
        })
        .filter(|bullet| !bullet.body.is_empty())
        .collect())
}

/// One file as export would leave it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileChange {
    pub target: Target,
    pub path: String,
    pub before: String,
    pub after: String,
    pub changed: bool,
}

/// The active rules of `scope`, the ones export writes.
fn rules_of(db: &Connection, scope: &str) -> Result<Vec<MemoryRecord>, BridgeError> {
    Ok(crate::memory_ledger::list_taste(db, None)?.into_iter().filter(|rule| rule.scope_key == scope).collect())
}

/// What export would do to each target in the checkout at `root`, and, when
/// `write`, does it. A file that does not exist reads as empty; `.cursor/` is
/// created for BUGBOT.md only when it is written.
pub fn export(db: &Connection, scope: &str, root: &Path, targets: &[Target], write: bool) -> Result<Vec<FileChange>, BridgeError> {
    let rules = rules_of(db, scope)?;
    let mut changes = Vec::new();
    for target in targets {
        let path: PathBuf = root.join(target.path());
        let before = std::fs::read_to_string(&path).unwrap_or_default();
        if !before.is_empty() {
            parse(&before).map_err(|error| BridgeError::Invalid(format!("{}: {error}", target.path())))?;
        }
        let after = merge(&before, &render(*target, &rules));
        let changed = after != before;
        if write && changed {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(|error| BridgeError::Invalid(format!("{}: {error}", target.path())))?;
            }
            std::fs::write(&path, &after).map_err(|error| BridgeError::Invalid(format!("{}: {error}", target.path())))?;
        }
        changes.push(FileChange { target: *target, path: target.path().to_string(), before, after, changed });
    }
    Ok(changes)
}

/// What import would do.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportPlan {
    /// New bullets, which become rules the person wrote.
    pub added: Vec<String>,
    /// Rules whose text the file changed: (id, new text).
    pub updated: Vec<(String, String)>,
    /// Active rules of the scope the files no longer list. Reported, never
    /// deleted: the person decides whether to pause them.
    pub missing: Vec<String>,
}

/// Read the managed sections of every target in `root` back into `scope`'s
/// rules; when `apply`, write the changes.
pub fn import(db: &Connection, scope: &str, root: &Path, apply: bool) -> Result<ImportPlan, BridgeError> {
    let rules = rules_of(db, scope)?;
    let mut plan = ImportPlan::default();
    let mut seen: Vec<String> = Vec::new();
    let mut read: Vec<Target> = Vec::new();
    for target in Target::ALL {
        let Ok(text) = std::fs::read_to_string(root.join(target.path())) else { continue };
        if !text.contains(START) {
            continue;
        }
        read.push(target);
        let bullets = parse(&text).map_err(|error| BridgeError::Invalid(format!("{}: {error}", target.path())))?;
        for bullet in bullets {
            let known = bullet.id.as_ref().and_then(|id| rules.iter().find(|rule| &rule.id == id));
            match known {
                Some(rule) => {
                    seen.push(rule.id.clone());
                    if rule.body.trim() != bullet.body.trim() && !plan.updated.iter().any(|(id, _)| id == &rule.id) {
                        plan.updated.push((rule.id.clone(), bullet.body.clone()));
                        if apply {
                            crate::memory_ledger::supersede(db, &rule.id, &bullet.body, None)?;
                        }
                    }
                }
                None => {
                    if rules.iter().any(|rule| rule.body.trim().eq_ignore_ascii_case(bullet.body.trim())) || plan.added.contains(&bullet.body) {
                        continue;
                    }
                    plan.added.push(bullet.body.clone());
                    if apply {
                        let kind = bullet
                            .kind
                            .as_deref()
                            .filter(|kind| bridge_protocol::messages::TASTE_RULE_KINDS.contains(kind))
                            .unwrap_or(target.default_kind());
                        crate::memory_ledger::save_taste_rule(db, scope, kind, &bullet.body, bullet.path_globs.as_deref())?;
                    }
                }
            }
        }
    }
    // Missing only from a file that would carry it: a review rule is not
    // missing from AGENTS.md.
    plan.missing = rules
        .iter()
        .filter(|rule| read.iter().any(|target| target.carries(&rule.kind)) && !seen.contains(&rule.id))
        .map(|rule| rule.id.clone())
        .collect();
    Ok(plan)
}

#[cfg(test)]
#[path = "export_tests.rs"]
mod tests;
