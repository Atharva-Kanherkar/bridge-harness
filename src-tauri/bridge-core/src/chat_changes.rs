//! What one chat changed, as opposed to everything uncommitted in its folder.
//!
//! A checkout is shared: other chats, other agents, the person's own editor,
//! and weeks of leftovers all sit in the same `git status`. Counting all of it
//! under a chat turned a three-file turn into "1817 files". So Bridge
//! snapshots the working tree at every turn boundary and scopes the chat's
//! changes to the paths its own turns touched.
//!
//! **Snapshots** are trees written through a private index
//! (`GIT_INDEX_FILE`), so the person's index, branch, and working tree are
//! never touched. Each one is committed onto a per-chat chain under
//! `refs/bridge/chats/<session>`, which keeps the objects alive and makes the
//! chain itself the record: a commit's message carries the phase, the turn,
//! and, for a turn's end, the paths that turn changed.
//!
//! **A chat's changes** are the diff from its first snapshot to a live one,
//! kept to the union of the paths its turns changed (plus, while a turn is
//! running, what changed since that turn started). Edits made between turns
//! by someone else stay out unless they land on a file the chat also touched.
//!
//! This is the scheme T3 Code and opencode use. Its limit is the same too:
//! another writer editing the folder *during* a turn is indistinguishable
//! from the turn itself.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::git::{self, WorkspaceChangeset};
use crate::BridgeError;

const REF_PREFIX: &str = "refs/bridge/chats/";
/// A snapshot that takes longer than this is abandoned: the chat's scope
/// reads as unavailable rather than wrong, and the folder view still works.
const SNAPSHOT_TIMEOUT: Duration = Duration::from_secs(30);
/// Untracked files larger than this are left out of snapshots, so a stray
/// dataset or build artifact is never copied into the object store.
const MAX_SNAPSHOT_FILE_BYTES: u64 = 20 * 1024 * 1024;
/// Bound on the untracked listing read before a snapshot.
const MAX_UNTRACKED_LISTING_BYTES: usize = 8 * 1024 * 1024;
/// Bound on the chain read back for a query.
const MAX_CHAIN_BYTES: usize = 16 * 1024 * 1024;
const SNAPSHOT_IDENTITY: [(&str, &str); 4] = [
    ("GIT_AUTHOR_NAME", "Bridge"),
    ("GIT_AUTHOR_EMAIL", "bridge@localhost"),
    ("GIT_COMMITTER_NAME", "Bridge"),
    ("GIT_COMMITTER_EMAIL", "bridge@localhost"),
];

/// Snapshots share a private index per chat, and a turn's start and end must
/// land in order. One lock for all of them: a snapshot with a warm index
/// takes milliseconds, and serialising them is simpler than proving two
/// never touch the same index.
static SNAPSHOTS: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Start,
    End,
}

/// What a snapshot commit's message records.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Record {
    phase: Phase,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    turn: Option<String>,
    /// The paths this turn changed, both sides of a rename. Only on `End`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    paths: Vec<String>,
}

#[derive(Debug, Clone)]
struct Snapshot {
    tree: String,
    committed_at: String,
    record: Record,
}

/// Why a chat has no scoped view yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionChangesUnavailable {
    /// The chat has no folder, or its folder is gone.
    NoFolder,
    /// The folder is not a Git repository.
    NotGit,
    /// No turn has started since Bridge began recording, so there is no
    /// baseline to measure from.
    NoTurns,
    /// The live snapshot failed or ran past its time budget.
    SnapshotFailed,
}

/// `workspaces/session_changes`' result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionChanges {
    pub available: bool,
    pub unavailable_reason: Option<SessionChangesUnavailable>,
    /// Turns recorded for this chat.
    pub turns: usize,
    /// When the chat's baseline was taken (RFC 3339).
    pub since: Option<String>,
    /// Line totals across every changed file, including any past the
    /// bounded `changes.files` window.
    pub additions: i64,
    pub deletions: i64,
    /// The chat's changeset, in the same shape as the folder's.
    pub changes: Option<WorkspaceChangeset>,
}

impl SessionChanges {
    pub fn unavailable(reason: SessionChangesUnavailable) -> Self {
        Self { available: false, unavailable_reason: Some(reason), turns: 0, since: None, additions: 0, deletions: 0, changes: None }
    }
}

/// A turn boundary to snapshot, off the thread that saw it.
pub struct Job {
    pub repo: PathBuf,
    pub session_id: String,
    pub phase: Phase,
    pub turn: Option<String>,
}

/// Snapshot a turn boundary in the background. The event handler that sees
/// `turn.started` holds the database lock; a `git add` under it would stall
/// every chat. One worker drains the queue in order, so a short turn's start
/// can never land after its end.
pub fn enqueue(job: Job) {
    static QUEUE: OnceLock<Mutex<Sender<Job>>> = OnceLock::new();
    let sender = QUEUE.get_or_init(|| {
        let (sender, receiver) = mpsc::channel::<Job>();
        let worker = std::thread::Builder::new()
            .name("bridge-chat-snapshots".into())
            .spawn(move || {
                for job in receiver {
                    if let Err(error) = record(&job.repo, &job.session_id, job.phase, job.turn.as_deref()) {
                        eprintln!("[bridge] chat snapshot for {} failed: {error}", job.session_id);
                    }
                }
            });
        // Without the worker every send below fails and chats fall back to
        // the folder view; nothing else depends on it.
        if let Err(error) = worker {
            eprintln!("[bridge] chat snapshots are off: {error}");
        }
        Mutex::new(sender)
    });
    let queued = sender.lock().unwrap_or_else(std::sync::PoisonError::into_inner).send(job);
    if let Err(unsent) = queued {
        eprintln!("[bridge] chat snapshot for {} skipped: no snapshot worker", unsent.0.session_id);
    }
}

/// Queue a snapshot of `session_id`'s folder for a turn boundary the event
/// handler just saw. Reads the folder through the caller's connection (the
/// handler already holds the lock) and does nothing for a chat without one.
pub fn enqueue_for_session(db: &rusqlite::Connection, session_id: &str, phase: Phase, turn: Option<&str>) {
    let root: Option<String> = db
        .query_row(
            "SELECT COALESCE(s.cwd, w.path) FROM sessions s LEFT JOIN workspaces w ON w.id=s.workspace_id WHERE s.id=?1",
            rusqlite::params![session_id],
            |row| row.get(0),
        )
        .ok()
        .flatten();
    let Some(repo) = root.map(PathBuf::from).filter(|path| path.is_dir()) else { return };
    enqueue(Job { repo, session_id: session_id.to_owned(), phase, turn: turn.map(str::to_owned) });
}

/// Record a turn boundary for `session_id` in `repo`. A folder that is not a
/// repository is skipped silently: there is nothing to scope.
pub fn record(repo: &Path, session_id: &str, phase: Phase, turn: Option<&str>) -> Result<(), BridgeError> {
    if !git::is_repository(repo) {
        return Ok(());
    }
    let reference = ref_name(session_id);
    let _guard = SNAPSHOTS.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let head = rev_parse(repo, &reference);
    let tree = write_tree(repo, session_id)?;
    let paths = match (phase, &head) {
        // The chain's head is this turn's start (or, when a harness reported
        // no start, the previous turn's end): what changed since is the turn.
        (Phase::End, Some(head)) => changed_paths(repo, &format!("{head}^{{tree}}"), &tree)?,
        _ => Vec::new(),
    };
    let record = Record { phase, turn: turn.map(str::to_owned), paths };
    let message = format!("Bridge chat snapshot\n\n{}\n", serde_json::to_string(&record).map_err(|error| BridgeError::Invalid(error.to_string()))?);
    let mut commit = git::git_command(repo);
    commit.envs(SNAPSHOT_IDENTITY).args(["-c", "commit.gpgsign=false", "commit-tree", &tree, "-m", &message]);
    if let Some(head) = &head {
        commit.args(["-p", head]);
    }
    let commit = output(&mut commit)?;
    let mut update = git::git_command(repo);
    update.args(["update-ref", "-m", "bridge chat snapshot", &reference, &commit]);
    if let Some(head) = &head {
        update.arg(head);
    }
    output(&mut update)?;
    Ok(())
}

/// What `session_id` changed in `repo`: its first snapshot diffed against a
/// live one, kept to the paths its turns touched.
pub fn session_changes(repo: &Path, session_id: &str, include_patches: bool) -> Result<SessionChanges, BridgeError> {
    if !git::is_repository(repo) {
        return Ok(SessionChanges::unavailable(SessionChangesUnavailable::NotGit));
    }
    let chain = read_chain(repo, session_id)?;
    let Some(first) = chain.first() else {
        return Ok(SessionChanges::unavailable(SessionChangesUnavailable::NoTurns));
    };
    let live = {
        let _guard = SNAPSHOTS.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        match write_tree(repo, session_id) {
            Ok(tree) => tree,
            Err(_) => return Ok(SessionChanges::unavailable(SessionChangesUnavailable::SnapshotFailed)),
        }
    };
    let mut paths: HashSet<String> = chain
        .iter()
        .filter(|snapshot| snapshot.record.phase == Phase::End)
        .flat_map(|snapshot| snapshot.record.paths.iter().cloned())
        .collect();
    let last = chain.last().unwrap_or(first);
    if last.record.phase == Phase::Start {
        // A turn still running: its changes so far are part of the chat.
        paths.extend(changed_paths(repo, &last.tree, &live)?);
    }
    let (changes, (additions, deletions)) =
        git::tree_changeset(repo, &first.tree, &live, &|path| paths.contains(path), include_patches)?;
    Ok(SessionChanges {
        available: true,
        unavailable_reason: None,
        turns: chain.iter().filter(|snapshot| snapshot.record.phase == Phase::Start).count(),
        since: Some(first.committed_at.clone()),
        additions,
        deletions,
        changes: Some(changes),
    })
}

/// The ref holding a chat's chain. Session ids are Bridge's own, but a ref
/// name has rules of its own; anything outside a safe alphabet becomes `-`.
fn ref_name(session_id: &str) -> String {
    format!("{REF_PREFIX}{}", safe_component(session_id))
}

fn safe_component(session_id: &str) -> String {
    let cleaned: String = session_id
        .chars()
        .map(|character| if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') { character } else { '-' })
        .collect();
    if cleaned.is_empty() { "unnamed".into() } else { cleaned }
}

/// The chat's private index. Kept between snapshots so git's stat cache
/// makes every snapshot after the first cost a directory walk, not a rehash.
fn index_path(repo: &Path, session_id: &str) -> Result<PathBuf, BridgeError> {
    let relative = format!("bridge-checkpoints/{}.index", safe_component(session_id));
    Ok(repo.join(output(git::git_command(repo).args(["rev-parse", "--git-path", &relative]))?))
}

/// Write the working tree, as `git add -A` sees it, to a tree object.
fn write_tree(repo: &Path, session_id: &str) -> Result<String, BridgeError> {
    let index = index_path(repo, session_id)?;
    if let Some(parent) = index.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if !index.exists() {
        // Seed from the person's own index: tracked files are then already
        // hashed, and the first snapshot only reads what changed.
        if let Ok(own) = output(git::git_command(repo).args(["rev-parse", "--git-path", "index"])) {
            let _copied = std::fs::copy(repo.join(own), &index);
        }
    }
    let oversized = oversized_untracked(repo)?;
    let mut add = git::git_command(repo);
    add.env("GIT_INDEX_FILE", &index).args(["-c", "advice.addEmbeddedRepo=false", "add", "-A"]);
    let pathspec_file = if oversized.is_empty() {
        None
    } else {
        let file = index.with_extension("pathspec");
        let mut body = b".\0".to_vec();
        for path in &oversized {
            body.extend_from_slice(format!(":(exclude,literal){path}").as_bytes());
            body.push(0);
        }
        std::fs::write(&file, body)?;
        add.arg(format!("--pathspec-from-file={}", file.display())).arg("--pathspec-file-nul");
        Some(file)
    };
    let added = output_within(&mut add, SNAPSHOT_TIMEOUT);
    if let Some(file) = pathspec_file {
        let _removed = std::fs::remove_file(file);
    }
    added?;
    output(git::git_command(repo).env("GIT_INDEX_FILE", &index).arg("write-tree"))
}

/// Untracked, unignored files over the size cap.
fn oversized_untracked(repo: &Path) -> Result<Vec<String>, BridgeError> {
    let mut list = git::git_command(repo);
    list.args(["ls-files", "--others", "--exclude-standard", "-z"]);
    let listing = git::command_output_deadline(&mut list, SNAPSHOT_TIMEOUT)?;
    if !listing.status.success() {
        return Err(BridgeError::Git(String::from_utf8_lossy(&listing.stderr).trim().into()));
    }
    if listing.stdout.len() > MAX_UNTRACKED_LISTING_BYTES {
        return Err(BridgeError::Git("too many untracked files to snapshot".into()));
    }
    Ok(listing
        .stdout
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(|path| String::from_utf8_lossy(path).into_owned())
        .filter(|path| {
            std::fs::symlink_metadata(repo.join(path))
                .is_ok_and(|metadata| metadata.is_file() && metadata.len() > MAX_SNAPSHOT_FILE_BYTES)
        })
        .collect())
}

/// Paths that differ between two trees, both sides of a rename, gitlinks
/// left out.
fn changed_paths(repo: &Path, from: &str, to: &str) -> Result<Vec<String>, BridgeError> {
    let names = output_bytes(git::git_command(repo).args([
        "diff-tree", "-r", "-z", "--name-only", "--no-renames", "--ignore-submodules=all", from, to,
    ]))?;
    Ok(names
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(|path| String::from_utf8_lossy(path).into_owned())
        .collect())
}

/// The chat's snapshots, oldest first.
fn read_chain(repo: &Path, session_id: &str) -> Result<Vec<Snapshot>, BridgeError> {
    let reference = ref_name(session_id);
    if rev_parse(repo, &reference).is_none() {
        return Ok(Vec::new());
    }
    let mut log = git::git_command(repo);
    log.args(["log", "--reverse", "--first-parent", "-z", "--format=%T%x1f%cI%x1f%B", &reference, "--"]);
    let listing = git::command_output_deadline(&mut log, SNAPSHOT_TIMEOUT)?;
    if !listing.status.success() || listing.stdout.len() > MAX_CHAIN_BYTES {
        return Err(BridgeError::Git("could not read this chat's snapshots".into()));
    }
    Ok(listing
        .stdout
        .split(|byte| *byte == 0)
        .filter_map(|entry| {
            let text = String::from_utf8_lossy(entry);
            let mut fields = text.splitn(3, '\u{1f}');
            let tree = fields.next()?.trim().to_owned();
            let committed_at = fields.next()?.trim().to_owned();
            let body = fields.next()?;
            let record = body.lines().rev().find_map(|line| serde_json::from_str::<Record>(line.trim()).ok())?;
            (!tree.is_empty()).then_some(Snapshot { tree, committed_at, record })
        })
        .collect())
}

fn rev_parse(repo: &Path, reference: &str) -> Option<String> {
    output(git::git_command(repo).args(["rev-parse", "--verify", "--quiet", reference])).ok()
}

fn output(command: &mut std::process::Command) -> Result<String, BridgeError> {
    Ok(String::from_utf8_lossy(&output_bytes(command)?).trim().to_owned())
}

fn output_bytes(command: &mut std::process::Command) -> Result<Vec<u8>, BridgeError> {
    output_within(command, SNAPSHOT_TIMEOUT)
}

fn output_within(command: &mut std::process::Command, timeout: Duration) -> Result<Vec<u8>, BridgeError> {
    let result = git::command_output_deadline(command, timeout)?;
    if !result.status.success() {
        return Err(BridgeError::Git(String::from_utf8_lossy(&result.stderr).trim().into()));
    }
    Ok(result.stdout)
}

#[cfg(test)]
mod tests;
