//! Durable chat-to-pull-request links with live status reads.
//!
//! A PR opened from a Bridge chat (`gh pr create` in a completed command tool
//! call) becomes a durable artifact of that chat: it survives restarts, keeps
//! updating after the agent's turn ends, and still shows merged or closed
//! states that the open-only PR list drops. Rows are keyed by canonical
//! repository identity, so two sessions in one workspace never share a card,
//! and a URL quoted in prose never attaches — only a verified tool completion
//! or an explicit user attach writes a row.

use chrono::Utc;
use regex::Regex;
use rusqlite::{params, Connection, Transaction};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::{Arc, LazyLock};

use crate::agent::NormalizedEvent;
use crate::events::CoreEvent;
use crate::github_surface::{
    CheckConclusion, CheckRollup, CheckStatus, PullRequestBrief, PullRequestCheck, PullRequestState,
};
use crate::{BridgeCore, BridgeError};

pub const ATTRIBUTION_TOOL_COMPLETION: &str = "toolCompletion";
pub const ATTRIBUTION_MANUAL: &str = "manual";

pub(crate) fn install_store(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS session_pull_requests (
            session_id TEXT NOT NULL,
            workspace_id TEXT NOT NULL,
            repo_host TEXT NOT NULL,
            repo_owner TEXT NOT NULL,
            repo_name TEXT NOT NULL,
            number INTEGER NOT NULL,
            url TEXT NOT NULL,
            title TEXT NOT NULL,
            head_branch TEXT NOT NULL,
            head_sha TEXT NOT NULL,
            attribution TEXT NOT NULL,
            snapshot TEXT,
            fetched_at TEXT,
            created_at TEXT NOT NULL,
            PRIMARY KEY (session_id, repo_host, repo_owner, repo_name, number)
        );
        CREATE INDEX IF NOT EXISTS session_pull_requests_workspace ON session_pull_requests(workspace_id);",
    )?;
    Ok(())
}

#[derive(Debug, Clone)]
pub struct SessionPullRequestRow {
    pub session_id: String,
    pub workspace_id: String,
    pub repo_host: String,
    pub repo_owner: String,
    pub repo_name: String,
    pub number: u64,
    pub url: String,
    pub title: String,
    pub head_branch: String,
    pub head_sha: String,
    pub attribution: String,
    pub snapshot: Option<String>,
    pub fetched_at: Option<String>,
    pub created_at: String,
}

/// Everything the in-chat card renders. Persisted as the per-row snapshot so a
/// transient `gh` failure can show the last known state labelled stale rather
/// than a false green.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionPrView {
    pub number: u64,
    pub title: String,
    pub url: String,
    pub state: PullRequestState,
    pub is_draft: bool,
    pub head_branch: String,
    pub head_sha: String,
    pub checks: CheckRollup,
    pub check_details: Vec<PullRequestCheck>,
    pub attribution: String,
    pub attached_at: String,
    pub fetched_at: Option<String>,
    pub stale: bool,
    pub error: Option<String>,
}

// --- detection: `gh pr create` in a completed command tool call -------------

/// `gh pr create` at a command position: start, or after a shell separator,
/// allowing `FOO=bar` environment prefixes. A mention inside prose, a quoted
/// string, or an argument (`echo gh pr create`) is not a command position.
static PR_CREATE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:^|[;&|(\n]\s*)(?:[A-Za-z_][A-Za-z0-9_]*=\S+\s+)*(?:command\s+|env\s+\S+\s+)*gh(?:\.exe)?\s+pr\s+create(?:\s|$)")
        .expect("pr-create detector compiles")
});

/// The URL `gh` prints on success. Identity segments stop at `/` and never
/// contain whitespace or quotes, which keeps a look-alike string inside a
/// larger blob from over-matching: the regex still only *candidates* — the
/// repository identity check in `attach` is the proof.
static PR_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"https?://([A-Za-z0-9._:-]+)/([^/\s"'<>]+)/([^/\s"'<>]+)/pull/([1-9][0-9]*)"#)
        .expect("pr-url detector compiles")
});

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullRequestUrl {
    pub host: String,
    pub owner: String,
    pub name: String,
    pub number: u64,
}

impl PullRequestUrl {
    fn matches_repository(&self, repository: &crate::github_surface::GithubRepository) -> bool {
        self.host.eq_ignore_ascii_case(&repository.host)
            && self.owner.eq_ignore_ascii_case(&repository.owner)
            && self
                .name
                .trim_end_matches(".git")
                .eq_ignore_ascii_case(repository.name.trim_end_matches(".git"))
    }
}

/// Parse a PR URL into its identity, or `None` when the shape is not a PR URL.
pub fn parse_pull_request_url(raw: &str) -> Option<PullRequestUrl> {
    let captures = PR_URL.captures(raw.trim())?;
    let number = captures.get(4)?.as_str().parse::<u64>().ok()?;
    Some(PullRequestUrl {
        host: captures.get(1)?.as_str().to_owned(),
        owner: captures.get(2)?.as_str().to_owned(),
        name: captures.get(3)?.as_str().to_owned(),
        number,
    })
}

/// Whether the command line invokes `gh pr create` in a command position.
pub fn detect_pr_create(command: &str) -> bool {
    // Codex reports the process invocation, including the login-shell wrapper.
    // Decode only a known shell's -c argument; never execute it or treat an
    // arbitrary quoted argument (e.g. echo "gh pr create") as a command.
    if let Ok(words) = shell_words::split(command) {
        if words.len() == 3
            && matches!(std::path::Path::new(&words[0]).file_name().and_then(|name| name.to_str()), Some("sh" | "bash" | "zsh"))
            && matches!(words[1].as_str(), "-c" | "-lc" | "-cl")
        {
            return PR_CREATE.is_match(&words[2]);
        }
    }
    PR_CREATE.is_match(command)
}

/// Command completion proves creation; Git's own linked-checkout inventory
/// proves the head belongs to this repository even when the agent used a
/// worktree rather than the workspace root.
fn checkout_matches_head(path: &std::path::Path, branch: &str, sha: &str) -> bool {
    crate::git::list_worktrees(path).is_ok_and(|entries| entries.into_iter().any(|entry| {
        !entry.bare && !entry.detached && !entry.prunable && entry.path.is_dir()
            && entry.branch.as_deref() == Some(branch)
            && entry.head.as_deref() == Some(sha)
    }))
}

/// The first PR URL in a command's output, when one is present.
pub fn extract_pull_request_url(output: &str) -> Option<PullRequestUrl> {
    parse_pull_request_url(output)
}

/// The card's headline numbers, computed from the detailed check set so the
/// card and the checks flyout never disagree.
pub fn rollup_for(checks: &[PullRequestCheck]) -> CheckRollup {
    let mut rollup = CheckRollup {
        total: checks.len() as u32,
        ..CheckRollup::default()
    };
    for check in checks {
        match check.status {
            CheckStatus::Queued => rollup.queued += 1,
            CheckStatus::InProgress => rollup.in_progress += 1,
            CheckStatus::Completed => match check.conclusion {
                Some(CheckConclusion::Success) => rollup.passed += 1,
                Some(CheckConclusion::Failure)
                | Some(CheckConclusion::TimedOut)
                | Some(CheckConclusion::StartupFailure)
                | Some(CheckConclusion::ActionRequired) => rollup.failed += 1,
                Some(CheckConclusion::Skipped) | Some(CheckConclusion::Neutral) => {
                    rollup.skipped += 1
                }
                Some(CheckConclusion::Cancelled) | Some(CheckConclusion::Stale) => {
                    rollup.cancelled += 1
                }
                None => rollup.queued += 1,
            },
        }
    }
    rollup
}

// --- attach ------------------------------------------------------------------

/// What the user (or the detector) pointed at: a full URL, or a bare number
/// interpreted against the repository the workspace resolves to.
pub enum AttachReference {
    Url(PullRequestUrl),
    Number(u64),
}

pub fn parse_attach_reference(raw: &str) -> Result<AttachReference, BridgeError> {
    let trimmed = raw.trim().trim_end_matches('/');
    if let Some(url) = parse_pull_request_url(trimmed) {
        return Ok(AttachReference::Url(url));
    }
    let digits = trimmed.strip_prefix('#').unwrap_or(trimmed);
    if !digits.is_empty() && digits.chars().all(|character| character.is_ascii_digit()) {
        if let Ok(number) = digits.parse::<u64>() {
            if number > 0 {
                return Ok(AttachReference::Number(number));
            }
        }
    }
    Err(BridgeError::Invalid(
        "Paste a pull request URL like https://github.com/owner/repo/pull/123, or a PR number."
            .into(),
    ))
}

/// Attach a chat to a verified pull request. Verification is the whole point:
/// the workspace's own Git configuration names the repository, a URL must
/// agree with it, and a tool-completion attach additionally requires the PR
/// head to match a checkout in this workspace's Git worktree inventory. The
/// session that ran `gh pr create` proves ownership; prose never does.
pub fn attach(
    core: &Arc<BridgeCore>,
    session_id: &str,
    reference: &AttachReference,
    attribution: &str,
) -> Result<SessionPrView, BridgeError> {
    let workspace_id: Option<String> = {
        let db = core.db.lock().unwrap();
        db.query_row(
            "SELECT workspace_id FROM sessions WHERE id=?1",
            params![session_id],
            |row| row.get(0),
        )?
    };
    let workspace_id = workspace_id.ok_or_else(|| {
        BridgeError::Invalid("This chat has no workspace to attach a pull request to.".into())
    })?;
    let path = PathBuf::from(core.workspace_path(&workspace_id)?);
    let repository = core
        .github_surface
        .resolve_repository(&path)
        .map_err(|error| BridgeError::Invalid(error.to_string()))?;
    let number = match reference {
        AttachReference::Url(url) => {
            if !url.matches_repository(&repository) {
                return Err(BridgeError::Invalid(format!(
                    "That pull request belongs to {}/{}/{}; this workspace resolves to {}.",
                    url.host,
                    url.owner,
                    url.name,
                    repository.selector()
                )));
            }
            url.number
        }
        AttachReference::Number(number) => *number,
    };
    core.github_surface.invalidate_brief(&path, number);
    let brief = core
        .github_surface
        .pr_brief(&path, number)
        .map_err(|error| BridgeError::Invalid(format!("Could not read PR #{number}: {error}")))?;
    if attribution == ATTRIBUTION_TOOL_COMPLETION {
        if !checkout_matches_head(&path, &brief.head_branch, &brief.head_sha) {
            return Err(BridgeError::Invalid(
                "The PR head does not match this workspace's checkout.".into(),
            ));
        }
    }
    upsert(
        &core.db.lock().unwrap(),
        session_id,
        &workspace_id,
        &brief,
        attribution,
    )?;
    core.events.publish(CoreEvent::GithubSessionPrsChanged {
        session_id: session_id.to_owned(),
    });
    core.github_poller.watch_attached(
        &workspace_id,
        path.clone(),
        number,
        &brief.head_branch,
        &brief.title,
    );
    let row = list(&core.db.lock().unwrap(), session_id)?.into_iter()
        .find(|row| row.number == number && row.repo_host == brief.repository.host
            && row.repo_owner == brief.repository.owner && row.repo_name == brief.repository.name)
        .ok_or_else(|| BridgeError::Invalid("The attached pull request could not be read.".into()))?;
    Ok(read_view(core, &path, &row, &brief))
}

fn build_view(
    brief: &PullRequestBrief,
    checks: Vec<PullRequestCheck>,
    attribution: &str,
    attached_at: &str,
) -> SessionPrView {
    SessionPrView {
        number: brief.number,
        title: brief.title.clone(),
        url: brief.url.clone(),
        state: brief.state,
        is_draft: brief.is_draft,
        head_branch: brief.head_branch.clone(),
        head_sha: brief.head_sha.clone(),
        checks: rollup_for(&checks),
        check_details: checks,
        attribution: attribution.to_owned(),
        attached_at: attached_at.to_owned(),
        fetched_at: Some(Utc::now().to_rfc3339()),
        stale: false,
        error: None,
    }
}

fn upsert(
    db: &Connection,
    session_id: &str,
    workspace_id: &str,
    brief: &PullRequestBrief,
    attribution: &str,
) -> Result<(), BridgeError> {
    let repository_host_owner_name = brief_repository(brief);
    db.execute(
        "INSERT INTO session_pull_requests(session_id,workspace_id,repo_host,repo_owner,repo_name,number,url,title,head_branch,head_sha,attribution,created_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)
         ON CONFLICT(session_id,repo_host,repo_owner,repo_name,number)
         DO UPDATE SET workspace_id=excluded.workspace_id,url=excluded.url,title=excluded.title,
           head_branch=excluded.head_branch,head_sha=excluded.head_sha,attribution=excluded.attribution",
        params![
            session_id,
            workspace_id,
            repository_host_owner_name.0,
            repository_host_owner_name.1,
            repository_host_owner_name.2,
            brief.number,
            brief.url,
            brief.title,
            brief.head_branch,
            brief.head_sha,
            attribution,
            Utc::now().to_rfc3339(),
        ],
    )?;
    Ok(())
}

fn brief_repository(brief: &PullRequestBrief) -> (String, String, String) {
    (
        brief.repository.host.clone(),
        brief.repository.owner.clone(),
        brief.repository.name.clone(),
    )
}

pub fn list(db: &Connection, session_id: &str) -> Result<Vec<SessionPullRequestRow>, BridgeError> {
    let mut statement = db.prepare(
        "SELECT session_id,workspace_id,repo_host,repo_owner,repo_name,number,url,title,head_branch,head_sha,attribution,snapshot,fetched_at,created_at
         FROM session_pull_requests WHERE session_id=?1 ORDER BY created_at DESC, rowid DESC",
    )?;
    let rows = statement
        .query_map(params![session_id], |row| {
            Ok(SessionPullRequestRow {
                session_id: row.get(0)?,
                workspace_id: row.get(1)?,
                repo_host: row.get(2)?,
                repo_owner: row.get(3)?,
                repo_name: row.get(4)?,
                number: row.get::<_, i64>(5)? as u64,
                url: row.get(6)?,
                title: row.get(7)?,
                head_branch: row.get(8)?,
                head_sha: row.get(9)?,
                attribution: row.get(10)?,
                snapshot: row.get(11)?,
                fetched_at: row.get(12)?,
                created_at: row.get(13)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

fn record_snapshot(
    db: &Connection,
    session_id: &str,
    row: &SessionPullRequestRow,
    view: &SessionPrView,
) -> Result<(), BridgeError> {
    let snapshot = serde_json::to_string(view)
        .map_err(|error| BridgeError::Invalid(format!("snapshot encode failed: {error}")))?;
    db.execute(
        "UPDATE session_pull_requests SET snapshot=?6,fetched_at=?7,title=?8,head_branch=?9,head_sha=?10
         WHERE session_id=?1 AND repo_host=?2 AND repo_owner=?3 AND repo_name=?4 AND number=?5",
        params![
            session_id,
            row.repo_host,
            row.repo_owner,
            row.repo_name,
            row.number,
            snapshot,
            view.fetched_at,
            view.title,
            view.head_branch,
            view.head_sha,
        ],
    )?;
    Ok(())
}

fn read_checks(
    core: &Arc<BridgeCore>,
    path: &std::path::Path,
    number: u64,
) -> Result<Vec<PullRequestCheck>, crate::github_surface::GithubSurfaceError> {
    core.github_surface.pr_checks(path, number)
}

/// The read behind `github/github_session_prs`: every attached row with its
/// freshest status, newest first. A row whose live read fails keeps its last
/// good snapshot, labelled stale with a retry affordance — never a false
/// green. Reading also (re)arms the poller, so reopening a settled chat
/// resumes background watching without a second loop.
pub fn session_pull_requests(
    core: &Arc<BridgeCore>,
    session_id: &str,
    refresh: bool,
) -> Result<Vec<SessionPrView>, BridgeError> {
    let rows = list(&core.db.lock().unwrap(), session_id)?;
    let mut views = Vec::with_capacity(rows.len());
    for row in rows {
        let path = match core.workspace_path(&row.workspace_id) {
            Ok(path) => PathBuf::from(path),
            Err(error) => {
                views.push(stale_view(&row, &error.to_string()));
                continue;
            }
        };
        if refresh {
            core.github_surface.invalidate_brief(&path, row.number);
            core.github_surface.invalidate_checks(&path, row.number);
        }
        match core.github_surface.pr_brief(&path, row.number) {
            Ok(brief) => {
                core.github_poller.watch_attached(
                    &row.workspace_id,
                    path.clone(),
                    row.number,
                    &brief.head_branch,
                    &brief.title,
                );
                let view = read_view(core, &path, &row, &brief);
                views.push(view);
            }
            Err(error) => {
                views.push(stale_view(&row, &error.to_string()));
            }
        }
    }
    Ok(views)
}

/// Both attachment and refresh preserve the last successful full snapshot.
/// On the first read there is no check history, but the verified brief still
/// supplies the real PR state and draft flag instead of guessing "open".
fn read_view(core: &Arc<BridgeCore>, path: &std::path::Path, row: &SessionPullRequestRow, brief: &PullRequestBrief) -> SessionPrView {
    match read_checks(core, path, row.number) {
        Ok(checks) => {
            let view = build_view(brief, checks, &row.attribution, &row.created_at);
            let _ = record_snapshot(&core.db.lock().unwrap(), &row.session_id, row, &view);
            view
        }
        Err(error) => {
            if row.snapshot.is_some() {
                return stale_view(row, &error.to_string());
            }
            let mut view = build_view(brief, Vec::new(), &row.attribution, &row.created_at);
            view.stale = true;
            view.error = Some(error.to_string());
            view.fetched_at = None;
            view
        }
    }
}

fn stale_view(row: &SessionPullRequestRow, error: &str) -> SessionPrView {
    if let Some(snapshot) = &row.snapshot {
        if let Ok(mut view) = serde_json::from_str::<SessionPrView>(snapshot) {
            view.stale = true;
            view.error = Some(error.to_owned());
            view.attached_at = row.created_at.clone();
            return view;
        }
    }
    // No successful read yet: the row's own columns are the honest minimum.
    SessionPrView {
        number: row.number,
        title: row.title.clone(),
        url: row.url.clone(),
        state: PullRequestState::Open,
        is_draft: false,
        head_branch: row.head_branch.clone(),
        head_sha: row.head_sha.clone(),
        checks: CheckRollup::default(),
        check_details: Vec::new(),
        attribution: row.attribution.clone(),
        attached_at: row.created_at.clone(),
        fetched_at: None,
        stale: true,
        error: Some(error.to_owned()),
    }
}

// --- the live-turn hook --------------------------------------------------------

/// Called for every completed command tool event. Extraction is cheap and
/// synchronous; the GitHub verification is detached so a slow or offline `gh`
/// never stalls the turn loop (which holds the database mutex).
pub fn detect_pull_request_creation(
    core: &Arc<BridgeCore>,
    session_id: &str,
    event: &NormalizedEvent,
) {
    let Some(candidate) = creation_candidate(event) else { return };
    let core = Arc::clone(core);
    let session_id = session_id.to_owned();
    std::thread::spawn(move || {
        // A rejection here is silent by design: detection is a heuristic
        // sitting on untrusted output, and the explicit Attach PR action is
        // the user's path when capture misses.
        let _ = attach(
            &core,
            &session_id,
            &AttachReference::Url(candidate),
            ATTRIBUTION_TOOL_COMPLETION,
        );
    });
}

fn creation_candidate(event: &NormalizedEvent) -> Option<PullRequestUrl> {
    if event.kind != "command.completed" || event.status.as_deref().is_some_and(|status| status != "completed") {
        return None;
    }
    let command = event
        .data
        .get("command")
        .or_else(|| event.data.pointer("/state/input/command"))
        .and_then(serde_json::Value::as_str);
    let Some(command) = command else { return None };
    if !detect_pr_create(command) {
        return None;
    }
    let output = [
        event.data.get("aggregatedOutput"),
        event.data.get("output"),
        event.data.pointer("/state/output"),
    ]
    .into_iter()
    .flatten()
    .filter_map(serde_json::Value::as_str)
    .collect::<Vec<_>>()
    .join("\n");
    let output = if output.is_empty() {
        event.text.clone().unwrap_or_default()
    } else {
        output
    };
    extract_pull_request_url(&output)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::github_surface::{CheckConclusion, CheckStatus};

    fn git(path: &std::path::Path, args: &[&str]) {
        let result = std::process::Command::new("git").current_dir(path).args(args)
            .env("GIT_AUTHOR_NAME", "Test").env("GIT_AUTHOR_EMAIL", "test@example.com")
            .env("GIT_COMMITTER_NAME", "Test").env("GIT_COMMITTER_EMAIL", "test@example.com")
            .output().unwrap();
        assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    }

    // A real Git checkout plus a deterministic gh process. The marker lets a
    // successful brief read coexist with a transient checks-only outage.
    pub(crate) fn fixture() -> (tempfile::TempDir, Arc<BridgeCore>, PathBuf) {
        use std::os::unix::fs::PermissionsExt;
        let scratch = tempfile::tempdir().unwrap();
        let repo = scratch.path().join("repo");
        std::fs::create_dir(&repo).unwrap();
        git(&repo, &["init", "-b", "main"]);
        git(&repo, &["commit", "--allow-empty", "-m", "fixture"]);
        git(&repo, &["remote", "add", "origin", "https://github.com/o/r.git"]);
        let linked = scratch.path().join("linked");
        git(&repo, &["worktree", "add", "-b", "feat/pr", linked.to_str().unwrap()]);
        git(&linked, &["commit", "--allow-empty", "-m", "worktree change"]);
        let sha = crate::git::head_commit(&linked).unwrap();
        let brief = serde_json::json!({"number":12,"title":"PR","state":"OPEN","headRefName":"feat/pr","headRefOid":sha,"url":"https://github.com/o/r/pull/12"});
        std::fs::write(scratch.path().join("brief.json"), brief.to_string()).unwrap();
        std::fs::write(scratch.path().join("checks.json"), r#"[{"name":"test","state":"SUCCESS","bucket":"pass","link":"","workflow":"CI"}]"#).unwrap();
        let binary = scratch.path().join("gh");
        std::fs::write(&binary, r#"#!/bin/sh
root=$(dirname "$0")
case "$1 $2" in
  "auth status") exit 0 ;;
  "pr view") cat "$root/brief.json" ;;
  "pr checks")
    if [ -f "$root/offline" ]; then echo offline >&2; exit 1; fi
    cat "$root/checks.json" ;;
  *) exit 1 ;;
esac
"#).unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut core = BridgeCore::for_tests(scratch.path());
        core.github_surface = crate::github_surface::GithubSurface::discover_on_path(scratch.path());
        {
            let db = core.db.lock().unwrap();
            db.execute("INSERT INTO projects(id,name,path,created_at) VALUES('p','P',?1,'now')", params![repo.to_str().unwrap()]).unwrap();
            db.execute("INSERT INTO workspaces(id,project_id,city,title,branch,path,status,created_at) VALUES('w','p','Kyoto','W','main',?1,'idle','now')", params![repo.to_str().unwrap()]).unwrap();
            db.execute("INSERT INTO sessions(id,workspace_id,harness,label,status) VALUES('s','w','codex','S','idle')", []).unwrap();
        }
        (scratch, Arc::new(core), repo)
    }

    #[test]
    fn tool_completion_attaches_a_verified_linked_worktree_head() {
        let (_scratch, core, repo) = fixture();
        let linked_head = crate::git::head_commit(&repo.parent().unwrap().join("linked")).unwrap();
        assert_ne!(crate::git::head_commit(&repo).unwrap(), linked_head);
        assert!(checkout_matches_head(&repo, "feat/pr", &linked_head));
        assert!(!checkout_matches_head(&repo, "feat/pr", "wrong-sha"));
        assert!(!checkout_matches_head(&repo, "unrelated", &linked_head));
        let view = attach(&core, "s", &AttachReference::Number(12), ATTRIBUTION_TOOL_COMPLETION).unwrap();
        assert_eq!(view.number, 12);
        assert!(!view.stale);
        assert_eq!(view.checks.passed, 1);
        assert_eq!(list(&core.db.lock().unwrap(), "s").unwrap().len(), 1);
        assert!(attach(&core, "s", &AttachReference::Url(PullRequestUrl { host: "github.com".into(), owner: "other".into(), name: "r".into(), number: 12 }), ATTRIBUTION_TOOL_COMPLETION).is_err());
    }

    #[test]
    fn tool_completion_rejects_an_unverified_sha_without_persisting_a_card() {
        let (scratch, core, _) = fixture();
        let path = scratch.path().join("brief.json");
        let mut brief: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        brief["headRefOid"] = serde_json::json!("unrelated-sha");
        std::fs::write(path, brief.to_string()).unwrap();
        assert!(attach(&core, "s", &AttachReference::Number(12), ATTRIBUTION_TOOL_COMPLETION).is_err());
        assert!(list(&core.db.lock().unwrap(), "s").unwrap().is_empty());
    }

    #[test]
    fn checks_only_outage_keeps_the_good_snapshot_until_recovery() {
        let (scratch, core, _) = fixture();
        let original = attach(&core, "s", &AttachReference::Number(12), ATTRIBUTION_TOOL_COMPLETION).unwrap();
        std::fs::write(scratch.path().join("offline"), "").unwrap();
        let stale = session_pull_requests(&core, "s", true).unwrap().remove(0);
        assert!(stale.stale);
        assert_eq!(stale.check_details, original.check_details);
        assert_eq!(stale.fetched_at, original.fetched_at);
        assert!(stale.error.unwrap().contains("offline"));
        std::fs::remove_file(scratch.path().join("offline")).unwrap();
        assert!(!session_pull_requests(&core, "s", true).unwrap()[0].stale);
    }

    #[test]
    fn first_attach_with_a_checks_outage_is_stale_and_durable() {
        let (scratch, core, _) = fixture();
        std::fs::write(scratch.path().join("offline"), "").unwrap();
        let view = attach(&core, "s", &AttachReference::Number(12), ATTRIBUTION_TOOL_COMPLETION).unwrap();
        assert!(view.stale);
        assert!(view.fetched_at.is_none());
        assert_eq!(list(&core.db.lock().unwrap(), "s").unwrap().len(), 1);
    }

    #[test]
    fn completed_provider_events_pair_creation_commands_with_output_only() {
        let mut codex = NormalizedEvent::new("command.completed");
        codex.status = Some("completed".into());
        codex.data = serde_json::json!({"command":"/bin/zsh -lc 'gh pr create --fill'", "aggregatedOutput":"https://github.com/o/r/pull/12"});
        assert_eq!(creation_candidate(&codex).unwrap().number, 12);
        let mut opencode = codex.clone();
        opencode.data = serde_json::json!({"state":{"input":{"command":"gh pr create --fill"},"output":"https://github.com/o/r/pull/12"}});
        assert_eq!(creation_candidate(&opencode).unwrap().number, 12);
        let mut claude = codex.clone();
        claude.data = serde_json::json!({"command":"gh pr create --fill"});
        claude.text = Some("https://github.com/o/r/pull/12".into());
        assert_eq!(creation_candidate(&claude).unwrap().number, 12);
        codex.status = Some("failed".into());
        assert!(creation_candidate(&codex).is_none());
        claude.kind = "assistant.message".into();
        assert!(creation_candidate(&claude).is_none());
    }

    #[test]
    fn shell_wrapped_creation_is_detected_without_matching_echo() {
        assert!(detect_pr_create("/bin/zsh -lc 'gh pr create --fill'"));
        assert!(detect_pr_create("/bin/bash -c \"cd repo && gh pr create --fill\""));
        assert!(!detect_pr_create("/bin/zsh -lc 'echo gh pr create'"));
        assert!(!detect_pr_create("echo \"gh pr create\""));
    }

    #[test]
    fn the_migration_installs_the_link_table() {
        let mut db = Connection::open_in_memory().unwrap();
        db.execute_batch(
            "CREATE TABLE schema_version(version INTEGER PRIMARY KEY, applied_at TEXT);",
        )
        .unwrap();
        let transaction = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).unwrap();
        install_store(&transaction).unwrap();
        transaction.commit().unwrap();
        db.execute(
            "INSERT INTO session_pull_requests(session_id,workspace_id,repo_host,repo_owner,repo_name,number,url,title,head_branch,head_sha,attribution,created_at)
             VALUES('s','w','github.com','bridge','harness',124,'https://github.com/bridge/harness/pull/124','Fix linking','feat/x','abc123','toolCompletion','now')",
            [],
        )
        .unwrap();
        let rows = list(&db, "s").unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].number, 124);
        assert_eq!(rows[0].repo_owner, "bridge");
        assert!(list(&db, "other").unwrap().is_empty(), "sessions are isolated");
    }

    #[test]
    fn a_duplicate_attach_updates_the_existing_row() {
        let mut db = Connection::open_in_memory().unwrap();
        let transaction = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).unwrap();
        install_store(&transaction).unwrap();
        transaction.commit().unwrap();
        for title in ["first title", "renamed"] {
            db.execute(
                "INSERT INTO session_pull_requests(session_id,workspace_id,repo_host,repo_owner,repo_name,number,url,title,head_branch,head_sha,attribution,created_at)
                 VALUES('s','w','github.com','bridge','harness',124,'u',?1,'feat/x','abc','manual','now')
                 ON CONFLICT(session_id,repo_host,repo_owner,repo_name,number) DO UPDATE SET title=excluded.title",
                params![title],
            )
            .unwrap();
        }
        let rows = list(&db, "s").unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].title, "renamed");
    }

    #[test]
    fn rows_from_two_sessions_never_mix() {
        let mut db = Connection::open_in_memory().unwrap();
        let transaction = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).unwrap();
        install_store(&transaction).unwrap();
        transaction.commit().unwrap();
        for session in ["a", "b"] {
            db.execute(
                "INSERT INTO session_pull_requests(session_id,workspace_id,repo_host,repo_owner,repo_name,number,url,title,head_branch,head_sha,attribution,created_at)
                 VALUES(?1,'w','github.com','bridge','harness',124,'u','t','feat/x','abc','manual','now')",
                params![session],
            )
            .unwrap();
        }
        // Same workspace, same PR, two sessions: each keeps its own card.
        assert_eq!(list(&db, "a").unwrap().len(), 1);
        assert_eq!(list(&db, "b").unwrap().len(), 1);
        db.execute(
            "DELETE FROM session_pull_requests WHERE session_id='a'",
            [],
        )
        .unwrap();
        assert_eq!(list(&db, "b").unwrap().len(), 1);
    }

    #[test]
    fn pr_create_is_detected_in_command_positions_only() {
        assert!(detect_pr_create("gh pr create --title \"x\" --body \"y\""));
        assert!(detect_pr_create("gh pr create"));
        assert!(detect_pr_create("cd repo && gh pr create --fill"));
        assert!(detect_pr_create("GITHUB_TOKEN=x gh pr create --fill"));
        assert!(detect_pr_create("git push -u origin head && gh pr create --fill"));
        assert!(!detect_pr_create("echo gh pr create"));
        assert!(!detect_pr_create("gh pr checkout 12"));
        assert!(!detect_pr_create("ghq pr create"));
        assert!(!detect_pr_create("# run gh pr create after push"));
        assert!(!detect_pr_create("agh pr create"));
    }

    #[test]
    fn pr_urls_parse_with_their_identity() {
        let url = parse_pull_request_url("https://github.com/bridge/harness/pull/124").unwrap();
        assert_eq!(url.host, "github.com");
        assert_eq!(url.owner, "bridge");
        assert_eq!(url.name, "harness");
        assert_eq!(url.number, 124);
        let enterprise = parse_pull_request_url("https://git.corp.example/Team/Some_Repo/pull/7").unwrap();
        assert_eq!(enterprise.host, "git.corp.example");
        assert_eq!(enterprise.name, "Some_Repo");
        assert!(parse_pull_request_url("https://github.com/bridge/harness/issues/124").is_none());
        assert!(parse_pull_request_url("https://github.com/bridge/harness/pull/0").is_none());
        assert!(parse_pull_request_url("not a url").is_none());
    }

    #[test]
    fn the_first_pr_url_in_output_is_the_candidate() {
        let output = "some log line\nhttps://github.com/bridge/harness/pull/124\ntrailing";
        assert_eq!(extract_pull_request_url(output).unwrap().number, 124);
        assert!(extract_pull_request_url("no link here").is_none());
        // A URL in quoted prose still only *candidates*; verification decides.
        let quoted = "he said \"https://github.com/o/r/pull/9\" loudly";
        assert_eq!(extract_pull_request_url(quoted).unwrap().number, 9);
    }

    #[test]
    fn attach_references_take_urls_or_numbers() {
        assert!(matches!(
            parse_attach_reference("https://github.com/o/r/pull/12").unwrap(),
            AttachReference::Url(PullRequestUrl { number: 12, .. })
        ));
        assert!(matches!(parse_attach_reference("341").unwrap(), AttachReference::Number(341)));
        assert!(matches!(parse_attach_reference("#341").unwrap(), AttachReference::Number(341)));
        assert!(parse_attach_reference("").is_err());
        assert!(parse_attach_reference("abc").is_err());
        assert!(parse_attach_reference("0").is_err());
    }

    #[test]
    fn rollups_count_each_outcome_once() {
        let check = |status: CheckStatus, conclusion: Option<CheckConclusion>| PullRequestCheck {
            name: "c".into(),
            status,
            conclusion,
            log_url: String::new(),
            workflow: "ci".into(),
        };
        let rollup = rollup_for(&[
            check(CheckStatus::Queued, None),
            check(CheckStatus::InProgress, None),
            check(CheckStatus::Completed, Some(CheckConclusion::Success)),
            check(CheckStatus::Completed, Some(CheckConclusion::Failure)),
            check(CheckStatus::Completed, Some(CheckConclusion::TimedOut)),
            check(CheckStatus::Completed, Some(CheckConclusion::Skipped)),
            check(CheckStatus::Completed, Some(CheckConclusion::Cancelled)),
            check(CheckStatus::Completed, Some(CheckConclusion::StartupFailure)),
        ]);
        assert_eq!(rollup.total, 8);
        assert_eq!(rollup.queued, 1);
        assert_eq!(rollup.in_progress, 1);
        assert_eq!(rollup.passed, 1);
        assert_eq!(rollup.failed, 3);
        assert_eq!(rollup.skipped, 1);
        assert_eq!(rollup.cancelled, 1);
        assert_eq!(rollup_for(&[]).total, 0, "zero checks is not a failure");
    }

    #[test]
    fn a_stale_view_prefers_the_last_good_snapshot() {
        let row = SessionPullRequestRow {
            session_id: "s".into(),
            workspace_id: "w".into(),
            repo_host: "github.com".into(),
            repo_owner: "o".into(),
            repo_name: "r".into(),
            number: 9,
            url: "u".into(),
            title: "stored title".into(),
            head_branch: "feat/x".into(),
            head_sha: "abc".into(),
            attribution: "manual".into(),
            snapshot: None,
            fetched_at: None,
            created_at: "then".into(),
        };
        let fallback = stale_view(&row, "gh is offline");
        assert!(fallback.stale);
        assert_eq!(fallback.title, "stored title");
        assert_eq!(fallback.fetched_at, None);
        assert_eq!(fallback.error.as_deref(), Some("gh is offline"));

        let good = SessionPrView {
            number: 9,
            title: "live title".into(),
            url: "u".into(),
            state: PullRequestState::Merged,
            is_draft: false,
            head_branch: "feat/x".into(),
            head_sha: "abc".into(),
            checks: CheckRollup { total: 2, passed: 2, ..CheckRollup::default() },
            check_details: Vec::new(),
            attribution: "manual".into(),
            attached_at: "then".into(),
            fetched_at: Some("earlier".into()),
            stale: false,
            error: None,
        };
        let mut snapshotted = row.clone();
        snapshotted.snapshot = Some(serde_json::to_string(&good).unwrap());
        let stale = stale_view(&snapshotted, "gh is offline");
        assert!(stale.stale);
        assert_eq!(stale.state, PullRequestState::Merged, "a merged PR stays merged through an outage");
        assert_eq!(stale.checks.passed, 2, "last known checks survive, labelled stale");
        assert_eq!(stale.fetched_at.as_deref(), Some("earlier"));
    }
}
