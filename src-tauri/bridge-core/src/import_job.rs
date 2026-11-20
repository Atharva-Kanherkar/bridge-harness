//! The background import behind the one Import button.
//!
//! It imports everything Bridge can bring over from a tool, in Bridge's own
//! process, so the window can close while it runs. It never builds a review
//! list: it lists file paths and sizes, then reads and commits a small batch
//! at a time, bounded by count and by bytes, so memory stays flat however many
//! transcripts there are. After every batch it saves its progress and a
//! resume cursor, and publishes `imports/progress`. A job Bridge quit in the
//! middle of resumes on the next start; chats imported earlier are recognised
//! by the importer and not imported twice.

use crate::claude_import::{self, ClaudeCodeImporter};
use crate::codex_import::{self, CodexImporter};
use crate::opencode_import::{self, OpenCodeImporter};
use crate::BridgeError;
use crate::events::CoreEvent;
use crate::external_import::{
    CandidateKind, ConflictPolicy, DiscoveredArtifact, DiscoverySelection, ExternalHarnessImporter,
    ImportCandidate, ImportCommit, ImportPlan, SetupActivationPolicy,
};
use crate::runtime::BridgeCore;
use bridge_protocol::messages as wire;
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Files read and committed per batch, and the bytes a batch may hold. A
/// parsed chat takes several times its file size in memory, so the byte limit
/// is what keeps the job's memory flat. One file larger than the limit is a
/// batch of its own.
const BATCH_FILES: usize = 25;
const BATCH_BYTES: u64 = 16 * 1024 * 1024;
/// Progress events are at most this frequent; the last one always goes out.
const PROGRESS_EVERY: Duration = Duration::from_millis(250);

fn install(db: &Connection) -> Result<(), BridgeError> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS import_jobs (
            job_id TEXT PRIMARY KEY,
            status TEXT NOT NULL,
            cursor TEXT,
            state TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );",
    )?;
    Ok(())
}

fn latest(db: &Connection) -> Result<Option<(wire::ImportJob, Option<String>)>, BridgeError> {
    install(db)?;
    let row: Option<(String, Option<String>)> = db
        .query_row(
            "SELECT state, cursor FROM import_jobs ORDER BY rowid DESC LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    row.map(|(state, cursor)| {
        serde_json::from_str(&state)
            .map(|job| (job, cursor))
            .map_err(|error| BridgeError::Invalid(format!("Saved import is unreadable: {error}")))
    })
    .transpose()
}

fn save(core: &BridgeCore, job: &wire::ImportJob, cursor: Option<&str>) -> Result<(), BridgeError> {
    let state = serde_json::to_string(job).map_err(|error| BridgeError::Invalid(error.to_string()))?;
    let db = core.db.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    install(&db)?;
    db.execute(
        "INSERT INTO import_jobs (job_id, status, cursor, state, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(job_id) DO UPDATE SET status=excluded.status, cursor=excluded.cursor,
             state=excluded.state, updated_at=excluded.updated_at",
        params![job.job_id, job.status, cursor, state, job.updated_at],
    )?;
    Ok(())
}

/// The latest import from each tool, newest first.
pub fn status_by_tool(core: &BridgeCore) -> Result<Vec<wire::ImportJob>, BridgeError> {
    let db = core.db.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    install(&db)?;
    let mut statement = db.prepare("SELECT state FROM import_jobs ORDER BY rowid DESC")?;
    let states = statement.query_map([], |row| row.get::<_, String>(0))?;
    let mut seen = HashSet::new();
    let mut jobs = Vec::new();
    for state in states {
        let Ok(job) = serde_json::from_str::<wire::ImportJob>(&state?) else { continue };
        if seen.insert(job.tool.clone()) {
            jobs.push(job);
        }
    }
    Ok(jobs)
}

/// The latest import, running or finished.
pub fn status(core: &BridgeCore) -> Result<Option<wire::ImportJob>, BridgeError> {
    let db = core.db.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    Ok(latest(&db)?.map(|(job, _)| job))
}

/// A tool the job can import from, and how it reads that tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Source {
    Claude,
    Codex,
    OpenCode,
}

impl Source {
    pub(crate) fn of(tool: &str) -> Option<Self> {
        match tool {
            claude_import::PROVIDER => Some(Self::Claude),
            codex_import::PROVIDER => Some(Self::Codex),
            opencode_import::PROVIDER => Some(Self::OpenCode),
            _ => None,
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::Claude => "Claude Code",
            Self::Codex => "Codex",
            Self::OpenCode => "OpenCode",
        }
    }
    /// The folders this machine keeps the tool's data in.
    fn machine_roots(self) -> Vec<PathBuf> {
        match self {
            Self::Claude => claude_import::machine_config_roots(),
            Self::Codex => codex_import::machine_codex_homes(),
            Self::OpenCode => opencode_import::machine_opencode_homes(),
        }
    }
    fn work(self, roots: &[PathBuf]) -> Result<claude_import::JobWork, BridgeError> {
        match self {
            Self::Claude => claude_import::job_work(roots),
            Self::Codex => codex_import::job_work(roots),
            Self::OpenCode => opencode_import::job_work(roots),
        }
    }
    fn key(self, artifact: &DiscoveredArtifact) -> String {
        match self {
            Self::Claude => claude_import::job_key(artifact),
            Self::Codex => codex_import::job_key(artifact),
            Self::OpenCode => opencode_import::job_key(artifact),
        }
    }
    /// The folder a chat ran in, for "now importing": the listing's when it
    /// has one (OpenCode's sessions are rows, not files), else read from the
    /// file.
    fn folder(self, artifact: &DiscoveredArtifact) -> Option<String> {
        if let Some(folder) = artifact.project_path.clone() {
            return Some(folder);
        }
        let path = Path::new(&artifact.canonical_source_ref);
        match self {
            Self::Claude => claude_import::transcript_folder(path),
            Self::Codex => codex_import::transcript_folder(path),
            Self::OpenCode => None,
        }
    }
    fn preview(self, roots: &[PathBuf], artifacts: Vec<DiscoveredArtifact>) -> Result<Vec<ImportCandidate>, BridgeError> {
        let selection = DiscoverySelection {
            artifact_ids: artifacts.iter().map(|artifact| artifact.artifact_id.clone()).collect(),
        };
        match self {
            Self::Claude => ClaudeCodeImporter.preview(&claude_import::job_discovery(roots, artifacts), &selection),
            Self::Codex => CodexImporter.preview(&codex_import::job_discovery(roots, artifacts), &selection),
            Self::OpenCode => OpenCodeImporter.preview(&opencode_import::job_discovery(roots, artifacts), &selection),
        }
    }
    fn normalize(self, candidate: ImportCandidate) -> Result<crate::external_import::NormalizedImportCandidate, BridgeError> {
        match self {
            Self::Claude => ClaudeCodeImporter.normalize(candidate),
            Self::Codex => CodexImporter.normalize(candidate),
            Self::OpenCode => OpenCodeImporter.normalize(candidate),
        }
    }
}

/// Start importing everything from `tool` in the background, and return at
/// once. While a job runs, it is returned instead of starting another.
pub fn start(core: &Arc<BridgeCore>, tool: &str) -> Result<wire::ImportJob, BridgeError> {
    let source = Source::of(tool).ok_or_else(|| BridgeError::Invalid(format!("Bridge cannot import from '{tool}' yet")))?;
    start_with_roots(core, tool, source.machine_roots())
}

pub(crate) fn start_with_roots(
    core: &Arc<BridgeCore>,
    tool: &str,
    roots: Vec<PathBuf>,
) -> Result<wire::ImportJob, BridgeError> {
    if core.import_job_running.swap(true, Ordering::SeqCst) {
        return status(core)?.ok_or_else(|| BridgeError::Invalid("An import is starting".into()));
    }
    let Some(source) = Source::of(tool) else {
        core.import_job_running.store(false, Ordering::SeqCst);
        return Err(BridgeError::Invalid(format!("Bridge cannot import from '{tool}' yet")));
    };
    if roots.is_empty() {
        core.import_job_running.store(false, Ordering::SeqCst);
        return Err(BridgeError::Invalid(format!("{} was not found on this Mac", source.label())));
    }
    let now = Utc::now().to_rfc3339();
    let job = wire::ImportJob {
        job_id: format!("import_job_{}", uuid::Uuid::new_v4().simple()),
        tool: tool.into(),
        status: "running".into(),
        started_at: now.clone(),
        updated_at: now,
        ..wire::ImportJob::default()
    };
    if let Err(error) = save(core, &job, None) {
        core.import_job_running.store(false, Ordering::SeqCst);
        return Err(error);
    }
    spawn(core.clone(), job.clone(), roots, None);
    Ok(job)
}

/// On startup: carry on with an import Bridge quit in the middle of, from
/// the tool it was importing.
pub fn start_import_maintenance(core: Arc<BridgeCore>) {
    if let Err(error) = resume_with(&core, |source| source.machine_roots()) {
        eprintln!("bridge: could not resume the import: {error}");
    }
}

#[cfg(test)]
pub(crate) fn resume_with_roots(core: &Arc<BridgeCore>, roots: Vec<PathBuf>) -> Result<bool, BridgeError> {
    resume_with(core, |_| roots.clone())
}

fn resume_with(core: &Arc<BridgeCore>, roots_for: impl Fn(Source) -> Vec<PathBuf>) -> Result<bool, BridgeError> {
    let saved = {
        let db = core.db.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        latest(&db)?
    };
    let Some((job, cursor)) = saved.filter(|(job, _)| job.status == "running") else {
        return Ok(false);
    };
    let Some(source) = Source::of(&job.tool) else {
        return Ok(false);
    };
    if core.import_job_running.swap(true, Ordering::SeqCst) {
        return Ok(false);
    }
    spawn(core.clone(), job, roots_for(source), cursor);
    Ok(true)
}

fn spawn(core: Arc<BridgeCore>, job: wire::ImportJob, roots: Vec<PathBuf>, cursor: Option<String>) {
    let worker = core.clone();
    let started = std::thread::Builder::new()
        .name("bridge-import".into())
        .spawn(move || run(&worker, job, &roots, cursor));
    if started.is_err() {
        core.import_job_running.store(false, Ordering::SeqCst);
    }
}

/// Run a job to the end, then record how it ended. Runs on its own thread.
pub(crate) fn run(core: &Arc<BridgeCore>, mut job: wire::ImportJob, roots: &[PathBuf], cursor: Option<String>) {
    let source = Source::of(&job.tool).unwrap_or(Source::Claude);
    let outcome = Runner { core, source, job: &mut job, last_publish: None }.run(roots, cursor);
    job.status = if outcome.is_ok() { "done" } else { "failed" }.into();
    if let Err(error) = outcome {
        job.errors.push(error.to_string());
    }
    job.current_project = None;
    let now = Utc::now().to_rfc3339();
    job.updated_at = now.clone();
    job.finished_at = Some(now);
    if let Err(error) = save(core, &job, None) {
        eprintln!("bridge: could not save the finished import: {error}");
    }
    core.events.publish(CoreEvent::ImportProgress(job));
    core.import_job_running.store(false, Ordering::SeqCst);
}

struct Runner<'a> {
    core: &'a Arc<BridgeCore>,
    source: Source,
    job: &'a mut wire::ImportJob,
    last_publish: Option<Instant>,
}

impl Runner<'_> {
    fn run(&mut self, roots: &[PathBuf], cursor: Option<String>) -> Result<(), BridgeError> {
        let work = self.source.work(roots)?;
        self.run_work(work, cursor)
    }

    fn run_work(&mut self, work: claude_import::JobWork, cursor: Option<String>) -> Result<(), BridgeError> {
        let remaining: Vec<DiscoveredArtifact> = work
            .artifacts
            .into_iter()
            .filter(|artifact| cursor.as_deref().is_none_or(|cursor| self.source.key(artifact).as_str() > cursor))
            .collect();
        self.job.total = self.job.processed + remaining.len() as u64;
        self.job.left_out = work.left_out;
        self.checkpoint(cursor.as_deref())?;
        for batch in batches(remaining) {
            let Some(last) = batch.last().map(|artifact| self.source.key(artifact)) else {
                continue;
            };
            let files = batch.len() as u64;
            self.job.current_project = batch
                .iter()
                .find(|artifact| artifact.kind == CandidateKind::Conversation)
                .and_then(|artifact| self.source.folder(artifact));
            match import_batch(self.core, self.source, &work.roots, batch) {
                Ok(outcome) => {
                    add_batch(self.job, &outcome.candidates, outcome.commit.as_ref());
                    for refusal in &outcome.refusals {
                        self.job.failed += refusal.count;
                        not_carried(self.job, &refusal_text(&refusal.kind, &refusal.reason), refusal.count);
                    }
                }
                Err(error) => {
                    self.job.failed += files;
                    self.job.errors.push(format!("{files} files could not be imported: {error}"));
                }
            }
            self.job.processed += files;
            self.checkpoint(Some(&last))?;
        }
        Ok(())
    }

    /// Save progress and the cursor, then tell the window (throttled).
    fn checkpoint(&mut self, cursor: Option<&str>) -> Result<(), BridgeError> {
        self.job.updated_at = Utc::now().to_rfc3339();
        save(self.core, self.job, cursor)?;
        if self.last_publish.is_none_or(|at| at.elapsed() >= PROGRESS_EVERY) {
            self.last_publish = Some(Instant::now());
            self.core.events.publish(CoreEvent::ImportProgress(self.job.clone()));
        }
        Ok(())
    }
}

/// Split the work into batches of at most `BATCH_FILES` files and
/// `BATCH_BYTES` bytes, in order.
pub(crate) fn batches(artifacts: Vec<DiscoveredArtifact>) -> Vec<Vec<DiscoveredArtifact>> {
    let mut out: Vec<Vec<DiscoveredArtifact>> = Vec::new();
    let mut bytes = 0u64;
    for artifact in artifacts {
        let size = artifact.estimated_bytes;
        match out.last_mut() {
            Some(batch) if batch.len() < BATCH_FILES && bytes.saturating_add(size) <= BATCH_BYTES => {
                bytes = bytes.saturating_add(size);
                batch.push(artifact);
            }
            _ => {
                bytes = size;
                out.push(vec![artifact]);
            }
        }
    }
    out
}

/// What one batch did: everything it read, what was committed, and the
/// families Bridge refused, each with why.
struct BatchOutcome {
    candidates: Vec<ImportCandidate>,
    commit: Option<ImportCommit>,
    refusals: Vec<Refusal>,
}

struct Refusal {
    kind: CandidateKind,
    reason: String,
    count: u64,
}

/// Read one batch at full fidelity and commit everything importable in it:
/// chats alongside what is already there, setup switched off.
/// Each family (a chat with its subagents, or one item) commits in its own
/// transaction, so one item Bridge refuses never costs the rest of the batch.
fn import_batch(
    core: &Arc<BridgeCore>,
    source: Source,
    roots: &[PathBuf],
    artifacts: Vec<DiscoveredArtifact>,
) -> Result<BatchOutcome, BridgeError> {
    // The report needs each candidate's id, kind and diagnostics, never its
    // messages: keep a copy without the payload, and move the payload on to
    // the commit instead of holding a second copy of every chat.
    let mut previewed = Vec::new();
    let mut normalized = Vec::new();
    for mut candidate in source.preview(roots, artifacts)? {
        let payload = std::mem::take(&mut candidate.normalized_payload);
        previewed.push(candidate.clone());
        if candidate.kind != CandidateKind::Unsupported {
            candidate.normalized_payload = payload;
            normalized.push(source.normalize(candidate)?);
        }
    }
    if normalized.is_empty() {
        return Ok(BatchOutcome { candidates: previewed, commit: None, refusals: Vec::new() });
    }
    let selected: HashSet<&str> = normalized
        .iter()
        .map(|candidate| candidate.candidate.candidate_id.as_str())
        .collect();
    let plan = ImportPlan {
        selected_candidate_ids: selected.iter().map(|id| (*id).to_owned()).collect(),
        conflict_policy: ConflictPolicy::ImportAlongside,
        setup_activation_policy: SetupActivationPolicy::Disabled,
        memory_scope: None,
        dry_run: false,
        created_at: Utc::now().to_rfc3339(),
    };
    let mut commit: Option<ImportCommit> = None;
    let mut refusals = Vec::new();
    for family in crate::api::import_families(&normalized, &selected) {
        let family_plan = ImportPlan { selected_candidate_ids: family.clone(), ..plan.clone() };
        let result = {
            let db = core.db.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            crate::external_import::commit_import(&db, &normalized, &family_plan)
        };
        match result {
            Ok(next) => merge_commit(&mut commit, next),
            Err(error) => refusals.push(Refusal {
                kind: normalized
                    .iter()
                    .find(|candidate| candidate.candidate.candidate_id == family[0])
                    .map_or(CandidateKind::Unsupported, |candidate| candidate.candidate.kind.clone()),
                reason: error.to_string(),
                count: family.len() as u64,
            }),
        }
    }
    let Some(commit) = commit else {
        return Ok(BatchOutcome { candidates: previewed, commit: None, refusals });
    };
    if commit.imported > 0 {
        core.events.publish(CoreEvent::StateChanged);
    }
    Ok(BatchOutcome { candidates: previewed, commit: Some(commit), refusals })
}

fn merge_commit(total: &mut Option<ImportCommit>, next: ImportCommit) {
    let Some(total) = total.as_mut() else {
        *total = Some(next);
        return;
    };
    total.candidate_results.extend(next.candidate_results);
    total.created_bridge_ids.extend(next.created_bridge_ids);
    total.imported += next.imported;
    total.skipped += next.skipped;
    total.changed += next.changed;
    total.conflicted += next.conflicted;
    total.rejected += next.rejected;
    total.unsupported += next.unsupported;
    total.diagnostics.extend(next.diagnostics);
}

/// Why Bridge refused an item, as a person reads it in "did not come over".
fn refusal_text(kind: &CandidateKind, reason: &str) -> String {
    let what = match kind {
        CandidateKind::Conversation => "chats",
        CandidateKind::Memory => "memories",
        _ => "setup items",
    };
    format!("{what} Bridge refused: {reason}")
}

/// Fold one batch's preview and commit into the job's report.
pub(crate) fn add_batch(job: &mut wire::ImportJob, candidates: &[ImportCandidate], commit: Option<&ImportCommit>) {
    if let Some(commit) = commit {
        job.imported += u64::from(commit.imported);
        job.already_in_bridge += u64::from(commit.skipped);
        job.changed += u64::from(commit.changed);
        job.failed += u64::from(commit.rejected + commit.conflicted);
        job.errors.extend(
            commit
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.severity == "error")
                .map(|diagnostic| diagnostic.message.clone()),
        );
    }
    for candidate in candidates {
        if candidate.kind == CandidateKind::Unsupported {
            job.failed += 1;
            let reason = candidate
                .diagnostics
                .first()
                .map_or("it could not be read", |diagnostic| diagnostic.message.as_str());
            not_carried(job, &format!("A chat could not be read: {reason}"), 1);
            continue;
        }
        for diagnostic in &candidate.diagnostics {
            if diagnostic.code == "transcript_record_skipped" {
                if let Some((text, count)) = skipped_record(&diagnostic.message) {
                    not_carried(job, &text, count);
                }
            } else if diagnostic.code.starts_with("excluded_") {
                let (count, text) = leading_count(&diagnostic.message);
                not_carried(job, text, count);
            }
        }
    }
}

fn not_carried(job: &mut wire::ImportJob, text: &str, count: u64) {
    match job.not_carried.iter_mut().find(|item| item.text == text) {
        Some(item) => item.count += count,
        None => job.not_carried.push(wire::ImportNotCarried { text: text.into(), count }),
    }
}

/// "3 images over 5 MB" is 3 of "images over 5 MB"; no number is one.
fn leading_count(message: &str) -> (u64, &str) {
    message
        .split_once(' ')
        .and_then(|(number, rest)| number.parse().ok().map(|count| (count, rest)))
        .unwrap_or((1, message))
}

/// "Skipped 4 'content:document' records" as what a person lost, or `None`
/// when nothing was lost (a repeat the reader dropped).
fn skipped_record(message: &str) -> Option<(String, u64)> {
    let rest = message.strip_prefix("Skipped ")?;
    let (count, rest) = rest.split_once(" '")?;
    let (reason, _) = rest.split_once('\'')?;
    Some((skipped_record_text(reason)?, count.parse().ok()?))
}

fn skipped_record_text(reason: &str) -> Option<String> {
    let fixed = match reason {
        "duplicate_source_id" => return None,
        "content:document" => "documents or PDFs inside messages",
        "content:redacted_thinking" => "thinking Claude Code kept hidden",
        "tool_result_without_call" => "tool results with no matching call",
        "malformed_line" => "unreadable lines",
        "oversized_line" => "lines over 64 MB",
        "non_object_line" | "untyped_record" | "unreadable_row" => "unreadable records",
        "invalid_timestamp" => "records with an unreadable time",
        "missing_uuid" => "records without an id",
        "missing_message_payload" => "records with no message",
        "unsupported_content_shape" => "messages in a shape Bridge cannot read",
        "untyped_content_block" => "message parts in a shape Bridge cannot read",
        _ => "",
    };
    if !fixed.is_empty() {
        return Some(fixed.into());
    }
    Some(if let Some(kind) = reason.strip_prefix("record:") {
        format!("records of a kind Bridge does not know ({kind})")
    } else if let Some(kind) = reason.strip_prefix("item:") {
        format!("items of a kind Bridge does not show ({kind})")
    } else if let Some(kind) = reason.strip_prefix("event:") {
        format!("events of a kind Bridge does not know ({kind})")
    } else if let Some(kind) = reason.strip_prefix("row:") {
        format!("records of a kind Bridge does not know ({kind})")
    } else if let Some(kind) = reason.strip_prefix("part:") {
        format!("message parts of a kind Bridge does not show ({kind})")
    } else if let Some(kind) = reason.strip_prefix("system:") {
        format!("system records of a kind Bridge does not know ({kind})")
    } else if let Some(kind) = reason.strip_prefix("content:") {
        format!("message parts of a kind Bridge does not show ({kind})")
    } else {
        reason.replace('_', " ")
    })
}

#[cfg(test)]
#[path = "import_job/tests.rs"]
mod tests;
