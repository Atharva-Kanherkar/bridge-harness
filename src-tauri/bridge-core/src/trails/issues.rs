//! GitHub issues on a trail: the reference a person pastes, the snapshot the
//! trail keeps, and the hard links between issues and pull requests.
//!
//! An issue member's ref is the issue's canonical URL. The snapshot is what
//! the last successful read said; nothing here calls `gh` while holding the
//! database lock. Links come from both directions, the issue's
//! `closedByPullRequestsReferences` and a pull request's
//! `closingIssuesReferences`, so either record joining a trail can bring the
//! other.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use bridge_protocol::messages::TrailMemberKind;
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};

use crate::github_surface::{GithubRepository, IssueRead, IssueState};
use crate::{BridgeCore, BridgeError};

/// The most of an issue body the store keeps.
pub const BODY_MAX_BYTES: usize = 32_000;
/// The comments a snapshot keeps, newest last.
pub const KEPT_COMMENTS: usize = 5;

pub fn install_store(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS issue_snapshots (
            url TEXT PRIMARY KEY,
            repo_host TEXT NOT NULL,
            repo_owner TEXT NOT NULL,
            repo_name TEXT NOT NULL,
            number INTEGER NOT NULL,
            title TEXT NOT NULL,
            state TEXT NOT NULL,
            state_reason TEXT,
            author TEXT,
            labels TEXT NOT NULL,
            body TEXT NOT NULL,
            comments TEXT NOT NULL,
            closed_by TEXT NOT NULL,
            created_at TEXT NOT NULL,
            closed_at TEXT,
            fetched_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS trail_issue_links (
            pr_url TEXT NOT NULL,
            issue_url TEXT NOT NULL,
            source TEXT NOT NULL,
            PRIMARY KEY (pr_url, issue_url, source)
        );
        CREATE INDEX IF NOT EXISTS idx_trail_issue_links_issue ON trail_issue_links(issue_url);
        CREATE TABLE IF NOT EXISTS trail_pr_link_reads (
            pr_url TEXT PRIMARY KEY,
            fetched_at TEXT NOT NULL
        );",
    )?;
    Ok(())
}

/// One issue, by repository and number.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueUrl {
    pub repository: GithubRepository,
    pub number: u64,
}

impl IssueUrl {
    pub fn url(&self) -> String {
        format!("https://{}/{}/{}/issues/{}", self.repository.host, self.repository.owner, self.repository.name, self.number)
    }
}

const FORMS: &str = "Paste the issue's link, or owner/repo#number";

/// `https://<host>/<owner>/<name>/issues/<n>` with or without the scheme; a
/// trailing slash, query or fragment is ignored. `None` for anything else.
pub fn parse_url(raw: &str) -> Option<IssueUrl> {
    let raw = raw.trim();
    let raw = raw.strip_prefix("https://").or_else(|| raw.strip_prefix("http://")).unwrap_or(raw);
    let raw = raw.split(['?', '#']).next().unwrap_or(raw).trim_end_matches('/');
    let parts: Vec<&str> = raw.split('/').collect();
    let [host, owner, name, "issues", number] = parts.as_slice() else { return None };
    if !host.contains('.') || owner.is_empty() || name.is_empty() {
        return None;
    }
    Some(IssueUrl {
        repository: GithubRepository { host: host.to_lowercase(), owner: (*owner).to_owned(), name: (*name).to_owned() },
        number: number.parse().ok().filter(|number| *number > 0)?,
    })
}

/// What a person pastes into "Add issue": the URL, `owner/repo#n`, or `#n`
/// and `n` when the trail's project gives a repository.
pub fn parse_reference(raw: &str, default: Option<&GithubRepository>) -> Result<IssueUrl, BridgeError> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(BridgeError::Invalid(format!("{FORMS}.")));
    }
    if let Some(issue) = parse_url(raw) {
        return Ok(issue);
    }
    if raw.contains("/pull/") {
        return Err(BridgeError::Invalid(format!("That link is a pull request, not an issue. {FORMS}.")));
    }
    let number = |text: &str| text.trim().parse::<u64>().ok().filter(|number| *number > 0);
    if let Some((repository, issue)) = raw.split_once('#') {
        if repository.is_empty() {
            return match (number(issue), default) {
                (Some(number), Some(repository)) => Ok(IssueUrl { repository: repository.clone(), number }),
                (Some(_), None) => Err(BridgeError::Invalid(format!("This trail has no project to read {raw} from. {FORMS}."))),
                (None, _) => Err(BridgeError::Invalid(format!("{FORMS}."))),
            };
        }
        if let (Some((owner, name)), Some(number)) = (repository.split_once('/'), number(issue)) {
            if !owner.is_empty() && !name.is_empty() && !name.contains('/') {
                return Ok(IssueUrl { repository: GithubRepository { host: "github.com".into(), owner: owner.into(), name: name.into() }, number });
            }
        }
        return Err(BridgeError::Invalid(format!("{FORMS}.")));
    }
    match (number(raw), default) {
        (Some(number), Some(repository)) => Ok(IssueUrl { repository: repository.clone(), number }),
        (Some(_), None) => Err(BridgeError::Invalid(format!("This trail has no project to read #{raw} from. {FORMS}."))),
        (None, _) => Err(BridgeError::Invalid(format!("{FORMS}."))),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueComment {
    pub author: Option<String>,
    pub body: String,
    pub created_at: String,
}

/// An issue as the last successful read said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueSnapshot {
    pub url: String,
    pub repository: GithubRepository,
    pub number: u64,
    pub title: String,
    pub open: bool,
    /// `COMPLETED`, `NOT_PLANNED`, ...; `None` while it never closed.
    pub state_reason: Option<String>,
    pub author: Option<String>,
    pub labels: Vec<String>,
    pub body: String,
    /// The last [`KEPT_COMMENTS`], oldest first.
    pub comments: Vec<IssueComment>,
    pub comment_count: usize,
    pub closed_by: Vec<String>,
    pub created_at: String,
    pub closed_at: Option<String>,
    pub fetched_at: String,
}

fn clip(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_owned();
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}

impl IssueSnapshot {
    pub fn from_read(read: IssueRead, fetched_at: &str) -> Result<Self, BridgeError> {
        let summary = read.detail.summary;
        let issue = parse_url(&summary.url)
            .ok_or_else(|| BridgeError::Invalid(format!("GitHub answered with an issue link Bridge cannot read: {}", summary.url)))?;
        let comment_count = read.detail.comments.len();
        let skip = comment_count.saturating_sub(KEPT_COMMENTS);
        Ok(Self {
            url: issue.url(),
            repository: issue.repository,
            number: summary.number,
            title: summary.title,
            open: summary.state == IssueState::Open,
            state_reason: read.state_reason,
            author: summary.author.map(|author| author.login),
            labels: summary.labels.into_iter().map(|label| label.name).collect(),
            body: clip(&read.detail.body, BODY_MAX_BYTES),
            comments: read
                .detail
                .comments
                .into_iter()
                .skip(skip)
                .map(|comment| IssueComment { author: comment.author.map(|author| author.login), body: clip(&comment.body, BODY_MAX_BYTES), created_at: comment.created_at })
                .collect(),
            comment_count,
            closed_by: read.closed_by,
            created_at: summary.created_at,
            closed_at: read.closed_at,
            fetched_at: fetched_at.to_owned(),
        })
    }

    /// `open`, `closed`, `closed as not planned`.
    pub fn state_words(&self) -> &'static str {
        match (self.open, self.state_reason.as_deref()) {
            (true, _) => "open",
            (false, Some("NOT_PLANNED")) => "closed as not planned",
            (false, Some("DUPLICATE")) => "closed as a duplicate",
            (false, _) => "closed",
        }
    }

    pub fn completed(&self) -> bool {
        !self.open && !matches!(self.state_reason.as_deref(), Some("NOT_PLANNED" | "DUPLICATE"))
    }
}

/// Store a snapshot and the links it carries (replacing the issue side's old
/// links, so an unlinked PR stops pulling the issue in).
pub fn save(db: &Connection, snapshot: &IssueSnapshot) -> Result<(), BridgeError> {
    db.execute(
        "INSERT INTO issue_snapshots(url,repo_host,repo_owner,repo_name,number,title,state,state_reason,author,labels,body,comments,closed_by,created_at,closed_at,fetched_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)
         ON CONFLICT(url) DO UPDATE SET title=excluded.title,state=excluded.state,state_reason=excluded.state_reason,author=excluded.author,
           labels=excluded.labels,body=excluded.body,comments=excluded.comments,closed_by=excluded.closed_by,created_at=excluded.created_at,
           closed_at=excluded.closed_at,fetched_at=excluded.fetched_at",
        params![
            snapshot.url,
            snapshot.repository.host,
            snapshot.repository.owner,
            snapshot.repository.name,
            snapshot.number as i64,
            snapshot.title,
            if snapshot.open { "open" } else { "closed" },
            snapshot.state_reason,
            snapshot.author,
            json(&snapshot.labels),
            snapshot.body,
            serde_json::json!({ "count": snapshot.comment_count, "last": snapshot.comments }).to_string(),
            json(&snapshot.closed_by),
            snapshot.created_at,
            snapshot.closed_at,
            snapshot.fetched_at,
        ],
    )?;
    db.execute("DELETE FROM trail_issue_links WHERE issue_url=?1 AND source='issue'", params![snapshot.url])?;
    for pr_url in &snapshot.closed_by {
        db.execute("INSERT OR IGNORE INTO trail_issue_links(pr_url,issue_url,source) VALUES(?1,?2,'issue')", params![pr_url, snapshot.url])?;
    }
    Ok(())
}

fn json<T: Serialize>(value: &T) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "[]".into())
}

pub fn get(db: &Connection, url: &str) -> Result<Option<IssueSnapshot>, BridgeError> {
    Ok(db
        .query_row(
            "SELECT url,repo_host,repo_owner,repo_name,number,title,state,state_reason,author,labels,body,comments,closed_by,created_at,closed_at,fetched_at
             FROM issue_snapshots WHERE url=?1",
            params![url],
            |row| {
                let comments: serde_json::Value = serde_json::from_str(&row.get::<_, String>(11)?).unwrap_or_default();
                Ok(IssueSnapshot {
                    url: row.get(0)?,
                    repository: GithubRepository { host: row.get(1)?, owner: row.get(2)?, name: row.get(3)? },
                    number: row.get::<_, i64>(4)? as u64,
                    title: row.get(5)?,
                    open: row.get::<_, String>(6)? == "open",
                    state_reason: row.get(7)?,
                    author: row.get(8)?,
                    labels: serde_json::from_str(&row.get::<_, String>(9)?).unwrap_or_default(),
                    body: row.get(10)?,
                    comment_count: comments.get("count").and_then(serde_json::Value::as_u64).unwrap_or_default() as usize,
                    comments: comments.get("last").cloned().and_then(|last| serde_json::from_value(last).ok()).unwrap_or_default(),
                    closed_by: serde_json::from_str(&row.get::<_, String>(12)?).unwrap_or_default(),
                    created_at: row.get(13)?,
                    closed_at: row.get(14)?,
                    fetched_at: row.get(15)?,
                })
            },
        )
        .optional()?)
}

/// What a pull request says it closes, replacing what it said before.
pub fn save_closing(db: &Connection, pr_url: &str, issue_urls: &[String], now: &str) -> Result<(), BridgeError> {
    db.execute("DELETE FROM trail_issue_links WHERE pr_url=?1 AND source='pr'", params![pr_url])?;
    for issue_url in issue_urls {
        // gh answers with canonical URLs; keep the form members use.
        let issue_url = parse_url(issue_url).map(|issue| issue.url()).unwrap_or_else(|| issue_url.clone());
        db.execute("INSERT OR IGNORE INTO trail_issue_links(pr_url,issue_url,source) VALUES(?1,?2,'pr')", params![pr_url, issue_url])?;
    }
    db.execute(
        "INSERT INTO trail_pr_link_reads(pr_url,fetched_at) VALUES(?1,?2) ON CONFLICT(pr_url) DO UPDATE SET fetched_at=excluded.fetched_at",
        params![pr_url, now],
    )?;
    Ok(())
}

// --- reading GitHub, outside the database lock --------------------------------

/// Where `gh` runs for a trail's read: the project's folder, else any
/// project's, else the home folder. `--repo` names the repository, so the
/// folder only has to exist.
pub fn cwd(core: &BridgeCore, workspace_id: Option<&str>) -> PathBuf {
    if let Some(path) = workspace_id.and_then(|id| core.workspace_path(id).ok()) {
        return PathBuf::from(path);
    }
    let any: Option<String> =
        core.db.lock().unwrap().query_row("SELECT path FROM workspaces WHERE path IS NOT NULL ORDER BY created_at DESC LIMIT 1", [], |row| row.get(0)).optional().ok().flatten();
    any.map(PathBuf::from).filter(|path| path.is_dir()).unwrap_or_else(crate::live_turn::user_home)
}

/// The repository `#n` means on a trail: its project's.
pub fn default_repository(core: &BridgeCore, workspace_id: Option<&str>) -> Option<GithubRepository> {
    let path = core.workspace_path(workspace_id?).ok()?;
    core.github_surface.resolve_repository(std::path::Path::new(&path)).ok()
}

pub fn fetch(core: &BridgeCore, issue: &IssueUrl, cwd: &std::path::Path) -> Result<IssueSnapshot, BridgeError> {
    let read = core
        .github_surface
        .read_issue(cwd, &issue.repository, issue.number)
        .map_err(|error| BridgeError::Invalid(format!("GitHub could not read {}: {error}", issue.url())))?;
    IssueSnapshot::from_read(read, &chrono::Utc::now().to_rfc3339())
}

/// The issues a pull request closes, read from GitHub. `None` when the read
/// failed (an older `gh` without the field, offline): nothing is stored then.
pub fn fetch_closing(core: &BridgeCore, pr_url: &str) -> Option<Vec<String>> {
    let pull = crate::session_prs::parse_pull_request_url(pr_url)?;
    let workspace: Option<String> =
        core.db.lock().unwrap().query_row("SELECT workspace_id FROM session_pull_requests WHERE url=?1 LIMIT 1", params![pr_url], |row| row.get(0)).optional().ok().flatten();
    let repository = GithubRepository { host: pull.host.clone(), owner: pull.owner.clone(), name: pull.name.clone() };
    core.github_surface.pr_closing_issues(&cwd(core, workspace.as_deref()), &repository, pull.number).ok()
}

/// What the maintenance loop still has to read: issue members with no
/// snapshot, PR members whose closing references were never read.
pub fn missing(db: &Connection) -> Result<(Vec<(String, Option<String>)>, Vec<String>), BridgeError> {
    let issues = db
        .prepare(
            "SELECT m.ref, t.workspace_id FROM trail_members m JOIN trails t ON t.id=m.trail_id
             WHERE t.archived_at IS NULL AND m.kind=?1 AND NOT EXISTS(SELECT 1 FROM issue_snapshots s WHERE s.url=m.ref) ORDER BY m.added_at",
        )?
        .query_map(params![TrailMemberKind::Issue.as_str()], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<Result<Vec<_>, _>>()?;
    let pulls = db
        .prepare(
            "SELECT m.ref FROM trail_members m JOIN trails t ON t.id=m.trail_id
             WHERE t.archived_at IS NULL AND m.kind=?1 AND NOT EXISTS(SELECT 1 FROM trail_pr_link_reads r WHERE r.pr_url=m.ref) ORDER BY m.added_at",
        )?
        .query_map(params![TrailMemberKind::PullRequest.as_str()], |row| row.get(0))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok((issues, pulls))
}

/// Read what [`missing`] names and store it. Returns whether anything was
/// stored, so the caller can sweep again and tell clients.
pub fn read_missing(core: &Arc<BridgeCore>) -> bool {
    let Ok((issues, pulls)) = missing(&core.db.lock().unwrap()) else { return false };
    read_some(core, &issues, &pulls, true)
}

/// Read every issue member and every PR member's closing references of one
/// trail, for an explicit refresh.
pub fn read_trail(core: &Arc<BridgeCore>, trail_id: &str) -> Result<bool, BridgeError> {
    let (workspace, members) = {
        let db = core.db.lock().unwrap();
        let trail = super::store::require(&db, trail_id)?;
        (trail.workspace_id, super::store::members(&db, trail_id)?)
    };
    let issues: Vec<(String, Option<String>)> =
        members.iter().filter(|member| member.kind == TrailMemberKind::Issue).map(|member| (member.reference.clone(), workspace.clone())).collect();
    let pulls: Vec<String> = members.iter().filter(|member| member.kind == TrailMemberKind::PullRequest).map(|member| member.reference.clone()).collect();
    Ok(read_some(core, &issues, &pulls, false))
}

/// How long a failed background read waits before the loop tries it again.
/// An explicit refresh never waits.
pub const RETRY_AFTER: Duration = Duration::from_secs(600);

/// URLs whose last background read failed, and when.
fn failures() -> &'static Mutex<HashMap<String, Instant>> {
    static FAILURES: OnceLock<Mutex<HashMap<String, Instant>>> = OnceLock::new();
    FAILURES.get_or_init(Default::default)
}

fn read_some(core: &Arc<BridgeCore>, issues: &[(String, Option<String>)], pulls: &[String], background: bool) -> bool {
    let waiting = |url: &str| background && failures().lock().unwrap().get(url).is_some_and(|at| at.elapsed() < RETRY_AFTER);
    let outcome = |url: &str, ok: bool| {
        let mut failed = failures().lock().unwrap();
        if ok {
            failed.remove(url);
        } else {
            failed.insert(url.to_owned(), Instant::now());
        }
    };
    let mut stored = false;
    for (url, workspace) in issues {
        let Some(issue) = parse_url(url) else { continue };
        if waiting(url) {
            continue;
        }
        // A failed read keeps the last snapshot; the page says how old it is.
        match fetch(core, &issue, &cwd(core, workspace.as_deref())) {
            Ok(snapshot) => {
                outcome(url, true);
                stored |= save(&core.db.lock().unwrap(), &snapshot).is_ok();
            }
            Err(error) => {
                outcome(url, false);
                eprintln!("trails: {error}");
            }
        }
    }
    for pr_url in pulls {
        if waiting(pr_url) {
            continue;
        }
        // An older `gh` without the field, or offline: nothing is stored.
        match fetch_closing(core, pr_url) {
            Some(closing) => {
                outcome(pr_url, true);
                stored |= save_closing(&core.db.lock().unwrap(), pr_url, &closing, &chrono::Utc::now().to_rfc3339()).is_ok();
            }
            None => outcome(pr_url, false),
        }
    }
    stored
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::trails::store::tests::database;

    fn repo() -> GithubRepository {
        GithubRepository { host: "github.com".into(), owner: "o".into(), name: "r".into() }
    }

    pub fn snapshot(number: u64, open: bool, reason: Option<&str>, closed_by: &[&str]) -> IssueSnapshot {
        IssueSnapshot {
            url: format!("https://github.com/o/r/issues/{number}"),
            repository: repo(),
            number,
            title: "Archived chats stay searchable".into(),
            open,
            state_reason: reason.map(str::to_owned),
            author: Some("mika".into()),
            labels: vec!["ux".into()],
            body: "Archived chats should stay searchable from ⌘K for 90 days.".into(),
            comments: vec![IssueComment { author: Some("atharva".into()), body: "agreed".into(), created_at: "2026-11-09T10:00:00Z".into() }],
            comment_count: 1,
            closed_by: closed_by.iter().map(|url| url.to_string()).collect(),
            created_at: "2026-11-09T09:00:00Z".into(),
            closed_at: (!open).then(|| "2026-11-13T09:00:00Z".into()),
            fetched_at: "2026-11-13T10:00:00Z".into(),
        }
    }

    #[test]
    fn parses_every_accepted_form() {
        let want = IssueUrl { repository: repo(), number: 41 };
        for raw in [
            "https://github.com/o/r/issues/41",
            "https://github.com/o/r/issues/41/",
            "https://github.com/o/r/issues/41#issuecomment-9",
            "https://github.com/o/r/issues/41?x=1",
            "github.com/o/r/issues/41",
            "  http://GitHub.com/o/r/issues/41  ",
            "o/r#41",
        ] {
            assert_eq!(parse_reference(raw, None).unwrap(), want, "{raw}");
        }
        assert_eq!(parse_reference("#41", Some(&repo())).unwrap(), want);
        assert_eq!(parse_reference("41", Some(&repo())).unwrap(), want);
        assert_eq!(want.url(), "https://github.com/o/r/issues/41");
        let enterprise = parse_reference("https://git.corp.example/team/app/issues/7", None).unwrap();
        assert_eq!(enterprise.url(), "https://git.corp.example/team/app/issues/7");
    }

    #[test]
    fn refuses_pull_requests_and_bare_numbers_without_a_repository() {
        let pull = parse_reference("https://github.com/o/r/pull/12", None).unwrap_err().to_string();
        assert!(pull.contains("pull request"), "{pull}");
        let bare = parse_reference("#41", None).unwrap_err().to_string();
        assert!(bare.contains("no project") && bare.contains("owner/repo#number"), "{bare}");
        for raw in ["", "  ", "fix the bug", "o/r#x", "#0", "o#4", "https://github.com/o/r/issues/abc"] {
            assert!(parse_reference(raw, Some(&repo())).is_err(), "{raw:?}");
        }
    }

    #[test]
    fn snapshot_round_trip_keeps_last_five_comments() {
        let db = database();
        let comments = (0..7)
            .map(|index| crate::github_surface::GithubComment {
                id: format!("c{index}"),
                author: Some(crate::github_surface::GithubActor { login: format!("u{index}") }),
                body: format!("comment {index}"),
                created_at: format!("2026-11-1{index}T00:00:00Z"),
                url: String::new(),
            })
            .collect();
        let read = IssueRead {
            detail: crate::github_surface::IssueDetail {
                summary: crate::github_surface::IssueSummary {
                    number: 41,
                    title: "T".into(),
                    state: IssueState::Closed,
                    author: None,
                    labels: Vec::new(),
                    created_at: "2026-11-01T00:00:00Z".into(),
                    updated_at: "2026-11-02T00:00:00Z".into(),
                    url: "https://github.com/o/r/issues/41".into(),
                },
                body: "x".repeat(BODY_MAX_BYTES + 10),
                comments,
            },
            state_reason: Some("NOT_PLANNED".into()),
            closed_at: Some("2026-11-02T00:00:00Z".into()),
            closed_by: vec!["https://github.com/o/r/pull/12".into()],
        };
        let snapshot = IssueSnapshot::from_read(read, "now").unwrap();
        save(&db, &snapshot).unwrap();
        let back = get(&db, "https://github.com/o/r/issues/41").unwrap().unwrap();
        assert_eq!(back, snapshot);
        assert_eq!(back.comment_count, 7);
        assert_eq!(back.comments.iter().map(|comment| comment.body.as_str()).collect::<Vec<_>>(), ["comment 2", "comment 3", "comment 4", "comment 5", "comment 6"]);
        assert_eq!(back.body.len(), BODY_MAX_BYTES);
        assert_eq!(back.state_words(), "closed as not planned");
        assert!(!back.completed());
    }

    #[test]
    fn links_come_from_both_directions() {
        let db = database();
        save(&db, &snapshot(41, true, None, &["https://github.com/o/r/pull/12"])).unwrap();
        save_closing(&db, "https://github.com/o/r/pull/13", &["https://github.com/o/r/issues/41".into()], "t").unwrap();
        // Idempotent, and a re-read replaces only its own side.
        save(&db, &snapshot(41, true, None, &["https://github.com/o/r/pull/12"])).unwrap();
        save_closing(&db, "https://github.com/o/r/pull/13", &["https://github.com/o/r/issues/41".into()], "t2").unwrap();
        let links = |db: &Connection| -> Vec<(String, String)> {
            db.prepare("SELECT pr_url, source FROM trail_issue_links WHERE issue_url='https://github.com/o/r/issues/41' ORDER BY pr_url")
                .unwrap()
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap()
        };
        assert_eq!(links(&db), [("https://github.com/o/r/pull/12".to_owned(), "issue".to_owned()), ("https://github.com/o/r/pull/13".to_owned(), "pr".to_owned())]);
        save(&db, &snapshot(41, true, None, &[])).unwrap();
        save_closing(&db, "https://github.com/o/r/pull/13", &[], "t3").unwrap();
        assert!(links(&db).is_empty());
        let read_at: String = db.query_row("SELECT fetched_at FROM trail_pr_link_reads WHERE pr_url='https://github.com/o/r/pull/13'", [], |row| row.get(0)).unwrap();
        assert_eq!(read_at, "t3");
    }
}
