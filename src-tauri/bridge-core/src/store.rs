use crate::{model::*, BridgeError};
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    io::Read,
    path::{Path, PathBuf},
};
use uuid::Uuid;

const LATEST_SCHEMA_VERSION: i64 = 55;
const MIGRATION_BACKUP_TIMESTAMP_FORMAT: &str = "%Y%m%dT%H%M%S%fZ";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TelemetrySpan {
    pub span_id: String,
    pub trace_id: String,
    pub name: String,
    pub attributes: String,
    pub started_at: String,
    pub ended_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HistorySnapshotManifest {
    pub schema_version: u32,
    pub database_file: String,
    pub sha256: String,
    pub created_at: String,
}

pub fn open(path: &Path) -> Result<Connection, BridgeError> {
    if path != Path::new(":memory:") && path.try_exists()? {
        // Even a SELECT on a read-write connection can recover a hot journal.
        // Inspect existing stores without permission to rewrite them first.
        let preflight =
            Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        supported_schema_version(&preflight)?;
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut connection = Connection::open(path)?;
    // An older build must not run recovery or prune a newer build's rollback
    // copies. Check before any application writes or maintenance on this store.
    supported_schema_version(&connection)?;
    // Migrations run with foreign keys disabled so table rebuilds (which drop and
    // recreate parent tables) don't trip referential checks; re-enabled after.
    connection.execute_batch("PRAGMA foreign_keys=OFF;")?;
    if path != Path::new(":memory:") {
        prune_migration_backups_and_report(path, None);
    }
    let migration_backup = run_migrations(&mut connection, path)?;
    if path != Path::new(":memory:") {
        prune_migration_backups_and_report(path, migration_backup.as_deref());
    }
    connection.execute_batch(
        "PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000;",
    )?;
    // Chats created before titles existed still read "Orchestrator"; name them
    // from what they already contain. Local-only, so opening stays cheap.
    let _ = crate::session_titles::backfill_from_messages(&connection);
    let now = Utc::now().to_rfc3339();
    connection.execute(
        "INSERT OR IGNORE INTO session_heads(session_id,native_provider_session_id,restoration_mode,resume_eligibility,updated_at)
         SELECT id,provider_session_id,'fresh',CASE WHEN provider_session_id IS NOT NULL THEN 'native' ELSE 'fresh' END,?1
         FROM sessions WHERE status IN ('working','waiting')",
        params![now],
    )?;
    connection.execute(
        "UPDATE session_heads
         SET native_provider_session_id=COALESCE(native_provider_session_id,(SELECT provider_session_id FROM sessions WHERE sessions.id=session_heads.session_id)),
             resume_eligibility=CASE
                 WHEN COALESCE(native_provider_session_id,(SELECT provider_session_id FROM sessions WHERE sessions.id=session_heads.session_id)) IS NOT NULL THEN 'native'
                 WHEN active_entry_id IS NOT NULL OR latest_checkpoint_entry_id IS NOT NULL THEN 'checkpoint_restored'
                 ELSE 'fresh'
             END,
             updated_at=?1
         WHERE session_id IN (SELECT id FROM sessions WHERE status IN ('working','waiting'))",
        params![now],
    )?;
    connection.execute(
        "UPDATE worker_leases SET lease_status='expired',updated_at=?1
         WHERE lease_status IN ('active','warm') AND session_id IN (
            SELECT session_id FROM worker_runtime
            WHERE lifecycle_state IN ('starting','working','waiting','warm','checkpointing','resuming','restored','failed')
         )",
        params![now],
    )?;
    connection.execute(
        "UPDATE sessions SET status='stopped', ended_at=?1, active_turn_id=NULL WHERE status IN ('working','waiting')",
        params![now],
    )?;
    // The invariant the composer renders from: a session in a terminal state
    // has no active turn. Crash recoveries used to stop sessions without
    // clearing the turn id, and each one left a composer stuck on Stop/Steer
    // with nothing running — so reconcile rows already damaged that way too.
    connection.execute(
        "UPDATE sessions SET active_turn_id=NULL
         WHERE active_turn_id IS NOT NULL
           AND status IN ('stopped','failed','completed','cancelled','ready')",
        [],
    )?;
    connection.execute(
        "UPDATE workspaces SET status='stopped' WHERE status IN ('working','waiting')",
        [],
    )?;
    Ok(connection)
}

pub fn open_telemetry(path: &Path) -> Result<Connection, BridgeError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let connection = Connection::open(path)?;
    connection.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA busy_timeout=50;
         CREATE TABLE IF NOT EXISTS telemetry_spans (
            span_id TEXT PRIMARY KEY,
            trace_id TEXT NOT NULL,
            parent_span_id TEXT,
            name TEXT NOT NULL,
            attributes TEXT NOT NULL,
            started_at TEXT NOT NULL,
            ended_at TEXT
         );
         CREATE INDEX IF NOT EXISTS idx_telemetry_trace ON telemetry_spans(trace_id,started_at);",
    )?;
    Ok(connection)
}

pub fn telemetry_span(
    trace_id: &str,
    session_id: &str,
    adapter_id: &str,
    event: &crate::agent::NormalizedEvent,
    occurred_at: &str,
) -> TelemetrySpan {
    TelemetrySpan {
        span_id: Uuid::new_v4().simple().to_string(),
        trace_id: trace_id.to_owned(),
        name: format!("gen_ai.{}", event.kind.replace('.', "_")),
        attributes: serde_json::json!({
            "gen_ai.operation.name": event.kind,
            "gen_ai.provider.name": adapter_id,
            "gen_ai.conversation.id": session_id,
        })
        .to_string(),
        started_at: occurred_at.to_owned(),
        ended_at: occurred_at.to_owned(),
    }
}

pub fn append_telemetry_batch(
    db: &Connection,
    spans: &[TelemetrySpan],
) -> Result<usize, BridgeError> {
    if spans.is_empty() {
        return Ok(0);
    }
    let transaction = db.unchecked_transaction()?;
    for span in spans {
        transaction.execute(
            "INSERT OR IGNORE INTO telemetry_spans(span_id,trace_id,name,attributes,started_at,ended_at)
             VALUES(?1,?2,?3,?4,?5,?6)",
            params![
                span.span_id,
                span.trace_id,
                span.name,
                span.attributes,
                span.started_at,
                span.ended_at,
            ],
        )?;
    }
    transaction.commit()?;
    Ok(spans.len())
}

pub fn export_history_snapshot(
    db: &Connection,
    snapshot_dir: &Path,
) -> Result<(PathBuf, PathBuf), BridgeError> {
    std::fs::create_dir_all(snapshot_dir)?;
    let id = format!(
        "{}-{}",
        Utc::now().format("%Y%m%dT%H%M%S%fZ"),
        Uuid::new_v4().simple()
    );
    let database_file = format!("bridge-history-{id}.sqlite");
    let database_path = snapshot_dir.join(&database_file);
    let manifest_path = snapshot_dir.join(format!("bridge-history-{id}.manifest.json"));
    // Everything slow — `VACUUM INTO`, the hash, the manifest write — happens
    // under dotted, Bridge-prefixed pending names that retention recognises as
    // this process's own debris. Publication is then two adjacent renames, so
    // the only torn state a crash can leave is a final-named database with no
    // manifest, which `reclaim_incomplete_snapshot_artifacts` removes once it
    // is older than the grace period.
    let pending_database = snapshot_dir.join(format!("{PENDING_PREFIX}{id}.sqlite.tmp"));
    let pending_manifest = snapshot_dir.join(format!("{PENDING_PREFIX}{id}.manifest.tmp"));
    let escaped = pending_database.to_string_lossy().replace('\'', "''");
    db.execute_batch(&format!("VACUUM INTO '{escaped}'"))?;
    let sha256 = hash_file_streaming(&pending_database)?;
    let manifest = HistorySnapshotManifest {
        schema_version: 1,
        database_file,
        sha256,
        created_at: Utc::now().to_rfc3339(),
    };
    let manifest_bytes = serde_json::to_vec_pretty(&manifest)
        .map_err(|error| BridgeError::Invalid(error.to_string()))?;
    std::fs::write(&pending_manifest, manifest_bytes)?;
    std::fs::rename(&pending_database, &database_path)?;
    std::fs::rename(&pending_manifest, &manifest_path)?;
    // Every producer prunes, so the directory stays within policy no matter
    // which cadence wrote last. Best effort: a prune failure must not fail the
    // export that just succeeded.
    prune_history_snapshots_and_report(snapshot_dir);
    Ok((database_path, manifest_path))
}

/// Prefix of the dotted pending files an export writes before publication.
/// Retention reclaims only pending files carrying this prefix, so another
/// tool's `.something.sqlite.tmp` in the directory is never Bridge's to delete.
const PENDING_PREFIX: &str = ".bridge-history-";

/// Prune under the default policy and log anything worth a human's attention:
/// deletions, files that could not be deleted, and a retained set over budget.
/// Both producers call this, and both used to discard the outcome, which left
/// a heuristic deletion path with no observability at all.
fn prune_history_snapshots_and_report(snapshot_dir: &Path) {
    match prune_history_snapshots(snapshot_dir, HistorySnapshotRetention::default()) {
        Ok(outcome) if outcome.is_quiet() => {}
        Ok(outcome) => eprintln!(
            "bridge: history snapshot retention removed_pairs={} removed_bytes={} \
             removed_incomplete_files={} removed_incomplete_bytes={} skipped_files={} \
             retained_pairs={} retained_bytes={} over_budget_bytes={}",
            outcome.removed_pairs,
            outcome.removed_bytes,
            outcome.removed_incomplete_files,
            outcome.removed_incomplete_bytes,
            outcome.skipped_files,
            outcome.retained_pairs,
            outcome.retained_bytes,
            outcome.over_budget_bytes,
        ),
        Err(error) => eprintln!("bridge: history snapshot retention failed: {error}"),
    }
}

/// Export unless the newest snapshot is younger than `max_age`. Booting used
/// to export unconditionally, which is where a development restart loop gets
/// its snapshot-per-restart growth; the skip path still prunes so an
/// over-full directory converges without waiting for the next export.
pub fn export_history_snapshot_if_stale(
    db: &Connection,
    snapshot_dir: &Path,
    max_age: std::time::Duration,
) -> Result<Option<(PathBuf, PathBuf)>, BridgeError> {
    let newest_manifest = valid_snapshot_pairs(snapshot_dir)
        .into_iter()
        .max_by(|left, right| left.database_file.cmp(&right.database_file));
    if let Some(pair) = newest_manifest {
        let age = chrono::DateTime::parse_from_rfc3339(&pair.created_at)
            .ok()
            .map(|created| Utc::now().signed_duration_since(created));
        if age.is_some_and(|age| {
            age >= chrono::Duration::zero()
                && age.to_std().is_ok_and(|elapsed| elapsed < max_age)
        }) {
            prune_history_snapshots_and_report(snapshot_dir);
            return Ok(None);
        }
    }
    export_history_snapshot(db, snapshot_dir).map(Some)
}

pub fn verify_history_snapshot(
    database_path: &Path,
    manifest_path: &Path,
) -> Result<bool, BridgeError> {
    let manifest: HistorySnapshotManifest = serde_json::from_slice(&std::fs::read(manifest_path)?)
        .map_err(|error| BridgeError::Invalid(error.to_string()))?;
    if manifest.schema_version != 1
        || database_path.file_name().and_then(|name| name.to_str())
            != Some(manifest.database_file.as_str())
    {
        return Ok(false);
    }
    Ok(hash_file_streaming(database_path)? == manifest.sha256)
}

/// Constant-memory SHA-256 of a file, chunked through a buffered reader. The
/// previous `fs::read` pulled the whole vacuumed database into one allocation
/// on every snapshot, an allocation spike that grows with total history.
fn hash_file_streaming(path: &Path) -> Result<String, BridgeError> {
    use std::io::BufRead;
    let file = std::fs::File::open(path)?;
    let mut reader = std::io::BufReader::with_capacity(64 * 1024, file);
    let mut hasher = Sha256::new();
    loop {
        let chunk = reader.fill_buf()?;
        if chunk.is_empty() {
            break;
        }
        hasher.update(chunk);
        let consumed = chunk.len();
        reader.consume(consumed);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HistorySnapshotRetention {
    /// Newest snapshots always kept.
    pub keep_recent: usize,
    /// Beyond those, the newest snapshot from each of this many further
    /// distinct days is kept. A day counts only when a snapshot from it
    /// actually survives, so the ladder is reachable whatever `keep_recent`
    /// covers.
    pub keep_daily_days: usize,
    /// Ceiling for retained snapshot pairs. The newest pair survives even when
    /// it alone exceeds the budget, and that breach is reported in
    /// `SnapshotPruneOutcome::over_budget_bytes`. Once a pair would cross the
    /// ceiling, nothing older is retained: the budget never trades a newer
    /// recovery point for older, smaller ones.
    pub max_total_bytes: u64,
    /// Interrupted exports are only removed after this grace period so a
    /// running export can never be mistaken for stale storage.
    pub incomplete_grace: std::time::Duration,
}

impl Default for HistorySnapshotRetention {
    fn default() -> Self {
        Self {
            // A snapshot is a complete database copy, not an incremental
            // backup. Keep a short recent window plus a week-long daily ladder
            // for late-noticed corruption, and let the byte budget be the
            // final authority as the database grows.
            keep_recent: 4,
            keep_daily_days: 7,
            max_total_bytes: 2 * 1024 * 1024 * 1024,
            incomplete_grace: std::time::Duration::from_secs(60 * 60),
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SnapshotPruneOutcome {
    pub removed_pairs: usize,
    pub removed_bytes: u64,
    pub removed_incomplete_files: usize,
    pub removed_incomplete_bytes: u64,
    /// Files retention decided to delete but could not. Deletion is best
    /// effort per file: one undeletable file must never stall the rest.
    pub skipped_files: usize,
    pub retained_pairs: usize,
    pub retained_bytes: u64,
    /// How far the retained set sits above `max_total_bytes`. Non-zero only
    /// when the newest pair alone is larger than the budget.
    pub over_budget_bytes: u64,
}

impl SnapshotPruneOutcome {
    /// True when nothing happened that deserves a log line.
    pub fn is_quiet(&self) -> bool {
        self.removed_pairs == 0
            && self.removed_incomplete_files == 0
            && self.skipped_files == 0
            && self.over_budget_bytes == 0
    }
}

struct SnapshotPair {
    manifest_path: PathBuf,
    database_path: PathBuf,
    database_file: String,
    created_at: String,
}

/// Snapshots with a valid manifest/database pairing. Pairs this version
/// cannot vouch for are simply not retention candidates; they are never
/// treated as debris on that basis (see
/// `is_reclaimable_incomplete_snapshot_artifact`).
fn valid_snapshot_pairs(snapshot_dir: &Path) -> Vec<SnapshotPair> {
    let Ok(entries) = std::fs::read_dir(snapshot_dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let manifest_path = entry.path();
            let name = manifest_path.file_name()?.to_str()?;
            if !name.starts_with("bridge-history-") || !name.ends_with(".manifest.json") {
                return None;
            }
            let manifest: HistorySnapshotManifest =
                serde_json::from_slice(&std::fs::read(&manifest_path).ok()?).ok()?;
            if manifest.schema_version != 1 {
                return None;
            }
            let database_path = snapshot_dir.join(&manifest.database_file);
            if !manifest.database_file.starts_with("bridge-history-")
                || !manifest.database_file.ends_with(".sqlite")
                || !database_path.is_file()
            {
                return None;
            }
            Some(SnapshotPair {
                manifest_path,
                database_path,
                database_file: manifest.database_file,
                created_at: manifest.created_at,
            })
        })
        .collect()
}

/// Delete snapshots beyond the retention policy, newest first. Valid pairs
/// follow the recency/day policy under a byte ceiling; incomplete Bridge
/// artifacts are reclaimed after `incomplete_grace`; foreign files and pairs
/// this version cannot read are never touched. Every deletion is best effort:
/// a file that will not go is counted in `skipped_files` and the pass moves
/// on, so one stuck file cannot turn retention back into unbounded growth.
pub fn prune_history_snapshots(
    snapshot_dir: &Path,
    retention: HistorySnapshotRetention,
) -> Result<SnapshotPruneOutcome, BridgeError> {
    let mut pairs = valid_snapshot_pairs(snapshot_dir);
    // The file name embeds the UTC timestamp, so name order is time order.
    pairs.sort_by(|left, right| right.database_file.cmp(&left.database_file));
    let mut days_represented = std::collections::BTreeSet::new();
    let mut daily_days_kept = 0_usize;
    let mut budget_exhausted = false;
    let mut retained_paths = std::collections::HashSet::new();
    let mut outcome = SnapshotPruneOutcome::default();
    for (index, pair) in pairs.iter().enumerate() {
        let day = pair
            .database_file
            .get("bridge-history-".len().."bridge-history-".len() + 8)
            .unwrap_or_default()
            .to_owned();
        let in_recent_window = index < retention.keep_recent;
        let keep_by_age = in_recent_window
            || (daily_days_kept < retention.keep_daily_days && !days_represented.contains(&day));
        let bytes = std::fs::metadata(&pair.database_path)
            .map(|meta| meta.len())
            .unwrap_or_default()
            + std::fs::metadata(&pair.manifest_path)
                .map(|meta| meta.len())
                .unwrap_or_default();
        let within_budget = outcome.retained_pairs == 0
            || outcome.retained_bytes.saturating_add(bytes) <= retention.max_total_bytes;
        if keep_by_age && !within_budget {
            // Stop at the ceiling rather than packing: an older pair must
            // never survive in place of the newer one that did not fit.
            budget_exhausted = true;
        }
        if keep_by_age && !budget_exhausted {
            // Day bookkeeping happens only for pairs that actually survive, so
            // a pair the budget vetoes cannot mark its day as covered.
            if !in_recent_window {
                daily_days_kept += 1;
            }
            days_represented.insert(day);
            outcome.retained_pairs += 1;
            outcome.retained_bytes = outcome.retained_bytes.saturating_add(bytes);
            retained_paths.insert(pair.database_path.clone());
            retained_paths.insert(pair.manifest_path.clone());
            continue;
        }
        // Database first: if it will not go, the manifest stays so the pair
        // remains a valid, verifiable snapshot rather than a torn one. A
        // manifest that then fails to go is an orphan the reclaim below (or a
        // later pass) picks up.
        if std::fs::remove_file(&pair.database_path).is_err() {
            outcome.skipped_files += 1;
            continue;
        }
        if std::fs::remove_file(&pair.manifest_path).is_err() {
            outcome.skipped_files += 1;
        }
        outcome.removed_pairs += 1;
        outcome.removed_bytes += bytes;
    }
    outcome.over_budget_bytes = outcome
        .retained_bytes
        .saturating_sub(retention.max_total_bytes);
    reclaim_incomplete_snapshot_artifacts(
        snapshot_dir,
        retention.incomplete_grace,
        &retained_paths,
        &mut outcome,
    );
    Ok(outcome)
}

/// Remove stale files from a torn snapshot publication. Names are deliberately
/// narrow and the directory is Bridge-owned, so user files in the snapshot
/// directory remain outside this cleanup boundary. `retained_paths` is the set
/// the pair loop just kept, so this pass reuses that scan instead of parsing
/// every manifest a second time.
fn reclaim_incomplete_snapshot_artifacts(
    snapshot_dir: &Path,
    grace: std::time::Duration,
    retained_paths: &std::collections::HashSet<PathBuf>,
    outcome: &mut SnapshotPruneOutcome,
) {
    let Ok(entries) = std::fs::read_dir(snapshot_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if retained_paths.contains(&path) || !is_reclaimable_incomplete_snapshot_artifact(&path) {
            continue;
        }
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if !metadata.is_file() || !is_older_than(&metadata, grace) {
            continue;
        }
        let bytes = metadata.len();
        if std::fs::remove_file(&path).is_err() {
            outcome.skipped_files += 1;
            continue;
        }
        outcome.removed_incomplete_files += 1;
        outcome.removed_incomplete_bytes += bytes;
    }
}

/// Return true only when an artifact is provably debris from an interrupted
/// publication by this manifest version: a Bridge-prefixed pending file, a
/// final-named database with no manifest file of the same stem at all, or a
/// readable schema-1 manifest whose database is gone. The load-bearing
/// distinction is "no manifest exists" versus "a manifest exists that this
/// version cannot read": the latter is data from a newer Bridge (or a
/// corrupted file whose checksum exists to make corruption visible) and must
/// survive unchanged.
fn is_reclaimable_incomplete_snapshot_artifact(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    if name.starts_with(PENDING_PREFIX)
        && (name.ends_with(".sqlite.tmp") || name.ends_with(".manifest.tmp"))
    {
        return true;
    }
    if !name.starts_with("bridge-history-") {
        return false;
    }
    if let Some(stem) = name.strip_suffix(".sqlite") {
        // A same-stem manifest, even one this version cannot parse, proves
        // the database is not an abandoned manifest-less export.
        return !path
            .with_file_name(format!("{stem}.manifest.json"))
            .exists();
    }
    let Some(stem) = name.strip_suffix(".manifest.json") else {
        return false;
    };
    if path.with_file_name(format!("{stem}.sqlite")).exists() {
        return false;
    }
    let Ok(bytes) = std::fs::read(path) else {
        return false;
    };
    let Ok(manifest) = serde_json::from_slice::<HistorySnapshotManifest>(&bytes) else {
        return false;
    };
    if manifest.schema_version != 1
        || !manifest.database_file.starts_with("bridge-history-")
        || !manifest.database_file.ends_with(".sqlite")
    {
        return false;
    }
    !path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(manifest.database_file)
        .exists()
}

fn is_older_than(metadata: &std::fs::Metadata, grace: std::time::Duration) -> bool {
    metadata
        .modified()
        .ok()
        .and_then(|modified| std::time::SystemTime::now().duration_since(modified).ok())
        .is_some_and(|age| age >= grace)
}

/// Count and total bytes of everything under the snapshot directory, for
/// health diagnostics.
pub fn history_snapshot_stats(snapshot_dir: &Path) -> (u64, u64) {
    let Ok(entries) = std::fs::read_dir(snapshot_dir) else {
        return (0, 0);
    };
    entries.flatten().fold((0, 0), |(count, bytes), entry| {
        let size = entry.metadata().map(|meta| meta.len()).unwrap_or_default();
        (count + 1, bytes + size)
    })
}

fn run_migrations(connection: &mut Connection, path: &Path) -> Result<Option<PathBuf>, BridgeError> {
    let current = supported_schema_version(connection)?;
    if current == LATEST_SCHEMA_VERSION {
        return Ok(None);
    }

    let mut migration_backup = None;
    if has_user_schema(connection)? && path != Path::new(":memory:") {
        let (busy, _, _): (i64, i64, i64) =
            connection.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })?;
        if busy != 0 {
            return Err(BridgeError::Invalid(
                "database WAL is busy; refusing to create an incomplete migration backup".into(),
            ));
        }
        migration_backup = Some(backup_database(connection, path)?);
    }

    for version in (current + 1)..=LATEST_SCHEMA_VERSION {
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        match version {
            1 => migration_1_current_schema(&transaction)?,
            2 => migration_2_session_forest(&transaction)?,
            3 => migration_3_capability_tiers(&transaction)?,
            4 => migration_4_resume_eligibility(&transaction)?,
            5 => migration_5_durable_worker_pool(&transaction)?,
            6 => migration_6_remove_legacy_agent_events(&transaction)?,
            7 => migration_7_optional_repo_and_direct_chats(&transaction)?,
            8 => migration_8_reliability_primitives(&transaction)?,
            9 => migration_9_semantic_event_version(&transaction)?,
            10 => migration_10_continuation_fidelity(&transaction)?,
            11 => migration_11_human_blocked_queue(&transaction)?,
            12 => migration_12_adapter_process_claims(&transaction)?,
            13 => migration_13_learning_router(&transaction)?,
            14 => migration_14_completion_proof(&transaction)?,
            15 => migration_15_role_profiles_and_learning_jobs(&transaction)?,
            16 => migration_16_complete_role_profile_schema(&transaction)?,
            17 => migration_17_configuration_entries(&transaction)?,
            18 => migration_18_prompt_cache_telemetry(&transaction)?,
            19 => migration_19_repair_learning_router_schema(&transaction)?,
            20 => migration_20_repair_legacy_learning_constraints(&transaction)?,
            21 => migration_21_approval_deadlines_and_worktree_adoption(&transaction)?,
            22 => migration_22_session_backend_binding(&transaction)?,
            23 => migration_23_session_title_source(&transaction)?,
            24 => migration_24_work_board(&transaction)?,
            25 => migration_25_ephemeral_work_evidence(&transaction)?,
            26 => migration_26_briefing_run_leases(&transaction)?,
            27 => migration_27_queued_session_input(&transaction)?,
            28 => migration_28_evidence_based_retries(&transaction)?,
            29 => migration_29_learning_scope(&transaction)?,
            30 => migration_30_session_entry_fts(&transaction)?,
            31 => migration_31_memory_ledger(&transaction)?,
            32 => migration_32_worker_progress_summary(&transaction)?,
            33 => crate::prompt_sections::install_revision_store(&transaction)?,
            34 => migration_34_memory_lifecycle(&transaction)?,
            35 => migration_35_memory_extraction(&transaction)?,
            36 => migration_36_memory_packet(&transaction)?,
            37 => migration_37_family_only_preferences(&transaction)?,
            38 => migration_38_routing_catalogs(&transaction)?,
            39 => migration_39_learning_tunables(&transaction)?,
            40 => migration_40_prompt_compilation_accounting(&transaction)?,
            41 => migration_41_routing_evaluation_runs(&transaction)?,
            42 => migration_42_memory_consolidation(&transaction)?,
            43 => migration_43_interaction_resolutions(&transaction)?,
            44 => migration_44_agent_usage_analytics(&transaction)?,
            45 => migration_45_latest_memory_packet_audit(&transaction)?,
            46 => crate::external_import::install_import_foundation(&transaction)?,
            47 => migration_47_model_profile_selection_mode(&transaction)?,
            48 => migration_48_harness_quota_cooldowns(&transaction)?,
            49 => migration_49_worker_repair_budget(&transaction)?,
            50 => migration_50_worktree_inventory(&transaction)?,
            51 => migration_51_archived_chats(&transaction)?,
            52 => migration_52_worker_failure_class(&transaction)?,
            53 => migration_53_usage_tracking(&transaction)?,
            54 => migration_54_usage_ledger_repair(&transaction)?,
            55 => {
                crate::prompt_sections::install_guidance_revision_store(&transaction)?;
                crate::prompt_mutations::install_store(&transaction)?;
            }
            _ => {
                return Err(BridgeError::Invalid(format!(
                    "unknown schema migration {version}"
                )))
            }
        }
        transaction.execute(
            "INSERT INTO schema_version(version, applied_at) VALUES(?1, ?2)",
            params![version, Utc::now().to_rfc3339()],
        )?;
        transaction.commit()?;
    }
    Ok(migration_backup)
}

fn migration_47_model_profile_selection_mode(
    transaction: &Transaction<'_>,
) -> Result<(), BridgeError> {
    add_column_if_missing(
        transaction,
        "model_profiles",
        "selection_mode",
        "TEXT NOT NULL DEFAULT 'track_standard'",
    )?;
    transaction.execute(
        "UPDATE model_profiles
         SET selection_mode=CASE WHEN pinned=1 THEN 'pinned' ELSE 'track_standard' END",
        [],
    )?;
    Ok(())
}

/// Persist the repair budget and preserve attempts spent before this migration.
/// The worktree inventory. Bridge cut worktrees from four places and recorded
/// them — if at all — in whichever table happened to need one, so no query
/// could answer "what exists on this disk, and who owns it". Orchestrator
/// checkouts had no record of any kind: they were named only by `sessions.cwd`,
/// which nothing consulted, so nothing could ever reclaim one.
///
/// Backfill works off recorded paths rather than the data directory, which this
/// layer does not know. Anything it misses is still found later: the reconcile
/// pass adopts unrecorded directories under the namespace root.
/// Archiving a chat. Bridge could archive a *workspace* — which deletes every
/// session in it — but had no way to put one conversation away, so the only
/// route to reclaiming an isolated chat's checkout was a destructive operation
/// on hundreds of unrelated chats. This is the per-chat marker: history is kept,
/// the conversation is simply no longer listed.
fn migration_51_archived_chats(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    add_column_if_missing(transaction, "sessions", "archived_at", "TEXT")
}

fn migration_50_worktree_inventory(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS worktrees (
            id TEXT PRIMARY KEY,
            kind TEXT NOT NULL,
            repo_root TEXT NOT NULL,
            path TEXT NOT NULL UNIQUE,
            branch TEXT,
            owner_session_id TEXT,
            owner_workspace_id TEXT,
            base_commit TEXT,
            state TEXT NOT NULL,
            disposition TEXT,
            retained_reason TEXT,
            assessed_at TEXT,
            size_bytes INTEGER,
            size_measured_at TEXT,
            created_at TEXT NOT NULL,
            last_used_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_worktrees_repo ON worktrees(repo_root,state);
        CREATE INDEX IF NOT EXISTS idx_worktrees_state ON worktrees(state,last_used_at);",
    )?;

    // Worker checkouts: the adoption row is the richer record (it carries the
    // task worktree and the base commit), so it wins where both exist.
    transaction.execute(
        "INSERT OR IGNORE INTO worktrees(
            id,kind,repo_root,path,branch,owner_session_id,owner_workspace_id,
            base_commit,state,retained_reason,created_at,last_used_at)
         SELECT lower(hex(randomblob(16))),'worker',a.task_worktree_path,a.worktree_path,
                a.worktree_branch,a.session_id,a.workspace_id,a.base_commit,'idle',
                'backfilled from an adoption record',a.created_at,a.updated_at
           FROM worker_worktree_adoptions a
          WHERE COALESCE(a.worktree_path,'')<>''
            AND a.worktree_path<>a.task_worktree_path",
        [],
    )?;
    transaction.execute(
        "INSERT OR IGNORE INTO worktrees(
            id,kind,repo_root,path,branch,owner_session_id,owner_workspace_id,
            base_commit,state,retained_reason,created_at,last_used_at)
         SELECT lower(hex(randomblob(16))),'worker',
                COALESCE((SELECT w.path FROM sessions s JOIN workspaces w ON w.id=s.workspace_id
                           WHERE s.id=r.session_id),''),
                r.worktree_path,r.worktree_branch,r.session_id,
                (SELECT s.workspace_id FROM sessions s WHERE s.id=r.session_id),
                NULL,'idle','backfilled from a worker runtime record',
                COALESCE(r.updated_at,?1),COALESCE(r.last_activity_at,r.updated_at,?1)
           FROM worker_runtime r
          WHERE COALESCE(r.worktree_path,'')<>''
            -- An in-place worker writes into the user's own checkout rather than
            -- one Bridge cut, so its recorded path can be the repository itself.
            -- Claiming that would put a main working tree in front of a reclaim
            -- decision.
            AND r.worktree_path<>COALESCE(
                (SELECT w.path FROM sessions s JOIN workspaces w ON w.id=s.workspace_id
                  WHERE s.id=r.session_id),'')",
        params![Utc::now().to_rfc3339()],
    )?;

    // Pull-request checkouts: registered as workspace nodes whose path sits
    // under the worktrees namespace.
    transaction.execute(
        "INSERT OR IGNORE INTO worktrees(
            id,kind,repo_root,path,branch,owner_session_id,owner_workspace_id,
            base_commit,state,retained_reason,created_at,last_used_at)
         SELECT lower(hex(randomblob(16))),'github','',w.path,w.branch,NULL,w.id,NULL,
                'idle','backfilled from a pull-request checkout',
                COALESCE(w.created_at,?1),COALESCE(w.created_at,?1)
           FROM workspaces w
          WHERE COALESCE(w.path,'')<>'' AND w.path LIKE '%/worktrees/github/%'",
        params![Utc::now().to_rfc3339()],
    )?;

    // Orchestrator checkouts, recognisable only by the path convention they
    // were created with. This is the class that previously leaked permanently.
    transaction.execute(
        "INSERT OR IGNORE INTO worktrees(
            id,kind,repo_root,path,branch,owner_session_id,owner_workspace_id,
            base_commit,state,retained_reason,created_at,last_used_at)
         SELECT lower(hex(randomblob(16))),'orchestrator',
                COALESCE((SELECT w.path FROM workspaces w WHERE w.id=s.workspace_id),''),
                s.cwd,NULL,s.id,s.workspace_id,NULL,'idle',
                'backfilled from a session working directory',?1,?1
           FROM sessions s
          WHERE COALESCE(s.cwd,'')<>'' AND s.cwd LIKE '%/worktrees/orchestrators/%'",
        params![Utc::now().to_rfc3339()],
    )?;
    Ok(())
}

/// Bridge's own verdict on why a worker failed, kept next to the result
/// rather than re-derived from it.
///
/// The UI used to decide "is this a stall?" with `/stopped responding/i` over
/// the summary — a regex against a Rust format string, so rewording one
/// `format!` silently downgraded every stall to a generic failure. And the
/// classification it was trying to recover is not in the summary anyway:
/// a stall is something Bridge observed, not something the worker reported.
/// Per-request usage tracking: the ledger learns the token breakdown and
/// provenance fields the summary needs, user price overrides get a table, and
/// the explicitly refreshed rate table gets one cached row. Additive only —
/// every existing ledger row keeps its values.
fn migration_53_usage_tracking(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    add_column_if_missing(transaction, "usage_ledger", "reasoning_tokens", "INTEGER")?;
    add_column_if_missing(transaction, "usage_ledger", "serving_model", "TEXT")?;
    add_column_if_missing(transaction, "usage_ledger", "context_window_tokens", "INTEGER")?;
    add_column_if_missing(transaction, "usage_ledger", "context_used_tokens", "INTEGER")?;
    add_column_if_missing(transaction, "usage_ledger", "provider_record_id", "TEXT")?;
    add_column_if_missing(transaction, "usage_ledger", "cache_savings_microusd", "INTEGER")?;
    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS usage_price_overrides (
            model TEXT PRIMARY KEY,
            input_microusd_per_mtok INTEGER NOT NULL CHECK (input_microusd_per_mtok >= 0),
            output_microusd_per_mtok INTEGER NOT NULL CHECK (output_microusd_per_mtok >= 0),
            cache_read_microusd_per_mtok INTEGER CHECK (cache_read_microusd_per_mtok IS NULL OR cache_read_microusd_per_mtok >= 0),
            cache_write_microusd_per_mtok INTEGER CHECK (cache_write_microusd_per_mtok IS NULL OR cache_write_microusd_per_mtok >= 0),
            updated_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS usage_rate_cache (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            fetched_at TEXT NOT NULL,
            source_url TEXT NOT NULL,
            body TEXT NOT NULL
        );",
    )?;
    Ok(())
}

/// One-shot repair of Claude costs recorded before the per-turn normalizer.
/// Codex rows have no persisted cumulative/per-request discriminator: neither
/// timestamps nor monotonic counts prove provenance. Leave them unchanged;
/// imported provider history supplies the per-request records instead.
///
/// Population note: the schema version was deliberately not bumped for the
/// Codex half of this repair, so two populations exist and stay as they are.
/// Databases that ran the earlier timestamp-gated Codex repair keep its delta
/// rows (per-turn figures, the honest shape); databases upgrading now keep
/// their raw cumulative rows until a history scan imports per-request
/// observations for those sessions. Guessing a discriminator to reunite them
/// would risk silently undercounting valid usage, which is worse than the
/// split — so the split is documented here instead of repaired.
fn migration_54_usage_ledger_repair(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    if !table_exists(transaction, "usage_ledger")? {
        return Ok(());
    }
    repair_claude_cumulative_cost(transaction)?;
    Ok(())
}

fn repair_claude_cumulative_cost(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    let mut statement = transaction.prepare(
        "SELECT id, COALESCE(session_id,''), COALESCE(model,''), cost_microusd FROM usage_ledger
         WHERE source='provider.claude' AND cost_microusd IS NOT NULL
         ORDER BY session_id, model, id",
    )?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, i64>(3)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let mut previous: Option<(String, String, i64)> = None;
    for (id, session, model, cost) in rows {
        let turn = match &previous {
            Some((prev_session, prev_model, prev_cost)) if *prev_session == session && *prev_model == model && cost >= *prev_cost => cost - prev_cost,
            _ => cost,
        };
        if turn != cost {
            transaction.execute("UPDATE usage_ledger SET cost_microusd=?1 WHERE id=?2", params![turn, id])?;
        }
        previous = Some((session, model, cost));
    }
    Ok(())
}

fn migration_52_worker_failure_class(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    add_column_if_missing(transaction, "worker_runtime", "failure_class", "TEXT")?;
    Ok(())
}

fn migration_49_worker_repair_budget(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    add_column_if_missing(
        transaction,
        "worker_runtime",
        "result_repair_count",
        "INTEGER NOT NULL DEFAULT 0",
    )?;
    transaction.execute(
        "UPDATE worker_runtime SET result_repair_count=1
         WHERE EXISTS(SELECT 1 FROM events WHERE entity_id=worker_runtime.session_id
                      AND kind='worker.result.repair_requested')",
        [],
    )?;
    Ok(())
}

/// A harness that just told Bridge it is out of quota is not "unknown" —
/// `learning_router::harness_capacity` otherwise only sees usage/context from
/// *live* sessions, so the signal would vanish the moment the failed worker's
/// session ends. This is the durable record that survives it: a per-workspace
/// cooldown the router checks in addition to live session state, so the next
/// delegation in this workspace routes around a harness that just failed for
/// quota reasons instead of picking it again and hitting the same wall.
fn migration_48_harness_quota_cooldowns(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS harness_quota_cooldowns (
            workspace_id TEXT NOT NULL,
            harness TEXT NOT NULL,
            reason TEXT NOT NULL,
            exhausted_at TEXT NOT NULL,
            cooldown_until TEXT NOT NULL,
            PRIMARY KEY (workspace_id, harness)
        );",
    )?;
    Ok(())
}

fn migration_44_agent_usage_analytics(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    // This index intentionally has no foreign keys to workspaces or the
    // Bridge-owned usage ledger. It represents source-owned, device-wide facts
    // and must survive workspace deletion or source-file disappearance.
    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS agent_usage_sources (
            id TEXT PRIMARY KEY,
            agent TEXT NOT NULL,
            provider TEXT NOT NULL,
            location_fingerprint TEXT NOT NULL UNIQUE,
            detected_version TEXT,
            format_version TEXT,
            scan_cursor TEXT,
            coverage_start_at TEXT,
            coverage_end_at TEXT,
            coverage_state TEXT NOT NULL CHECK (coverage_state IN ('complete','partial','stale','unsupported','unreadable','empty')),
            coverage_reason TEXT,
            records_imported INTEGER NOT NULL DEFAULT 0 CHECK (records_imported >= 0),
            records_skipped INTEGER NOT NULL DEFAULT 0 CHECK (records_skipped >= 0),
            last_successful_scan_at TEXT,
            last_error TEXT,
            importer_version TEXT NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_agent_usage_sources_coverage ON agent_usage_sources(coverage_state,updated_at DESC);

        CREATE TABLE IF NOT EXISTS agent_usage_sessions (
            id TEXT PRIMARY KEY,
            source_id TEXT NOT NULL REFERENCES agent_usage_sources(id),
            native_session_id TEXT NOT NULL,
            parent_native_session_id TEXT,
            agent TEXT NOT NULL,
            provider TEXT NOT NULL,
            model TEXT,
            project_label TEXT,
            project_path_fingerprint TEXT,
            session_type TEXT,
            started_at TEXT,
            ended_at TEXT,
            outcome TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            UNIQUE(source_id,native_session_id)
        );
        CREATE INDEX IF NOT EXISTS idx_agent_usage_sessions_source_time ON agent_usage_sessions(source_id,started_at DESC);

        CREATE TABLE IF NOT EXISTS agent_usage_observations (
            id TEXT PRIMARY KEY,
            source_id TEXT NOT NULL REFERENCES agent_usage_sources(id),
            session_id TEXT REFERENCES agent_usage_sessions(id),
            native_record_id TEXT NOT NULL,
            occurred_at TEXT NOT NULL,
            model TEXT,
            input_semantics TEXT NOT NULL,
            output_semantics TEXT NOT NULL,
            total_input_tokens INTEGER CHECK (total_input_tokens IS NULL OR total_input_tokens >= 0),
            uncached_input_tokens INTEGER CHECK (uncached_input_tokens IS NULL OR uncached_input_tokens >= 0),
            cache_read_tokens INTEGER CHECK (cache_read_tokens IS NULL OR cache_read_tokens >= 0),
            cache_write_tokens INTEGER CHECK (cache_write_tokens IS NULL OR cache_write_tokens >= 0),
            output_tokens INTEGER CHECK (output_tokens IS NULL OR output_tokens >= 0),
            reasoning_tokens INTEGER CHECK (reasoning_tokens IS NULL OR reasoning_tokens >= 0),
            tool_use_tokens INTEGER CHECK (tool_use_tokens IS NULL OR tool_use_tokens >= 0),
            provider_reported_total_tokens INTEGER CHECK (provider_reported_total_tokens IS NULL OR provider_reported_total_tokens >= 0),
            exact_total_formula TEXT NOT NULL,
            reported_cost_microusd INTEGER CHECK (reported_cost_microusd IS NULL OR reported_cost_microusd >= 0),
            calculated_cost_microusd INTEGER CHECK (calculated_cost_microusd IS NULL OR calculated_cost_microusd >= 0),
            cost_source TEXT,
            pricing_source TEXT,
            pricing_version TEXT,
            pricing_effective_at TEXT,
            pricing_model_id TEXT,
            pricing_rates_json TEXT,
            numeric_usage_json TEXT NOT NULL DEFAULT '{}',
            source_file_fingerprint TEXT,
            importer_version TEXT NOT NULL,
            created_at TEXT NOT NULL,
            UNIQUE(source_id,native_record_id)
        );
        CREATE INDEX IF NOT EXISTS idx_agent_usage_observations_time ON agent_usage_observations(occurred_at DESC);
        CREATE INDEX IF NOT EXISTS idx_agent_usage_observations_source_session ON agent_usage_observations(source_id,session_id,occurred_at DESC);

        CREATE TABLE IF NOT EXISTS agent_usage_attributions (
            id TEXT PRIMARY KEY,
            source_id TEXT NOT NULL REFERENCES agent_usage_sources(id),
            session_id TEXT REFERENCES agent_usage_sessions(id),
            observation_id TEXT REFERENCES agent_usage_observations(id),
            attribution_kind TEXT NOT NULL CHECK (attribution_kind IN ('skill','slash_command','tool','mcp_server','agent_role')),
            attribution_value TEXT NOT NULL,
            attribution_source TEXT NOT NULL,
            created_at TEXT NOT NULL,
            UNIQUE(source_id,session_id,observation_id,attribution_kind,attribution_value,attribution_source)
        );
        CREATE INDEX IF NOT EXISTS idx_agent_usage_attributions_lookup ON agent_usage_attributions(attribution_kind,attribution_value);",
    )?;
    Ok(())
}

fn migration_17_configuration_entries(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    transaction.execute_batch(
        "CREATE TABLE configuration_entries (
            kind TEXT NOT NULL,
            id TEXT NOT NULL,
            payload TEXT NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            PRIMARY KEY(kind,id)
        );
        CREATE INDEX idx_configuration_entries_kind ON configuration_entries(kind,updated_at);",
    )?;
    Ok(())
}

fn migration_18_prompt_cache_telemetry(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    add_column_if_missing(
        transaction,
        "usage_ledger",
        "uncached_input_tokens",
        "INTEGER",
    )?;
    add_column_if_missing(transaction, "usage_ledger", "stable_prefix_id", "TEXT")?;
    add_column_if_missing(transaction, "usage_ledger", "stable_prefix_hash", "TEXT")?;
    add_column_if_missing(
        transaction,
        "usage_ledger",
        "prompt_schema_version",
        "INTEGER",
    )?;
    add_column_if_missing(
        transaction,
        "usage_ledger",
        "prefix_token_estimate",
        "INTEGER",
    )?;
    add_column_if_missing(transaction, "usage_ledger", "harness", "TEXT")?;
    add_column_if_missing(transaction, "usage_ledger", "model", "TEXT")?;
    add_column_if_missing(transaction, "usage_ledger", "role", "TEXT")?;
    add_column_if_missing(transaction, "usage_ledger", "task_family", "TEXT")?;
    add_column_if_missing(transaction, "usage_ledger", "restoration_mode", "TEXT")?;
    add_column_if_missing(transaction, "usage_ledger", "cross_harness_reuse", "TEXT")?;
    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS prompt_compilations (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
            turn_id TEXT,
            prefix_id TEXT NOT NULL,
            prefix_hash TEXT NOT NULL,
            schema_version INTEGER NOT NULL,
            prefix_bytes INTEGER NOT NULL,
            prefix_token_estimate INTEGER NOT NULL,
            harness TEXT NOT NULL,
            model TEXT,
            role TEXT NOT NULL,
            task_family TEXT NOT NULL,
            restoration_mode TEXT NOT NULL,
            cross_harness_reuse TEXT NOT NULL,
            created_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_prompt_compilations_session
            ON prompt_compilations(session_id,id DESC);
        CREATE INDEX IF NOT EXISTS idx_prompt_compilations_prefix
            ON prompt_compilations(harness,prefix_hash,id DESC);",
    )?;
    add_column_if_missing(transaction, "prompt_compilations", "turn_id", "TEXT")?;
    transaction.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_prompt_compilations_turn
            ON prompt_compilations(session_id,turn_id,id DESC);",
    )?;
    Ok(())
}

fn migration_40_prompt_compilation_accounting(
    transaction: &Transaction<'_>,
) -> Result<(), BridgeError> {
    add_column_if_missing(transaction, "prompt_compilations", "sections_json", "TEXT")?;
    add_column_if_missing(
        transaction,
        "prompt_compilations",
        "stable_bytes",
        "INTEGER",
    )?;
    add_column_if_missing(
        transaction,
        "prompt_compilations",
        "variable_bytes",
        "INTEGER",
    )?;
    add_column_if_missing(
        transaction,
        "prompt_compilations",
        "stable_token_estimate",
        "INTEGER",
    )?;
    add_column_if_missing(
        transaction,
        "prompt_compilations",
        "variable_token_estimate",
        "INTEGER",
    )?;
    add_column_if_missing(
        transaction,
        "prompt_compilations",
        "token_estimate_source",
        "TEXT",
    )?;
    Ok(())
}

fn migration_19_repair_learning_router_schema(
    transaction: &Transaction<'_>,
) -> Result<(), BridgeError> {
    // Migration 15 changed while several feature branches were shipping with
    // the same schema version. Databases that recorded the earlier v15 shape
    // never received the later router columns, so worker routing failed before
    // a child session could be created. Replaying the idempotent migration
    // repairs every affected table instead of only the first missing column.
    add_column_if_missing(
        transaction,
        "learning_jobs",
        "run_budget_tokens",
        "INTEGER NOT NULL DEFAULT 50000",
    )?;
    add_column_if_missing(transaction, "learning_triggers", "auth_digest", "TEXT")?;
    add_column_if_missing(transaction, "learning_triggers", "expires_at", "TEXT")?;
    add_column_if_missing(
        transaction,
        "learning_triggers",
        "experimental",
        "INTEGER NOT NULL DEFAULT 0",
    )?;
    add_column_if_missing(
        transaction,
        "learning_triggers",
        "updated_at",
        "TEXT NOT NULL DEFAULT '1970-01-01T00:00:00Z'",
    )?;
    add_column_if_missing(transaction, "learning_job_runs", "lease_owner", "TEXT")?;
    add_column_if_missing(transaction, "learning_job_runs", "lease_expires_at", "TEXT")?;
    add_column_if_missing(
        transaction,
        "learning_job_runs",
        "snapshot_frozen_at",
        "TEXT NOT NULL DEFAULT '1970-01-01T00:00:00Z'",
    )?;
    add_column_if_missing(
        transaction,
        "learning_job_runs",
        "evaluated_spend_microusd",
        "INTEGER NOT NULL DEFAULT 0",
    )?;
    add_column_if_missing(
        transaction,
        "learning_job_runs",
        "evaluated_tokens",
        "INTEGER NOT NULL DEFAULT 0",
    )?;
    add_column_if_missing(transaction, "learning_job_runs", "replay_passed", "INTEGER")?;
    add_column_if_missing(
        transaction,
        "learning_job_runs",
        "promotion_status",
        "TEXT NOT NULL DEFAULT 'not_requested'",
    )?;
    add_column_if_missing(
        transaction,
        "routing_evaluations",
        "learning_run_id",
        "TEXT REFERENCES learning_job_runs(id) ON DELETE SET NULL",
    )?;
    add_column_if_missing(
        transaction,
        "routing_evaluations",
        "decision_id",
        "TEXT REFERENCES router_decisions(id) ON DELETE CASCADE",
    )?;
    add_column_if_missing(
        transaction,
        "routing_evaluations",
        "bounded_metrics",
        "TEXT NOT NULL DEFAULT '{}'",
    )?;
    add_column_if_missing(
        transaction,
        "routing_evaluations",
        "status",
        "TEXT NOT NULL DEFAULT 'completed'",
    )?;
    add_column_if_missing(
        transaction,
        "routing_policies",
        "rollback_of",
        "INTEGER REFERENCES routing_policies(version)",
    )?;
    add_column_if_missing(transaction, "routing_policies", "replay_report", "TEXT")?;
    add_column_if_missing(transaction, "routing_policies", "promoted_at", "TEXT")?;
    add_column_if_missing(
        transaction,
        "routing_policies",
        "activation_boundary",
        "INTEGER",
    )?;
    add_column_if_missing(
        transaction,
        "learning_trigger_events",
        "registration_id",
        "TEXT",
    )?;
    add_column_if_missing(transaction, "learning_trigger_events", "reason", "TEXT")?;
    migration_15_role_profiles_and_learning_jobs(transaction)?;
    if column_exists(transaction, "routing_evaluations", "run_id")? {
        transaction.execute(
            "UPDATE routing_evaluations SET learning_run_id=run_id WHERE learning_run_id IS NULL",
            [],
        )?;
    }
    migration_16_complete_role_profile_schema(transaction)
}

fn migration_20_repair_legacy_learning_constraints(
    transaction: &Transaction<'_>,
) -> Result<(), BridgeError> {
    // The first migration-15 shape used a required `run_id` column. Adding the
    // current columns did not relax that constraint, so inserts that correctly
    // omit the legacy field still failed after v19. Preserve its data, then
    // remove it now that `learning_run_id` is canonical.
    if table_exists(transaction, "routing_evaluations")?
        && column_exists(transaction, "routing_evaluations", "run_id")?
    {
        add_column_if_missing(
            transaction,
            "routing_evaluations",
            "learning_run_id",
            "TEXT REFERENCES learning_job_runs(id) ON DELETE SET NULL",
        )?;
        transaction.execute(
            "UPDATE routing_evaluations SET learning_run_id=run_id WHERE learning_run_id IS NULL",
            [],
        )?;
        transaction.execute_batch("ALTER TABLE routing_evaluations DROP COLUMN run_id;")?;
    }

    // Rebuild instead of ALTERing because the legacy column was NOT NULL and
    // used ON DELETE CASCADE; SQLite cannot relax either property in place.
    if table_exists(transaction, "learning_trigger_events")? {
        transaction.execute_batch(
            "CREATE TABLE learning_trigger_events_v20 (
                id TEXT PRIMARY KEY,
                run_id TEXT REFERENCES learning_job_runs(id) ON DELETE SET NULL,
                trigger_kind TEXT NOT NULL,
                registration_id TEXT,
                result TEXT NOT NULL,
                reason TEXT,
                created_at TEXT NOT NULL
            );
            INSERT INTO learning_trigger_events_v20(id,run_id,trigger_kind,registration_id,result,reason,created_at)
                SELECT id,run_id,trigger_kind,registration_id,result,reason,created_at
                FROM learning_trigger_events;
            DROP TABLE learning_trigger_events;
            ALTER TABLE learning_trigger_events_v20 RENAME TO learning_trigger_events;",
        )?;
    }

    // Index names, unlike definitions, satisfy IF NOT EXISTS. Recreate this
    // invariant explicitly so an active policy and a canary cannot coexist.
    if table_exists(transaction, "routing_policies")? {
        transaction.execute_batch(
            "DROP INDEX IF EXISTS idx_routing_policy_active;
             CREATE UNIQUE INDEX idx_routing_policy_active
                ON routing_policies((1)) WHERE status IN ('active','canary');",
        )?;
    }

    add_column_if_missing(transaction, "worker_runtime", "last_activity_at", "TEXT")
}

fn migration_21_approval_deadlines_and_worktree_adoption(
    transaction: &Transaction<'_>,
) -> Result<(), BridgeError> {
    // `waiting` workers were excluded from every watchdog, so an unanswered
    // in-session approval left the worker pending forever. Stamping the entry
    // time gives the approval deadline something durable to measure.
    add_column_if_missing(transaction, "worker_runtime", "waiting_since", "TEXT")?;
    add_column_if_missing(transaction, "worker_runtime", "waiting_reason", "TEXT")?;
    // Completion evidence used to be a bare HEAD string, which cannot say what
    // the change is relative to or which worker revision produced it.
    add_column_if_missing(transaction, "eval_attempts", "base_ref", "TEXT")?;
    add_column_if_missing(transaction, "eval_attempts", "base_commit", "TEXT")?;
    add_column_if_missing(transaction, "eval_attempts", "worker_branch", "TEXT")?;
    add_column_if_missing(transaction, "eval_attempts", "worker_session_id", "TEXT")?;
    // Why an attempt ended, when it ended for a reason other than its checks.
    // A gate that could not be built must keep failing closed; a gate that ran
    // out of time must release the parent so it can report.
    add_column_if_missing(transaction, "eval_attempts", "escalation", "TEXT")?;
    // Verified work that lives only in a child worktree must not silently
    // disappear: it needs a durable adoption state that survives restart.
    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS worker_worktree_adoptions (
            session_id TEXT PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE,
            parent_session_id TEXT NOT NULL,
            workspace_id TEXT NOT NULL,
            worktree_path TEXT NOT NULL,
            worktree_branch TEXT NOT NULL,
            task_worktree_path TEXT NOT NULL,
            state TEXT NOT NULL,
            head TEXT,
            base_commit TEXT,
            base_branch TEXT,
            baseline_dirty_paths TEXT NOT NULL DEFAULT '[]',
            changed_paths TEXT NOT NULL DEFAULT '[]',
            diffstat TEXT,
            dirty INTEGER NOT NULL DEFAULT 0,
            detail TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_worker_worktree_adoptions_parent
            ON worker_worktree_adoptions(parent_session_id,state);",
    )?;
    Ok(())
}

/// Which backend actually served a session, and the authorization to change it.
///
/// `sessions.harness` says which agent a user picked. It has never said which
/// implementation ran, because until the marketplace there was only ever one —
/// so a session resumed after an install or a backend swap had no way to know it
/// had moved. These three columns are that missing provenance.
///
/// All nullable, and no backfill. A row written before this migration is
/// genuinely unbound rather than bound to a guess: inferring a backend for it
/// would be inventing history, and the read path treats null as "not recorded"
/// and binds it on its next successful start.
/// Records where a session's title came from, so a heading Bridge derived from the
/// first message can later be replaced by the one the harness writes, while a
/// title the user or the provider chose is never overwritten.
fn migration_23_session_title_source(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    let has_column = transaction
        .prepare("SELECT 1 FROM pragma_table_info('sessions') WHERE name='title_source'")?
        .exists([])?;
    if !has_column {
        transaction.execute_batch("ALTER TABLE sessions ADD COLUMN title_source TEXT;")?;
    }
    // Titles that predate this column were set by the user at creation, so they
    // stay untouched: an absent source is read as "not ours to replace".
    Ok(())
}

fn migration_22_session_backend_binding(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    add_column_if_missing(transaction, "sessions", "backend_id", "TEXT")?;
    add_column_if_missing(transaction, "sessions", "backend_version", "TEXT")?;
    add_column_if_missing(transaction, "sessions", "backend_installation_id", "TEXT")?;
    // A backend change is refused until it is authorized for that exact
    // transition. One pending authorization per session, consumed when it is
    // used, so it cannot be spent twice or generalize to a later change.
    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS backend_change_authorizations (
            session_id TEXT PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE,
            from_backend TEXT NOT NULL,
            to_backend TEXT NOT NULL,
            to_version TEXT,
            authorized_at TEXT NOT NULL
        );",
    )?;
    Ok(())
}

fn supported_schema_version(connection: &Connection) -> Result<i64, BridgeError> {
    let current = current_schema_version(connection)?;
    if current > LATEST_SCHEMA_VERSION {
        return Err(BridgeError::Invalid(format!(
            "This database uses schema version {current}, but this Bridge build supports up to \
             {LATEST_SCHEMA_VERSION}. Open it with a newer Bridge version."
        )));
    }
    Ok(current)
}

fn current_schema_version(connection: &Connection) -> Result<i64, BridgeError> {
    let exists: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='schema_version')",
        [],
        |row| row.get(0),
    )?;
    if !exists {
        return Ok(0);
    }
    Ok(connection.query_row(
        "SELECT COALESCE(MAX(version), 0) FROM schema_version",
        [],
        |row| row.get(0),
    )?)
}

fn has_user_schema(connection: &Connection) -> Result<bool, BridgeError> {
    Ok(connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' AND name != 'schema_version')",
        [],
        |row| row.get(0),
    )?)
}

fn backup_database(connection: &Connection, path: &Path) -> Result<PathBuf, BridgeError> {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("bridge.db");
    let suffix = Utc::now().format(MIGRATION_BACKUP_TIMESTAMP_FORMAT);
    let backup = path.with_file_name(format!("{file_name}.backup-{suffix}"));
    let pending = path.with_file_name(format!("{file_name}.backup-{suffix}.pending"));
    if let Err(error) = connection.execute("VACUUM INTO ?1", params![pending.to_string_lossy()]) {
        let _ = std::fs::remove_file(&pending);
        return Err(error.into());
    }
    if let Err(error) = std::fs::rename(&pending, &backup) {
        let _ = std::fs::remove_file(&pending);
        return Err(error.into());
    }
    Ok(backup)
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct MigrationBackupPruneOutcome {
    removed_files: usize,
    removed_bytes: u64,
    skipped_files: usize,
}

fn is_sqlite_backup(path: &Path) -> bool {
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    let mut header = [0_u8; 16];
    file.read_exact(&mut header).is_ok() && header == *b"SQLite format 3\0"
}

fn is_structurally_valid_sqlite_backup(path: &Path) -> bool {
    let Ok(connection) = Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) else {
        return false;
    };
    connection
        .query_row("PRAGMA quick_check(1)", [], |row| row.get::<_, String>(0))
        .is_ok_and(|result| result == "ok")
}

/// Keep one verified-by-name rollback point for the primary database. Schema
/// upgrades used to append a complete copy forever, so each release multiplied
/// the user's whole chat history. Names that are not exactly Bridge's timestamp
/// format are left alone rather than guessed to be disposable.
fn prune_migration_backups(
    path: &Path,
    protected_backup: Option<&Path>,
) -> Result<MigrationBackupPruneOutcome, BridgeError> {
    let Some(parent) = path.parent() else {
        return Ok(MigrationBackupPruneOutcome::default());
    };
    let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
        return Ok(MigrationBackupPruneOutcome::default());
    };
    let prefix = format!("{file_name}.backup-");
    let mut outcome = MigrationBackupPruneOutcome::default();
    let mut backups = Vec::new();
    for entry in std::fs::read_dir(parent)?.flatten() {
        if !entry.file_type().is_ok_and(|kind| kind.is_file()) {
            continue;
        }
        let candidate = entry.path();
        let Some(name) = candidate.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let Some(suffix) = name.strip_prefix(&prefix) else {
            continue;
        };
        if let Some(timestamp) = suffix.strip_suffix(".pending") {
            if chrono::NaiveDateTime::parse_from_str(timestamp, MIGRATION_BACKUP_TIMESTAMP_FORMAT)
                .is_ok()
            {
                let bytes = entry.metadata().map(|metadata| metadata.len()).unwrap_or_default();
                if std::fs::remove_file(&candidate).is_err() {
                    outcome.skipped_files += 1;
                } else {
                    outcome.removed_files += 1;
                    outcome.removed_bytes = outcome.removed_bytes.saturating_add(bytes);
                }
            }
            continue;
        }
        if chrono::NaiveDateTime::parse_from_str(suffix, MIGRATION_BACKUP_TIMESTAMP_FORMAT).is_err()
            || !is_sqlite_backup(&candidate)
        {
            continue;
        }
        backups.push(candidate);
    }
    backups.sort_by(|left, right| right.file_name().cmp(&left.file_name()));
    let keep = protected_backup
        .filter(|protected| backups.iter().any(|candidate| candidate == *protected))
        .map(Path::to_path_buf)
        .or_else(|| match backups.as_slice() {
            [] => None,
            [only] => Some(only.clone()),
            competing => competing
                .iter()
                .find(|candidate| is_structurally_valid_sqlite_backup(candidate))
                .cloned(),
        });

    // If no competing candidate is structurally readable, preserve them all.
    // Failing to reclaim space is safer than deleting the last possible rollback.
    if keep.is_none() && !backups.is_empty() {
        outcome.skipped_files = outcome.skipped_files.saturating_add(backups.len());
        return Ok(outcome);
    }

    for backup in backups {
        if keep.as_ref() == Some(&backup) {
            continue;
        }
        let bytes = std::fs::metadata(&backup)
            .map(|metadata| metadata.len())
            .unwrap_or_default();
        if std::fs::remove_file(&backup).is_err() {
            outcome.skipped_files += 1;
            continue;
        }
        outcome.removed_files += 1;
        outcome.removed_bytes = outcome.removed_bytes.saturating_add(bytes);
    }
    Ok(outcome)
}

fn prune_migration_backups_and_report(path: &Path, protected_backup: Option<&Path>) {
    match prune_migration_backups(path, protected_backup) {
        Ok(outcome) if outcome == MigrationBackupPruneOutcome::default() => {}
        Ok(outcome) => eprintln!(
            "bridge: migration backup retention removed_files={} removed_bytes={} skipped_files={}",
            outcome.removed_files, outcome.removed_bytes, outcome.skipped_files,
        ),
        Err(error) => eprintln!("bridge: migration backup retention failed: {error}"),
    }
}

/// The Work board's storage. Five tables, no changes to existing ones: a
/// database this migration has touched stays readable by the previous binary
/// apart from tables it never looks at.
///
/// `work_fact_cache` is the only one the offline board reads. The other four
/// exist so the briefing slices have somewhere to land without a second
/// migration, and so the constraints that keep a board idempotent
/// (`(run_id, evidence_ref)`, `(run_id, connector_instance_id)`, the task
/// fingerprint) are declared once, by the schema, rather than by whichever
/// writer remembers.
///
/// Nothing here holds a raw connector payload or a credential: provenance is
/// kept as digests and Bridge-derived identity, and the hidden briefing session
/// remains the diagnostic transcript.
fn migration_24_work_board(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    transaction.execute_batch(
        // `trigger_kind`, not `trigger`: TRIGGER is a SQLite keyword and a
        // column that needs quoting to be read is a column that will one day be
        // read unquoted.
        "CREATE TABLE IF NOT EXISTS work_brief_runs (
            id TEXT PRIMARY KEY,
            trigger_kind TEXT NOT NULL,
            status TEXT NOT NULL,
            profile_reference TEXT,
            session_id TEXT REFERENCES sessions(id) ON DELETE SET NULL,
            max_wall_seconds INTEGER NOT NULL,
            max_turns INTEGER NOT NULL,
            max_tool_calls INTEGER NOT NULL,
            max_output_tokens INTEGER,
            cost_ceiling_microusd INTEGER,
            output_digest TEXT,
            failure_code TEXT,
            failure_detail TEXT,
            input_tokens INTEGER NOT NULL DEFAULT 0,
            output_tokens INTEGER NOT NULL DEFAULT 0,
            cached_input_tokens INTEGER NOT NULL DEFAULT 0,
            cost_microusd INTEGER,
            tool_calls INTEGER NOT NULL DEFAULT 0,
            turns INTEGER NOT NULL DEFAULT 0,
            idempotency_key TEXT,
            started_at TEXT NOT NULL,
            completed_at TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_work_brief_runs_status ON work_brief_runs(status,started_at);
        -- Collapses a focus/cadence/manual race before a provider starts. Partial
        -- so runs that predate an idempotency key do not all collide on NULL.
        CREATE UNIQUE INDEX IF NOT EXISTS idx_work_brief_runs_idempotency
            ON work_brief_runs(idempotency_key) WHERE idempotency_key IS NOT NULL;
        CREATE TABLE IF NOT EXISTS work_brief_sources (
            run_id TEXT NOT NULL REFERENCES work_brief_runs(id) ON DELETE CASCADE,
            connector_instance_id TEXT NOT NULL,
            connector_family TEXT NOT NULL,
            status TEXT NOT NULL,
            detail TEXT,
            observed_at TEXT,
            UNIQUE(run_id,connector_instance_id)
        );
        CREATE INDEX IF NOT EXISTS idx_work_brief_sources_run ON work_brief_sources(run_id,status);
        CREATE TABLE IF NOT EXISTS work_evidence (
            run_id TEXT NOT NULL REFERENCES work_brief_runs(id) ON DELETE CASCADE,
            evidence_ref TEXT NOT NULL,
            tool_call_id TEXT NOT NULL,
            connector_instance_id TEXT NOT NULL,
            canonical_resource_id TEXT NOT NULL,
            source_kind TEXT NOT NULL,
            -- A serialized Bridge-derived target, never a model-authored URL.
            target TEXT,
            tool_definition_digest TEXT NOT NULL,
            result_digest TEXT NOT NULL,
            succeeded INTEGER NOT NULL DEFAULT 0,
            observed_at TEXT NOT NULL,
            UNIQUE(run_id,evidence_ref)
        );
        CREATE INDEX IF NOT EXISTS idx_work_evidence_resource
            ON work_evidence(connector_instance_id,canonical_resource_id);
        CREATE TABLE IF NOT EXISTS work_tasks (
            id TEXT PRIMARY KEY,
            -- NULL for an ephemeral task: one Bridge could not give a canonical
            -- identity. SQLite counts NULLs as distinct in a unique index, so
            -- several ephemeral tasks coexist while two identified tasks can
            -- never share a fingerprint.
            fingerprint TEXT,
            connector_instance_id TEXT NOT NULL,
            canonical_resource_id TEXT,
            source_kind TEXT NOT NULL,
            title TEXT NOT NULL,
            why TEXT NOT NULL,
            rank INTEGER NOT NULL,
            confidence_bps INTEGER NOT NULL,
            state TEXT NOT NULL DEFAULT 'active',
            pinned INTEGER NOT NULL DEFAULT 0,
            snoozed_until TEXT,
            evidence_digest TEXT,
            evidence_target TEXT,
            evidence_observed_at TEXT,
            -- Consecutive *successful* source-scoped misses. A connector failure
            -- never increments it, which is why it is stored rather than derived.
            miss_count INTEGER NOT NULL DEFAULT 0,
            ephemeral INTEGER NOT NULL DEFAULT 0,
            workspace_id TEXT REFERENCES workspaces(id) ON DELETE SET NULL,
            first_run_id TEXT REFERENCES work_brief_runs(id) ON DELETE SET NULL,
            last_run_id TEXT REFERENCES work_brief_runs(id) ON DELETE SET NULL,
            resolution TEXT,
            resolved_at TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            UNIQUE(fingerprint)
        );
        CREATE INDEX IF NOT EXISTS idx_work_tasks_board ON work_tasks(state,pinned,rank);
        CREATE INDEX IF NOT EXISTS idx_work_tasks_source
            ON work_tasks(connector_instance_id,canonical_resource_id);
        -- Snapshots of facts that cannot be observed on a store-only read path.
        -- `cache_key` is kind-defined (a workspace id for base divergence), so it
        -- carries no foreign key; every projection joins the entity it names, and
        -- a row whose entity is gone simply stops projecting.
        CREATE TABLE IF NOT EXISTS work_fact_cache (
            kind TEXT NOT NULL,
            cache_key TEXT NOT NULL,
            status TEXT NOT NULL,
            payload TEXT,
            detail TEXT,
            observed_at TEXT NOT NULL,
            PRIMARY KEY(kind,cache_key)
        );
        CREATE INDEX IF NOT EXISTS idx_work_fact_cache_observed ON work_fact_cache(kind,observed_at);",
    )?;
    Ok(())
}

/// Successful connector results without a stable provider id remain valid run-scoped
/// evidence. Their tasks are ephemeral and are never deduplicated across runs.
fn migration_25_ephemeral_work_evidence(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    transaction.execute_batch(
        "DROP INDEX IF EXISTS idx_work_evidence_resource;
         ALTER TABLE work_evidence RENAME TO work_evidence_v24;
         CREATE TABLE work_evidence (
            run_id TEXT NOT NULL REFERENCES work_brief_runs(id) ON DELETE CASCADE,
            evidence_ref TEXT NOT NULL,
            tool_call_id TEXT NOT NULL,
            connector_instance_id TEXT NOT NULL,
            canonical_resource_id TEXT,
            source_kind TEXT NOT NULL,
            target TEXT,
            tool_definition_digest TEXT NOT NULL,
            result_digest TEXT NOT NULL,
            succeeded INTEGER NOT NULL DEFAULT 0,
            observed_at TEXT NOT NULL,
            UNIQUE(run_id,evidence_ref)
         );
         INSERT INTO work_evidence(
            run_id,evidence_ref,tool_call_id,connector_instance_id,canonical_resource_id,
            source_kind,target,tool_definition_digest,result_digest,succeeded,observed_at)
         SELECT run_id,evidence_ref,tool_call_id,connector_instance_id,canonical_resource_id,
            source_kind,target,tool_definition_digest,result_digest,succeeded,observed_at
         FROM work_evidence_v24;
         DROP TABLE work_evidence_v24;
         CREATE INDEX idx_work_evidence_resource
            ON work_evidence(connector_instance_id,canonical_resource_id);",
    )?;
    Ok(())
}

/// The durable lease that makes racing briefing triggers safe: one active run,
/// heartbeated by its owner, reclaimable by compare-and-swap once the lease
/// expires, and a cancellation flag the run loop polls. Columns rather than a
/// new table because a lease without a run is meaningless — it is the run row
/// that is leased.
fn migration_26_briefing_run_leases(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    // Guarded per column: the repair path replays migrations over a database
    // whose tables may already carry them, and a blind ALTER would refuse the
    // whole replay over a column that is exactly what it should be.
    for (column, definition) in [
        ("lease_owner", "TEXT"),
        ("lease_expires_at", "TEXT"),
        ("cancellation_requested", "INTEGER NOT NULL DEFAULT 0"),
    ] {
        if !column_exists(transaction, "work_brief_runs", column)? {
            transaction.execute_batch(&format!(
                "ALTER TABLE work_brief_runs ADD COLUMN {column} {definition};"
            ))?;
        }
    }
    Ok(())
}

/// Scope learned routing policies to a workspace. Existing rows become
/// `legacy:global`, which live routing never selects. The unique live-policy
/// index stays "one active or canary", now per scope rather than globally —
/// the previous constant-expression unique index already made those two
/// statuses mutually exclusive, so the backfill cannot collide.
fn migration_29_learning_scope(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    add_column_if_missing(
        transaction,
        "routing_policies",
        "learning_scope",
        "TEXT NOT NULL DEFAULT 'legacy:global'",
    )?;
    add_column_if_missing(
        transaction,
        "learning_job_runs",
        "learning_scope",
        "TEXT NOT NULL DEFAULT 'legacy:global'",
    )?;
    add_column_if_missing(
        transaction,
        "routing_policy_promotions",
        "learning_scope",
        "TEXT NOT NULL DEFAULT 'legacy:global'",
    )?;
    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS learning_scope_cursors (
            learning_scope TEXT PRIMARY KEY,
            last_evidence_boundary INTEGER NOT NULL DEFAULT 0,
            updated_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_router_decisions_workspace_family
            ON router_decisions(workspace_id,task_family,created_at);
        DROP INDEX IF EXISTS idx_routing_policy_active;
        CREATE UNIQUE INDEX idx_routing_policy_active
            ON routing_policies(learning_scope) WHERE status IN ('active','canary');",
    )?;
    Ok(())
}

/// The durable home for user input submitted while a turn was already running.
///
/// A follow-up the user typed must not live only in a UI state hook: a reconnect
/// or a daemon restart would lose it, and an in-memory queue drained twice would
/// deliver it twice. `state` is the exactly-once guard — delivery claims a row
/// with a compare-and-swap out of `queued`, so two concurrent drains cannot both
/// win it.
fn migration_27_queued_session_input(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS queued_session_input (
            sequence INTEGER PRIMARY KEY AUTOINCREMENT,
            id TEXT NOT NULL UNIQUE,
            session_id TEXT NOT NULL,
            provider_text TEXT NOT NULL,
            display_text TEXT NOT NULL,
            state TEXT NOT NULL CHECK(state IN ('queued','claiming','delivered','abandoned')),
            created_at TEXT NOT NULL,
            delivered_at TEXT
        );
        CREATE INDEX IF NOT EXISTS queued_session_input_pending
            ON queued_session_input(session_id, state, sequence);",
    )?;
    Ok(())
}

/// Retry accounting, so a retry has to be earned rather than assumed.
///
/// `worker_retry_budget` is keyed by objective rather than by session: retrying
/// the same objective through a fresh worker is the same spend, and counting per
/// session let an identical task be paid for again under a new id.
/// `recovery_turns` records the three kinds of turn Bridge spends on its own
/// recovery separately, because "the agent used 40 turns" and "the agent used 12
/// turns and 28 corrections" are very different bills.
fn migration_32_worker_progress_summary(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    // One truthful line per live worker — "what it is doing right now",
    // derived from its own event stream — so Mission Control and the
    // orchestrator's fleet digest read progress without loading a feed.
    add_column_if_missing(transaction, "worker_runtime", "progress_summary", "TEXT")
}

fn migration_28_evidence_based_retries(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS worker_retry_budget (
            objective_key TEXT PRIMARY KEY,
            parent_session_id TEXT NOT NULL,
            attempts INTEGER NOT NULL DEFAULT 0,
            last_signal TEXT,
            updated_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS recovery_turns (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            session_id TEXT NOT NULL,
            kind TEXT NOT NULL,
            detail TEXT,
            created_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS recovery_turns_by_session
            ON recovery_turns(session_id, kind);",
    )?;
    Ok(())
}

fn migration_30_session_entry_fts(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    crate::session_recall::install_fts(transaction)
}

fn migration_31_memory_ledger(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    crate::memory_ledger::install_ledger(transaction)
}

fn migration_34_memory_lifecycle(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    crate::memory_ledger::install_lifecycle(transaction)
}

fn migration_35_memory_extraction(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    crate::memory_ledger::install_trust_fields(transaction)?;
    crate::memory_extraction::install(transaction)
}

fn migration_36_memory_packet(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    crate::memory_packet::install(transaction)
}

fn migration_41_routing_evaluation_runs(
    transaction: &Transaction<'_>,
) -> Result<(), BridgeError> {
    crate::routing_evaluation::install(transaction)
}

fn migration_42_memory_consolidation(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    crate::memory_ledger::install_validity_intervals(transaction)?;
    crate::memory_consolidation::install(transaction)
}

fn migration_45_latest_memory_packet_audit(
    transaction: &Transaction<'_>,
) -> Result<(), BridgeError> {
    crate::memory_packet::install_bounded_audit_retention(transaction)
}

/// Single-owner durable claims for provider permissions and questions.
///
/// The primary key is the immutable request entry. A claim commits before any
/// provider response, so another window/process/retry observes ownership and
/// cannot send a second answer. `status='settling'` deliberately survives a
/// crash: an uncertain external side effect is never retried automatically.
fn migration_43_interaction_resolutions(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS interaction_resolutions (
            session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
            request_sequence INTEGER NOT NULL,
            interaction_kind TEXT NOT NULL,
            status TEXT NOT NULL,
            decision TEXT NOT NULL,
            option_id TEXT,
            resolved_by TEXT NOT NULL,
            reason TEXT,
            result_event_sequence INTEGER,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            PRIMARY KEY(session_id, request_sequence)
        );
        CREATE INDEX IF NOT EXISTS interaction_resolutions_status
            ON interaction_resolutions(status, updated_at);",
    )?;
    Ok(())
}

fn migration_39_learning_tunables(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS learning_tunables (
            workspace_id TEXT PRIMARY KEY,
            body TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );",
    )?;
    Ok(())
}

fn migration_38_routing_catalogs(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS routing_catalogs (
            hash TEXT PRIMARY KEY,
            snapshot TEXT NOT NULL,
            created_at TEXT NOT NULL
        );",
    )?;
    add_column_if_missing(transaction, "router_decisions", "catalog_hash", "TEXT")?;
    Ok(())
}

fn migration_37_family_only_preferences(
    transaction: &Transaction<'_>,
) -> Result<(), BridgeError> {
    let rows: Vec<(i64, String)> = {
        let mut statement = transaction.prepare("SELECT version, weights FROM routing_policies")?;
        let mapped = statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        mapped
    };
    for (version, weights) in rows {
        let Ok(mut parsed) = serde_json::from_str::<serde_json::Value>(&weights) else {
            continue;
        };
        if crate::routing_policy::strip_fingerprint_preferences(&mut parsed) {
            transaction.execute(
                "UPDATE routing_policies SET weights=?2 WHERE version=?1",
                rusqlite::params![version, parsed.to_string()],
            )?;
        }
    }
    Ok(())
}

fn migration_1_current_schema(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_version (
            version INTEGER PRIMARY KEY,
            applied_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS projects (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            path TEXT NOT NULL UNIQUE,
            created_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS workspaces (
            id TEXT PRIMARY KEY,
            project_id TEXT NOT NULL REFERENCES projects(id),
            city TEXT NOT NULL,
            title TEXT NOT NULL,
            branch TEXT NOT NULL,
            path TEXT NOT NULL UNIQUE,
            status TEXT NOT NULL,
            dirty_files INTEGER NOT NULL DEFAULT 0,
            additions INTEGER NOT NULL DEFAULT 0,
            deletions INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS sessions (
            id TEXT PRIMARY KEY,
            workspace_id TEXT NOT NULL REFERENCES workspaces(id),
            harness TEXT NOT NULL,
            label TEXT NOT NULL,
            status TEXT NOT NULL,
            started_at TEXT,
            ended_at TEXT,
            context_percent INTEGER,
            usage_percent INTEGER,
            metric_source TEXT NOT NULL DEFAULT 'estimated'
        );
        CREATE TABLE IF NOT EXISTS events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            source TEXT NOT NULL,
            kind TEXT NOT NULL,
            entity_id TEXT NOT NULL,
            body TEXT NOT NULL,
            created_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS agent_events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            session_id TEXT NOT NULL REFERENCES sessions(id),
            sequence INTEGER NOT NULL,
            protocol_version INTEGER NOT NULL DEFAULT 1,
            kind TEXT NOT NULL,
            item_id TEXT,
            role TEXT,
            status TEXT,
            title TEXT,
            text TEXT,
            data TEXT NOT NULL DEFAULT '{}',
            provider_meta TEXT NOT NULL DEFAULT '{}',
            created_at TEXT NOT NULL,
            UNIQUE(session_id, sequence)
        );
        CREATE INDEX IF NOT EXISTS idx_agent_events_session ON agent_events(session_id, sequence);",
    )?;
    add_column_if_missing(transaction, "sessions", "provider_session_id", "TEXT")?;
    add_column_if_missing(transaction, "sessions", "active_turn_id", "TEXT")?;
    add_column_if_missing(transaction, "sessions", "model", "TEXT")?;
    add_column_if_missing(transaction, "sessions", "effort", "TEXT")?;
    add_column_if_missing(transaction, "sessions", "parent_session_id", "TEXT")?;
    add_column_if_missing(transaction, "sessions", "depth", "INTEGER")?;
    Ok(())
}

pub(crate) fn add_column_if_missing(
    transaction: &Transaction<'_>,
    table: &str,
    column: &str,
    definition: &str,
) -> Result<(), BridgeError> {
    if !table_exists(transaction, table)? {
        return Ok(());
    }
    if !column_exists(transaction, table, column)? {
        transaction.execute_batch(&format!(
            "ALTER TABLE {table} ADD COLUMN {column} {definition}"
        ))?;
    }
    Ok(())
}

fn table_exists(transaction: &Transaction<'_>, table: &str) -> Result<bool, BridgeError> {
    Ok(transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
        params![table],
        |row| row.get(0),
    )?)
}

fn column_exists(
    transaction: &Transaction<'_>,
    table: &str,
    column: &str,
) -> Result<bool, BridgeError> {
    let mut statement = transaction.prepare(&format!("PRAGMA table_info({table})"))?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(columns.iter().any(|existing| existing == column))
}

fn migration_2_session_forest(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    transaction.execute_batch(
        "CREATE TABLE session_entries (
            id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL REFERENCES sessions(id),
            parent_entry_id TEXT REFERENCES session_entries(id),
            sequence INTEGER NOT NULL,
            kind TEXT NOT NULL,
            payload TEXT NOT NULL DEFAULT '{}',
            provider_event_id TEXT,
            context_visibility TEXT NOT NULL DEFAULT 'eligible',
            token_estimate INTEGER,
            created_at TEXT NOT NULL,
            UNIQUE(session_id, sequence)
        );
        CREATE INDEX idx_session_entries_parent
            ON session_entries(session_id, parent_entry_id);
        CREATE TABLE session_heads (
            session_id TEXT PRIMARY KEY REFERENCES sessions(id),
            active_entry_id TEXT REFERENCES session_entries(id),
            native_provider_session_id TEXT,
            restoration_mode TEXT NOT NULL DEFAULT 'fresh',
            latest_checkpoint_entry_id TEXT REFERENCES session_entries(id),
            updated_at TEXT NOT NULL
        );
        CREATE TABLE task_knowledge (
            id TEXT PRIMARY KEY,
            workspace_id TEXT NOT NULL REFERENCES workspaces(id),
            session_id TEXT REFERENCES sessions(id),
            kind TEXT NOT NULL,
            body TEXT NOT NULL,
            source_entry_id TEXT REFERENCES session_entries(id),
            superseded_by TEXT REFERENCES task_knowledge(id),
            created_at TEXT NOT NULL
        );
        CREATE TABLE worker_leases (
            session_id TEXT PRIMARY KEY REFERENCES sessions(id),
            workspace_id TEXT NOT NULL REFERENCES workspaces(id),
            role TEXT NOT NULL,
            capability_tier TEXT NOT NULL,
            owned_paths TEXT NOT NULL DEFAULT '[]',
            write_mode TEXT NOT NULL,
            lease_status TEXT NOT NULL,
            expires_at TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );
        CREATE INDEX idx_worker_leases_workspace
            ON worker_leases(workspace_id, lease_status);
        CREATE TABLE usage_ledger (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            workspace_id TEXT NOT NULL,
            session_id TEXT,
            turn_id TEXT,
            input_tokens INTEGER,
            output_tokens INTEGER,
            cache_read_tokens INTEGER,
            cache_write_tokens INTEGER,
            context_percent INTEGER,
            capability_units INTEGER NOT NULL DEFAULT 0,
            runtime_ms INTEGER,
            source TEXT NOT NULL,
            created_at TEXT NOT NULL
        );
        CREATE INDEX idx_usage_ledger_scope
            ON usage_ledger(workspace_id, session_id, created_at);",
    )?;
    backfill_agent_events(transaction)?;
    Ok(())
}

fn migration_3_capability_tiers(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    add_column_if_missing(transaction, "sessions", "requested_tier", "TEXT")
}

fn migration_4_resume_eligibility(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    add_column_if_missing(
        transaction,
        "session_heads",
        "resume_eligibility",
        "TEXT NOT NULL DEFAULT 'fresh'",
    )
}

fn migration_5_durable_worker_pool(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    add_column_if_missing(
        transaction,
        "worker_leases",
        "task_family",
        "TEXT NOT NULL DEFAULT ''",
    )?;
    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS worker_runtime (
            session_id TEXT PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE,
            parent_session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
            lifecycle_state TEXT NOT NULL,
            task_family TEXT NOT NULL,
            compatibility_key TEXT NOT NULL,
            result_status TEXT NOT NULL DEFAULT 'pending',
            retry_count INTEGER NOT NULL DEFAULT 0,
            warm_until TEXT,
            worktree_path TEXT,
            worktree_branch TEXT,
            last_result TEXT,
            updated_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_worker_runtime_parent
            ON worker_runtime(parent_session_id, result_status, lifecycle_state);
        CREATE INDEX IF NOT EXISTS idx_worker_runtime_compatibility
            ON worker_runtime(compatibility_key, lifecycle_state);
        CREATE TABLE IF NOT EXISTS delegation_receipts (
            dedupe_key TEXT PRIMARY KEY,
            session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
            item_id TEXT NOT NULL,
            created_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS worker_queue (
            sequence INTEGER PRIMARY KEY AUTOINCREMENT,
            id TEXT NOT NULL UNIQUE,
            parent_session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
            workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
            turn_id TEXT NOT NULL,
            request TEXT NOT NULL,
            actual_model TEXT NOT NULL,
            queue_status TEXT NOT NULL DEFAULT 'queued',
            dispatched_session_id TEXT REFERENCES sessions(id) ON DELETE SET NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_worker_queue_dispatch
            ON worker_queue(workspace_id, queue_status, sequence);",
    )?;
    Ok(())
}

fn migration_6_remove_legacy_agent_events(
    transaction: &Transaction<'_>,
) -> Result<(), BridgeError> {
    transaction.execute_batch(
        "DROP INDEX IF EXISTS idx_agent_events_session;
         DROP TABLE IF EXISTS agent_events;",
    )?;
    Ok(())
}

/// Make git optional and support direct chats:
/// - workspaces: project_id, city, branch, path become nullable (repo-less workspaces)
/// - sessions: workspace_id becomes nullable (standalone chats); add title, kind, cwd
/// Runs with foreign keys disabled (see `open`), so the parent-table rebuilds are safe.
fn migration_7_optional_repo_and_direct_chats(
    transaction: &Transaction<'_>,
) -> Result<(), BridgeError> {
    transaction.execute_batch(
        "CREATE TABLE workspaces_new (
            id TEXT PRIMARY KEY,
            project_id TEXT REFERENCES projects(id),
            city TEXT,
            title TEXT NOT NULL,
            branch TEXT,
            path TEXT UNIQUE,
            status TEXT NOT NULL,
            dirty_files INTEGER NOT NULL DEFAULT 0,
            additions INTEGER NOT NULL DEFAULT 0,
            deletions INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL
        );
        INSERT INTO workspaces_new (id,project_id,city,title,branch,path,status,dirty_files,additions,deletions,created_at)
            SELECT id,project_id,city,title,branch,path,status,dirty_files,additions,deletions,created_at FROM workspaces;
        DROP TABLE workspaces;
        ALTER TABLE workspaces_new RENAME TO workspaces;

        CREATE TABLE sessions_new (
            id TEXT PRIMARY KEY,
            workspace_id TEXT REFERENCES workspaces(id),
            harness TEXT NOT NULL,
            label TEXT NOT NULL,
            status TEXT NOT NULL,
            started_at TEXT,
            ended_at TEXT,
            context_percent INTEGER,
            usage_percent INTEGER,
            metric_source TEXT NOT NULL DEFAULT 'estimated',
            provider_session_id TEXT,
            active_turn_id TEXT,
            model TEXT,
            effort TEXT,
            parent_session_id TEXT,
            depth INTEGER,
            requested_tier TEXT,
            title TEXT,
            kind TEXT NOT NULL DEFAULT 'orchestrator',
            cwd TEXT
        );
        INSERT INTO sessions_new (id,workspace_id,harness,label,status,started_at,ended_at,context_percent,usage_percent,metric_source,provider_session_id,active_turn_id,model,effort,parent_session_id,depth,requested_tier)
            SELECT id,workspace_id,harness,label,status,started_at,ended_at,context_percent,usage_percent,metric_source,provider_session_id,active_turn_id,model,effort,parent_session_id,depth,requested_tier FROM sessions;
        DROP TABLE sessions;
        ALTER TABLE sessions_new RENAME TO sessions;",
    )?;
    Ok(())
}

fn migration_8_reliability_primitives(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    add_column_if_missing(
        transaction,
        "worker_queue",
        "attempt_count",
        "INTEGER NOT NULL DEFAULT 0",
    )?;
    add_column_if_missing(transaction, "worker_queue", "expires_at", "TEXT")?;
    add_column_if_missing(transaction, "worker_queue", "claimed_at", "TEXT")?;
    add_column_if_missing(transaction, "worker_queue", "last_error", "TEXT")?;
    add_column_if_missing(transaction, "sessions", "trace_id", "TEXT")?;
    add_column_if_missing(transaction, "usage_ledger", "trace_id", "TEXT")?;
    transaction.execute_batch("UPDATE worker_queue SET expires_at=COALESCE(expires_at,datetime(created_at, '+24 hours'));
        CREATE INDEX IF NOT EXISTS idx_worker_queue_lifecycle ON worker_queue(queue_status,expires_at,claimed_at,sequence);
        CREATE TABLE IF NOT EXISTS durable_outbox (id TEXT PRIMARY KEY,destination TEXT NOT NULL,event_type TEXT NOT NULL,payload TEXT NOT NULL,idempotency_key TEXT NOT NULL UNIQUE,status TEXT NOT NULL DEFAULT 'pending',attempt_count INTEGER NOT NULL DEFAULT 0,next_attempt_at TEXT NOT NULL,last_error TEXT,created_at TEXT NOT NULL,delivered_at TEXT);
        CREATE INDEX IF NOT EXISTS idx_durable_outbox_delivery ON durable_outbox(status,next_attempt_at);
        CREATE TABLE IF NOT EXISTS integration_inbox (source TEXT NOT NULL,idempotency_key TEXT NOT NULL,received_at TEXT NOT NULL,PRIMARY KEY(source,idempotency_key));
        CREATE TABLE IF NOT EXISTS handoff_packets (id TEXT PRIMARY KEY,schema_version INTEGER NOT NULL,trace_id TEXT NOT NULL,source_harness TEXT NOT NULL,target_harness TEXT NOT NULL,payload TEXT NOT NULL,created_at TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS telemetry_spans (span_id TEXT PRIMARY KEY,trace_id TEXT NOT NULL,parent_span_id TEXT,name TEXT NOT NULL,attributes TEXT NOT NULL,started_at TEXT NOT NULL,ended_at TEXT);")?;
    Ok(())
}

fn migration_9_semantic_event_version(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    add_column_if_missing(
        transaction,
        "session_entries",
        "semantic_schema_version",
        "INTEGER NOT NULL DEFAULT 1",
    )
}

fn migration_10_continuation_fidelity(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    add_column_if_missing(
        transaction,
        "sessions",
        "continuation_fidelity",
        "TEXT NOT NULL DEFAULT 'native'",
    )?;
    transaction.execute_batch(
        "UPDATE sessions SET continuation_fidelity=CASE
            WHEN parent_session_id IS NULL THEN 'native'
            WHEN id IN (SELECT session_id FROM session_heads WHERE restoration_mode='checkpoint_restored') THEN 'projected_at_boundary'
            WHEN id IN (SELECT session_id FROM session_heads WHERE restoration_mode IN ('native','hot')) THEN 'native'
            ELSE 'projected_mid_turn'
         END;",
    )?;
    Ok(())
}

fn migration_11_human_blocked_queue(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    add_column_if_missing(transaction, "worker_queue", "blocked_at", "TEXT")
}

fn migration_12_adapter_process_claims(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    add_column_if_missing(transaction, "sessions", "adapter_pid", "INTEGER")?;
    add_column_if_missing(transaction, "sessions", "adapter_process_identity", "TEXT")
}

fn migration_13_learning_router(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS router_preferences (
            workspace_id TEXT PRIMARY KEY REFERENCES workspaces(id) ON DELETE CASCADE,
            mode TEXT NOT NULL,
            preferences TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS router_decisions (
            id TEXT PRIMARY KEY,
            workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
            parent_session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
            turn_id TEXT NOT NULL,
            task_family TEXT NOT NULL,
            mode TEXT NOT NULL,
            manual_override INTEGER NOT NULL,
            baseline_candidate TEXT,
            recommended_candidate TEXT,
            executed_candidate TEXT,
            decision TEXT NOT NULL,
            policy_outcome TEXT,
            route_status TEXT,
            created_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_router_decisions_workspace
            ON router_decisions(workspace_id,created_at);
        CREATE INDEX IF NOT EXISTS idx_router_decisions_turn
            ON router_decisions(parent_session_id,turn_id);
        CREATE TABLE IF NOT EXISTS router_assignments (
            decision_id TEXT PRIMARY KEY REFERENCES router_decisions(id) ON DELETE CASCADE,
            child_session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
            status TEXT NOT NULL,
            created_at TEXT NOT NULL,
            completed_at TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_router_assignments_child
            ON router_assignments(child_session_id,status,created_at);
        CREATE TABLE IF NOT EXISTS router_outcomes (
            decision_id TEXT PRIMARY KEY REFERENCES router_decisions(id) ON DELETE CASCADE,
            child_session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
            candidate TEXT NOT NULL,
            succeeded INTEGER NOT NULL,
            status TEXT NOT NULL,
            runtime_ms INTEGER NOT NULL,
            normalized_cost INTEGER NOT NULL,
            retry_count INTEGER NOT NULL,
            human_intervention INTEGER NOT NULL DEFAULT 0,
            recorded_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_router_outcomes_candidate
            ON router_outcomes(candidate,recorded_at);",
    )?;
    Ok(())
}

fn migration_14_completion_proof(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS completion_contracts (
            id TEXT PRIMARY KEY,
            workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
            session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
            schema_version INTEGER NOT NULL,
            acceptance_criteria TEXT NOT NULL,
            markdown_projection TEXT,
            markdown_committed INTEGER NOT NULL DEFAULT 0,
            status TEXT NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_completion_contracts_session
            ON completion_contracts(session_id,status,created_at);
        CREATE TABLE IF NOT EXISTS eval_plans (
            id TEXT PRIMARY KEY,
            contract_id TEXT NOT NULL REFERENCES completion_contracts(id) ON DELETE CASCADE,
            schema_version INTEGER NOT NULL,
            risk TEXT NOT NULL,
            plan TEXT NOT NULL,
            created_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS eval_attempts (
            id TEXT PRIMARY KEY,
            plan_id TEXT NOT NULL REFERENCES eval_plans(id) ON DELETE CASCADE,
            session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
            repository_head TEXT NOT NULL,
            dirty_digest TEXT NOT NULL,
            repository_path TEXT NOT NULL,
            status TEXT NOT NULL,
            implementer_family TEXT,
            started_at TEXT NOT NULL,
            completed_at TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_eval_attempts_session
            ON eval_attempts(session_id,status,started_at);
        CREATE TABLE IF NOT EXISTS eval_check_runs (
            id TEXT PRIMARY KEY,
            attempt_id TEXT NOT NULL REFERENCES eval_attempts(id) ON DELETE CASCADE,
            check_id TEXT NOT NULL,
            kind TEXT NOT NULL,
            required INTEGER NOT NULL,
            status TEXT NOT NULL,
            executor TEXT NOT NULL,
            command TEXT,
            verifier_family TEXT,
            detail TEXT,
            output_digest TEXT,
            artifact_refs TEXT NOT NULL DEFAULT '[]',
            started_at TEXT,
            completed_at TEXT,
            UNIQUE(attempt_id,check_id)
        );
        CREATE INDEX IF NOT EXISTS idx_eval_check_runs_attempt
            ON eval_check_runs(attempt_id,status,required);
        CREATE TABLE IF NOT EXISTS eval_findings (
            id TEXT PRIMARY KEY,
            attempt_id TEXT NOT NULL REFERENCES eval_attempts(id) ON DELETE CASCADE,
            check_id TEXT NOT NULL,
            severity TEXT NOT NULL,
            summary TEXT NOT NULL,
            affected_paths TEXT NOT NULL DEFAULT '[]',
            resolved_at TEXT,
            created_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS eval_waivers (
            id TEXT PRIMARY KEY,
            attempt_id TEXT NOT NULL REFERENCES eval_attempts(id) ON DELETE CASCADE,
            check_ids TEXT NOT NULL,
            reason TEXT NOT NULL,
            granted_by TEXT NOT NULL,
            repository_head TEXT NOT NULL,
            dirty_digest TEXT NOT NULL,
            created_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS proof_bundles (
            id TEXT PRIMARY KEY,
            attempt_id TEXT NOT NULL UNIQUE REFERENCES eval_attempts(id) ON DELETE CASCADE,
            schema_version INTEGER NOT NULL,
            verdict TEXT NOT NULL,
            bundle TEXT NOT NULL,
            bundle_digest TEXT NOT NULL,
            created_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS verifier_manifests (
            id TEXT PRIMARY KEY,
            source TEXT NOT NULL,
            schema_version INTEGER NOT NULL,
            manifest TEXT NOT NULL,
            enabled INTEGER NOT NULL DEFAULT 1,
            updated_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS worker_completion_inputs (
            child_session_id TEXT PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE,
            request TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );",
    )?;
    Ok(())
}

fn migration_15_role_profiles_and_learning_jobs(
    transaction: &Transaction<'_>,
) -> Result<(), BridgeError> {
    add_column_if_missing(transaction, "usage_ledger", "cost_microusd", "INTEGER")?;
    add_column_if_missing(transaction, "usage_ledger", "cost_source", "TEXT")?;
    add_column_if_missing(
        transaction,
        "router_decisions",
        "task_fingerprint",
        "TEXT NOT NULL DEFAULT 'legacy'",
    )?;
    add_column_if_missing(transaction, "router_decisions", "trace_id", "TEXT")?;
    add_column_if_missing(
        transaction,
        "router_decisions",
        "repository_revision",
        "TEXT",
    )?;
    add_column_if_missing(
        transaction,
        "router_decisions",
        "profile_version",
        "INTEGER",
    )?;
    add_column_if_missing(transaction, "router_decisions", "profile_purpose", "TEXT")?;
    add_column_if_missing(
        transaction,
        "router_decisions",
        "policy_version",
        "INTEGER NOT NULL DEFAULT 1",
    )?;
    add_column_if_missing(
        transaction,
        "router_decisions",
        "catalog_snapshot",
        "TEXT NOT NULL DEFAULT '{}'",
    )?;
    add_column_if_missing(transaction, "router_decisions", "selection_reason", "TEXT")?;
    add_column_if_missing(transaction, "router_decisions", "actual_provider", "TEXT")?;
    add_column_if_missing(transaction, "router_decisions", "actual_model", "TEXT")?;
    add_column_if_missing(transaction, "router_decisions", "actual_effort", "TEXT")?;
    add_column_if_missing(
        transaction,
        "router_outcomes",
        "success_state",
        "TEXT NOT NULL DEFAULT 'unknown'",
    )?;
    add_column_if_missing(
        transaction,
        "router_outcomes",
        "acceptance_state",
        "TEXT NOT NULL DEFAULT 'unknown'",
    )?;
    add_column_if_missing(transaction, "router_outcomes", "cost_microusd", "INTEGER")?;
    add_column_if_missing(transaction, "router_outcomes", "cost_source", "TEXT")?;
    add_column_if_missing(transaction, "router_outcomes", "confidence_bps", "INTEGER")?;
    add_column_if_missing(
        transaction,
        "router_outcomes",
        "edit_count",
        "INTEGER NOT NULL DEFAULT 0",
    )?;
    add_column_if_missing(
        transaction,
        "router_outcomes",
        "override_signal",
        "INTEGER NOT NULL DEFAULT 0",
    )?;
    add_column_if_missing(transaction, "router_outcomes", "total_tokens", "INTEGER")?;
    add_column_if_missing(transaction, "router_outcomes", "latency_source", "TEXT")?;
    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS model_profiles (
            version INTEGER NOT NULL,
            profile_id TEXT NOT NULL DEFAULT 'legacy',
            purpose TEXT NOT NULL,
            canonical_role TEXT NOT NULL,
            provider TEXT NOT NULL,
            model TEXT NOT NULL,
            effort TEXT NOT NULL,
            fallback_purpose TEXT,
            pinned INTEGER NOT NULL DEFAULT 0,
            learning_enabled INTEGER NOT NULL DEFAULT 1,
            budget_preference TEXT,
            latency_preference TEXT,
            created_at TEXT NOT NULL,
            PRIMARY KEY(version,purpose)
        );
        CREATE INDEX IF NOT EXISTS idx_model_profiles_purpose
            ON model_profiles(purpose,version);
        CREATE TABLE IF NOT EXISTS model_setup_state (
            id TEXT PRIMARY KEY,
            active_version INTEGER NOT NULL,
            updated_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS routing_policies (
            version INTEGER PRIMARY KEY,
            status TEXT NOT NULL,
            predecessor INTEGER REFERENCES routing_policies(version),
            rollback_of INTEGER REFERENCES routing_policies(version),
            weights TEXT NOT NULL,
            thresholds TEXT NOT NULL,
            replay_report TEXT,
            created_reason TEXT NOT NULL,
            created_at TEXT NOT NULL,
            promoted_at TEXT,
            activation_boundary INTEGER
        );
        CREATE UNIQUE INDEX IF NOT EXISTS idx_routing_policy_active
            ON routing_policies((1)) WHERE status IN ('active','canary');
        CREATE TABLE IF NOT EXISTS routing_evaluations (
            id TEXT PRIMARY KEY,
            learning_run_id TEXT REFERENCES learning_job_runs(id) ON DELETE SET NULL,
            decision_id TEXT REFERENCES router_decisions(id) ON DELETE CASCADE,
            evaluator_kind TEXT NOT NULL,
            evaluator_version TEXT NOT NULL,
            score_bps INTEGER,
            confidence_bps INTEGER,
            evidence_entry_ids TEXT NOT NULL DEFAULT '[]',
            bounded_metrics TEXT NOT NULL DEFAULT '{}',
            status TEXT NOT NULL DEFAULT 'completed',
            created_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS learning_jobs (
            id TEXT PRIMARY KEY,
            enabled INTEGER NOT NULL DEFAULT 0,
            cadence_minutes INTEGER NOT NULL DEFAULT 1440,
            next_run_at TEXT,
            last_evidence_boundary INTEGER NOT NULL DEFAULT 0,
            run_budget_microusd INTEGER NOT NULL DEFAULT 100000,
            run_budget_tokens INTEGER NOT NULL DEFAULT 50000,
            mode TEXT NOT NULL DEFAULT 'manual',
            updated_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS learning_triggers (
            id TEXT PRIMARY KEY,
            job_id TEXT NOT NULL REFERENCES learning_jobs(id) ON DELETE CASCADE,
            kind TEXT NOT NULL,
            registration_id TEXT NOT NULL,
            credential_ref TEXT,
            auth_digest TEXT,
            enabled INTEGER NOT NULL DEFAULT 1,
            expires_at TEXT,
            experimental INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            UNIQUE(kind,registration_id)
        );
        CREATE TABLE IF NOT EXISTS learning_job_runs (
            id TEXT PRIMARY KEY,
            job_id TEXT NOT NULL REFERENCES learning_jobs(id) ON DELETE CASCADE,
            trigger_kind TEXT NOT NULL,
            idempotency_key TEXT NOT NULL UNIQUE,
            evidence_boundary INTEGER NOT NULL,
            base_policy_version INTEGER NOT NULL,
            status TEXT NOT NULL,
            lease_owner TEXT,
            lease_expires_at TEXT,
            snapshot_frozen_at TEXT NOT NULL,
            report TEXT,
            candidate_policy_version INTEGER REFERENCES routing_policies(version),
            cancellation_requested INTEGER NOT NULL DEFAULT 0,
            evaluated_spend_microusd INTEGER NOT NULL DEFAULT 0,
            evaluated_tokens INTEGER NOT NULL DEFAULT 0,
            replay_passed INTEGER,
            promotion_status TEXT NOT NULL DEFAULT 'not_requested',
            created_at TEXT NOT NULL,
            completed_at TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_learning_job_runs_status
            ON learning_job_runs(job_id,status,created_at);
        CREATE UNIQUE INDEX IF NOT EXISTS idx_learning_job_active_lease
            ON learning_job_runs(job_id) WHERE status IN ('queued','running');
        CREATE TABLE IF NOT EXISTS learning_trigger_events (
            id TEXT PRIMARY KEY,
            run_id TEXT REFERENCES learning_job_runs(id) ON DELETE SET NULL,
            trigger_kind TEXT NOT NULL,
            registration_id TEXT,
            result TEXT NOT NULL,
            reason TEXT,
            created_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS routing_policy_promotions (
            id TEXT PRIMARY KEY,
            from_version INTEGER NOT NULL REFERENCES routing_policies(version),
            to_version INTEGER NOT NULL REFERENCES routing_policies(version),
            learning_run_id TEXT REFERENCES learning_job_runs(id) ON DELETE SET NULL,
            action TEXT NOT NULL,
            actor TEXT NOT NULL,
            explanation TEXT NOT NULL,
            replay_report TEXT,
            created_at TEXT NOT NULL
        );
        INSERT OR IGNORE INTO routing_policies(version,status,weights,thresholds,created_reason,created_at)
            VALUES(1,'active','{}','{}','initial deterministic routing policy',CURRENT_TIMESTAMP);
        INSERT OR IGNORE INTO learning_jobs(id,enabled,cadence_minutes,run_budget_microusd,run_budget_tokens,mode,updated_at)
            VALUES('default',0,1440,100000,50000,'manual',CURRENT_TIMESTAMP);
        INSERT OR IGNORE INTO learning_triggers(id,job_id,kind,registration_id,enabled,experimental,created_at,updated_at)
            VALUES('builtin-manual','default','manual','built-in',1,0,CURRENT_TIMESTAMP,CURRENT_TIMESTAMP);
        INSERT OR IGNORE INTO learning_triggers(id,job_id,kind,registration_id,enabled,experimental,created_at,updated_at)
            VALUES('builtin-in-app','default','in_app','built-in',1,0,CURRENT_TIMESTAMP,CURRENT_TIMESTAMP);",
    )?;
    add_column_if_missing(
        transaction,
        "model_profiles",
        "profile_id",
        "TEXT NOT NULL DEFAULT 'legacy'",
    )?;
    add_column_if_missing(
        transaction,
        "learning_jobs",
        "last_evidence_boundary",
        "INTEGER NOT NULL DEFAULT 0",
    )?;
    transaction.execute(
        "CREATE INDEX IF NOT EXISTS idx_model_profiles_id ON model_profiles(profile_id,version)",
        [],
    )?;
    Ok(())
}

fn migration_16_complete_role_profile_schema(
    transaction: &Transaction<'_>,
) -> Result<(), BridgeError> {
    // These columns were added to migration 15 after some databases had
    // already recorded version 15. A new migration is required to repair
    // those databases because completed migrations are never replayed.
    add_column_if_missing(
        transaction,
        "model_profiles",
        "profile_id",
        "TEXT NOT NULL DEFAULT 'legacy'",
    )?;
    add_column_if_missing(
        transaction,
        "learning_jobs",
        "last_evidence_boundary",
        "INTEGER NOT NULL DEFAULT 0",
    )?;
    transaction.execute(
        "CREATE INDEX IF NOT EXISTS idx_model_profiles_id ON model_profiles(profile_id,version)",
        [],
    )?;
    Ok(())
}

#[derive(Debug)]
struct LegacyAgentEvent {
    id: i64,
    session_id: String,
    sequence: i64,
    protocol_version: i64,
    kind: String,
    item_id: Option<String>,
    role: Option<String>,
    status: Option<String>,
    title: Option<String>,
    text: Option<String>,
    data: String,
    provider_meta: String,
    created_at: String,
}

fn backfill_agent_events(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    let legacy_events = {
        let mut statement = transaction.prepare(
            "SELECT id,session_id,sequence,protocol_version,kind,item_id,role,status,title,text,data,provider_meta,created_at
             FROM agent_events ORDER BY session_id,sequence,id",
        )?;
        let events = statement
            .query_map([], |row| {
                Ok(LegacyAgentEvent {
                    id: row.get(0)?,
                    session_id: row.get(1)?,
                    sequence: row.get(2)?,
                    protocol_version: row.get(3)?,
                    kind: row.get(4)?,
                    item_id: row.get(5)?,
                    role: row.get(6)?,
                    status: row.get(7)?,
                    title: row.get(8)?,
                    text: row.get(9)?,
                    data: row.get(10)?,
                    provider_meta: row.get(11)?,
                    created_at: row.get(12)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        events
    };
    let mut previous_by_session: HashMap<String, String> = HashMap::new();
    for event in legacy_events {
        let entry_id = format!("agent-event-{}", event.id);
        let parent_entry_id = previous_by_session.get(&event.session_id).cloned();
        let data = serde_json::from_str(&event.data).unwrap_or(serde_json::Value::Null);
        let provider_meta =
            serde_json::from_str(&event.provider_meta).unwrap_or(serde_json::Value::Null);
        let payload = serde_json::json!({
            "protocolVersion": event.protocol_version,
            "itemId": event.item_id,
            "role": event.role,
            "status": event.status,
            "title": event.title,
            "text": event.text,
            "data": data,
            "providerMeta": provider_meta,
        });
        transaction.execute(
            "INSERT INTO session_entries(id,session_id,parent_entry_id,sequence,kind,payload,provider_event_id,created_at)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
            params![
                entry_id,
                event.session_id,
                parent_entry_id,
                event.sequence,
                event.kind,
                payload.to_string(),
                event.item_id,
                event.created_at
            ],
        )?;
        previous_by_session.insert(event.session_id, entry_id);
    }
    let now = Utc::now().to_rfc3339();
    transaction.execute(
        "INSERT INTO session_heads(session_id,active_entry_id,native_provider_session_id,restoration_mode,updated_at)
         SELECT sessions.id,
                (SELECT id FROM session_entries WHERE session_id=sessions.id ORDER BY sequence DESC LIMIT 1),
                sessions.provider_session_id,
                'fresh',
                ?1
         FROM sessions",
        params![now],
    )?;
    Ok(())
}

pub fn state(db: &Connection) -> Result<BridgeState, BridgeError> {
    let projects = query(
        db,
        "SELECT id,name,path,created_at FROM projects ORDER BY created_at",
        |r| {
            Ok(Project {
                id: r.get(0)?,
                name: r.get(1)?,
                path: r.get(2)?,
                created_at: r.get(3)?,
            })
        },
    )?;
    let workspaces = query(db, "SELECT id,project_id,city,title,branch,path,status,dirty_files,additions,deletions,created_at FROM workspaces ORDER BY created_at", |r| Ok(Workspace { id:r.get(0)?, project_id:r.get(1)?, city:r.get(2)?, title:r.get(3)?, branch:r.get(4)?, path:r.get(5)?, status:status(&r.get::<_,String>(6)?), dirty_files:r.get(7)?, additions:r.get(8)?, deletions:r.get(9)?, created_at:r.get(10)? }))?;
    let sessions = query(db, "SELECT s.id,s.workspace_id,s.harness,s.label,s.status,s.started_at,s.ended_at,s.context_percent,s.usage_percent,s.metric_source,s.provider_session_id,s.active_turn_id,s.model,s.requested_tier,s.effort,s.parent_session_id,s.depth,COALESCE(h.restoration_mode,'fresh'),s.continuation_fidelity,s.title,s.kind,s.cwd FROM sessions s LEFT JOIN session_heads h ON h.session_id=s.id
         WHERE s.archived_at IS NULL
           AND (s.parent_session_id IS NULL
                OR NOT EXISTS(SELECT 1 FROM sessions p
                               WHERE p.id=s.parent_session_id AND p.archived_at IS NOT NULL))
         ORDER BY s.rowid", |r| Ok(Session { id:r.get(0)?, workspace_id:r.get(1)?, harness:harness(&r.get::<_,String>(2)?), label:r.get(3)?, status:status(&r.get::<_,String>(4)?), started_at:r.get(5)?, ended_at:r.get(6)?, context_percent:r.get(7)?, usage_percent:r.get(8)?, metric_source:r.get(9)?, provider_session_id:r.get(10)?, active_turn_id:r.get(11)?, model:r.get(12)?, requested_tier:capability_tier(r.get::<_,Option<String>>(13)?), effort:r.get(14)?, parent_session_id:r.get(15)?, depth:r.get(16)?, restoration_mode:restoration_mode(&r.get::<_,String>(17)?), continuation_fidelity:continuation_fidelity(&r.get::<_,String>(18)?), title:r.get(19)?, kind:r.get(20)?, cwd:r.get(21)? }))?;
    let events = query(
        db,
        "SELECT id,source,kind,entity_id,body,created_at FROM events ORDER BY id DESC LIMIT 200",
        |r| {
            Ok(BridgeEvent {
                id: r.get(0)?,
                source: r.get(1)?,
                kind: r.get(2)?,
                entity_id: r.get(3)?,
                body: r.get(4)?,
                created_at: r.get(5)?,
            })
        },
    )?;
    Ok(BridgeState {
        projects,
        workspaces,
        sessions,
        events,
    })
}

fn query<T, F>(db: &Connection, sql: &str, mut map: F) -> Result<Vec<T>, BridgeError>
where
    F: FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
{
    let mut stmt = db.prepare(sql)?;
    let rows = stmt.query_map([], |row| map(row))?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}
pub fn event(
    db: &Connection,
    source: &str,
    kind: &str,
    entity: &str,
    body: &str,
) -> Result<(), BridgeError> {
    db.execute(
        "INSERT INTO events(source,kind,entity_id,body,created_at) VALUES(?1,?2,?3,?4,?5)",
        params![source, kind, entity, body, Utc::now().to_rfc3339()],
    )?;
    Ok(())
}
pub fn status(value: &str) -> SessionStatus {
    match value {
        "starting" => SessionStatus::Starting,
        "working" => SessionStatus::Working,
        "waiting" => SessionStatus::Waiting,
        "warm" => SessionStatus::Warm,
        "checkpointing" => SessionStatus::Checkpointing,
        "ready" => SessionStatus::Ready,
        "failed" => SessionStatus::Failed,
        "stopped" => SessionStatus::Stopped,
        "resuming" => SessionStatus::Resuming,
        "restored" => SessionStatus::Restored,
        "completed" => SessionStatus::Completed,
        "cancelled" => SessionStatus::Cancelled,
        _ => SessionStatus::Idle,
    }
}
/// Interpret the `harness` column. Never guesses: an id this build cannot
/// parse is preserved as [`Harness::Unknown`] rather than being read as some
/// other harness. See [`Harness::from_stored`].
pub fn harness(value: &str) -> Harness {
    Harness::from_stored(value)
}
fn capability_tier(value: Option<String>) -> Option<CapabilityTier> {
    match value.as_deref() {
        Some("fast") => Some(CapabilityTier::Fast),
        Some("standard") => Some(CapabilityTier::Standard),
        Some("strong") => Some(CapabilityTier::Strong),
        _ => None,
    }
}
fn restoration_mode(value: &str) -> RestorationMode {
    match value {
        "hot" => RestorationMode::Hot,
        "native" => RestorationMode::Native,
        "native_fork" => RestorationMode::NativeFork,
        "checkpoint_restored" => RestorationMode::CheckpointRestored,
        _ => RestorationMode::Fresh,
    }
}
fn resume_eligibility(value: &str) -> ResumeEligibility {
    match value {
        "native" => ResumeEligibility::Native,
        "checkpoint_restored" => ResumeEligibility::CheckpointRestored,
        _ => ResumeEligibility::Fresh,
    }
}

fn continuation_fidelity(value: &str) -> ContinuationFidelity {
    match value {
        "projected_at_boundary" => ContinuationFidelity::ProjectedAtBoundary,
        "projected_mid_turn" => ContinuationFidelity::ProjectedMidTurn,
        _ => ContinuationFidelity::Native,
    }
}
/// The value written to the `harness` column, which is also the wire id and
/// the adapter-registry key. See [`Harness::id`].
pub fn harness_name(value: &Harness) -> std::borrow::Cow<'static, str> {
    value.id()
}

#[allow(clippy::too_many_arguments)]
pub fn append_session_entry(
    db: &Connection,
    session_id: &str,
    parent_entry_id: Option<&str>,
    kind: &str,
    payload: &serde_json::Value,
    provider_event_id: Option<&str>,
    context_visibility: &str,
    token_estimate: Option<i64>,
) -> Result<SessionEntry, BridgeError> {
    let transaction = db.unchecked_transaction()?;
    let entry = append_session_entry_tx(
        &transaction,
        session_id,
        parent_entry_id,
        kind,
        payload,
        provider_event_id,
        context_visibility,
        token_estimate,
    )?;
    transaction.commit()?;
    Ok(entry)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn append_session_entry_tx(
    transaction: &Transaction<'_>,
    session_id: &str,
    parent_entry_id: Option<&str>,
    kind: &str,
    payload: &serde_json::Value,
    provider_event_id: Option<&str>,
    context_visibility: &str,
    token_estimate: Option<i64>,
) -> Result<SessionEntry, BridgeError> {
    let sequence = transaction.query_row(
        "SELECT COALESCE(MAX(sequence),0)+1 FROM session_entries WHERE session_id=?1",
        params![session_id],
        |row| row.get(0),
    )?;
    let mut stored_payload = payload.clone();
    let object = stored_payload.as_object_mut().ok_or_else(|| {
        BridgeError::Invalid("session entry payload must be a JSON object".into())
    })?;
    object.insert(
        "_bridgeRepoState".into(),
        repository_state_for_session(transaction, session_id)?,
    );
    let entry = SessionEntry {
        id: Uuid::new_v4().to_string(),
        session_id: session_id.to_owned(),
        parent_entry_id: parent_entry_id.map(str::to_owned),
        sequence,
        semantic_schema_version: SEMANTIC_EVENT_SCHEMA_VERSION,
        kind: kind.to_owned(),
        payload: stored_payload,
        provider_event_id: provider_event_id.map(str::to_owned),
        context_visibility: context_visibility.to_owned(),
        token_estimate,
        created_at: Utc::now().to_rfc3339(),
    };
    transaction.execute(
        "INSERT INTO session_entries(id,session_id,parent_entry_id,sequence,semantic_schema_version,kind,payload,provider_event_id,context_visibility,token_estimate,created_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
        params![
            entry.id,
            entry.session_id,
            entry.parent_entry_id,
            entry.sequence,
            entry.semantic_schema_version,
            entry.kind,
            entry.payload.to_string(),
            entry.provider_event_id,
            entry.context_visibility,
            entry.token_estimate,
            entry.created_at,
        ],
    )?;
    transaction.execute(
        "INSERT INTO session_heads(session_id,active_entry_id,native_provider_session_id,restoration_mode,updated_at)
         VALUES(?1,?2,(SELECT provider_session_id FROM sessions WHERE id=?1),'fresh',?3)
         ON CONFLICT(session_id) DO UPDATE SET active_entry_id=excluded.active_entry_id,updated_at=excluded.updated_at",
        params![entry.session_id, entry.id, entry.created_at],
    )?;
    Ok(entry)
}

pub fn repository_state_for_session(
    db: &Connection,
    session_id: &str,
) -> Result<serde_json::Value, BridgeError> {
    let Some(path) = repository_path_for_session(db, session_id)? else {
        return Ok(serde_json::json!({"status":"unavailable"}));
    };
    Ok(repository_state_for_path(&path))
}

pub fn repository_path_for_session(
    db: &Connection,
    session_id: &str,
) -> Result<Option<PathBuf>, BridgeError> {
    let path: Option<String> = db.query_row(
        "SELECT COALESCE(s.cwd,w.path) FROM sessions s LEFT JOIN workspaces w ON w.id=s.workspace_id WHERE s.id=?1",
        params![session_id],
        |row| row.get(0),
    ).optional()?.flatten();
    let path = path.map(PathBuf::from);
    // A private chat has no connected repository. Git's normal ancestor
    // discovery can otherwise reach a home-directory repository and scan it
    // while a durable event holds the shared database lock. Only an explicit
    // repository initialized in this scratch directory belongs to the chat.
    if let (Some(path), Some(database_path)) = (&path, db.path()) {
        let chats = Path::new(database_path)
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("chats");
        // Asides share their source chat's scratch directory, so ownership
        // follows the directory's parent rather than this session's ID.
        // SQLite resolves aliases such as macOS /var -> /private/var. The
        // stored cwd may retain the spelling supplied by the caller.
        let is_scratch = path.parent() == Some(chats.as_path()) || std::fs::canonicalize(path)
            .and_then(|actual| std::fs::canonicalize(&chats).map(|owned| actual.parent() == Some(owned.as_path())))
            .unwrap_or(false);
        if is_scratch && !path.join(".git").exists() {
            return Ok(None);
        }
    }
    Ok(path)
}

/// The directory a session's base-branch facts describe: the workspace root
/// when the session belongs to a workspace — the same directory the Work
/// observer measures and the board projects — falling back to the session's
/// own cwd only for direct chats, which have no workspace row.
///
/// This deliberately differs from [`repository_path_for_session`], whose
/// cwd-first order answers "where is this session running". A drift fact
/// measured at the workspace root but acted on at the session cwd produced
/// issue #306: a fresh measurement of one directory next to a "not a git
/// repository" failure from another.
pub fn base_branch_path_for_session(
    db: &Connection,
    session_id: &str,
) -> Result<Option<PathBuf>, BridgeError> {
    let path: Option<String> = db.query_row(
        "SELECT w.path FROM sessions s LEFT JOIN workspaces w ON w.id=s.workspace_id WHERE s.id=?1",
        params![session_id],
        |row| row.get(0),
    ).optional()?.flatten();
    match path {
        Some(path) => Ok(Some(PathBuf::from(path))),
        None => repository_path_for_session(db, session_id),
    }
}

pub fn repository_state_for_path(path: &Path) -> serde_json::Value {
    let Ok(head) = crate::git::git_command(path)
        .args(["rev-parse", "HEAD"])
        .output() else {
            return serde_json::json!({"status":"unavailable"});
        };
    // This snapshot needs a commit. An unborn repository cannot provide one;
    // do not scan all its untracked files while holding a session transaction.
    // A repository above the workspace (including a user's home) can make that
    // unnecessary scan stall every daemon request and startup recovery.
    if !head.status.success() {
        return serde_json::json!({"status":"unavailable"});
    }
    let Ok(status) = crate::git::git_command(path)
        .args(["status", "--porcelain=v1", "-z", "--untracked-files=all"])
        .output() else {
            return serde_json::json!({"status":"unavailable"});
        };
    if !status.status.success() {
        return serde_json::json!({"status":"unavailable"});
    }
    let head = String::from_utf8_lossy(&head.stdout).trim().to_owned();
    serde_json::json!({
        "status": if status.stdout.is_empty() { "clean" } else { "dirty" },
        "head": head,
        "dirtyHash": stable_dirty_hash(&status.stdout),
    })
}

fn stable_dirty_hash(bytes: &[u8]) -> String {
    // FNV-1a is sufficient here: this is a deterministic change detector, not a
    // security boundary. Keeping the algorithm local makes stamps comparable
    // across controller restarts and Rust versions.
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

/// Durable events for a session with a sequence strictly greater than the
/// cursor, in sequence order — the replay half of the notify-then-replay
/// contract. Replayed events carry their durable forest kind (e.g.
/// `assistant.message`) and payload exactly as persisted; transient frames
/// (sequence 0 on the live channel) were never stored and are never replayed.
pub fn session_events_after(
    db: &Connection,
    session_id: &str,
    after_sequence: i64,
    limit: u32,
) -> Result<Vec<AgentEvent>, BridgeError> {
    let entries = query_with_params(
        db,
        "SELECT id,session_id,parent_entry_id,sequence,semantic_schema_version,kind,payload,provider_event_id,context_visibility,token_estimate,created_at
         FROM session_entries WHERE session_id=?1 AND sequence>?2 ORDER BY sequence LIMIT ?3",
        params![session_id, after_sequence, limit],
        |row| {
            Ok(SessionEntry {
                id: row.get(0)?,
                session_id: row.get(1)?,
                parent_entry_id: row.get(2)?,
                sequence: row.get(3)?,
                semantic_schema_version: row.get(4)?,
                kind: row.get(5)?,
                payload: parse_json_column(row, 6),
                provider_event_id: row.get(7)?,
                context_visibility: row.get(8)?,
                token_estimate: row.get(9)?,
                created_at: row.get(10)?,
            })
        },
    )?;
    session_entries_to_events(db, entries)
}

pub fn session_events_tail(
    db: &Connection,
    session_id: &str,
    limit: u32,
) -> Result<Vec<AgentEvent>, BridgeError> {
    let entries = query_with_params(
        db,
        "SELECT id,session_id,parent_entry_id,sequence,semantic_schema_version,kind,payload,provider_event_id,context_visibility,token_estimate,created_at
         FROM (
             SELECT id,session_id,parent_entry_id,sequence,semantic_schema_version,kind,payload,provider_event_id,context_visibility,token_estimate,created_at
             FROM session_entries WHERE session_id=?1 ORDER BY sequence DESC LIMIT ?2
         ) ORDER BY sequence",
        params![session_id, limit],
        |row| {
            Ok(SessionEntry {
                id: row.get(0)?,
                session_id: row.get(1)?,
                parent_entry_id: row.get(2)?,
                sequence: row.get(3)?,
                semantic_schema_version: row.get(4)?,
                kind: row.get(5)?,
                payload: parse_json_column(row, 6),
                provider_event_id: row.get(7)?,
                context_visibility: row.get(8)?,
                token_estimate: row.get(9)?,
                created_at: row.get(10)?,
            })
        },
    )?;
    session_entries_to_events(db, entries)
}

fn session_entries_to_events(
    db: &Connection,
    entries: Vec<SessionEntry>,
) -> Result<Vec<AgentEvent>, BridgeError> {
    let forest = crate::session_forest::SessionForest::new(db);
    entries
        .into_iter()
        .map(|entry| {
            forest.validate_stored_entry(&entry).map_err(|error| {
                BridgeError::Invalid(format!(
                    "cannot replay session entry {} at sequence {}: {error}",
                    entry.id, entry.sequence
                ))
            })?;
            let payload = &entry.payload;
            let field = |name: &str| {
                payload
                    .get(name)
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned)
            };
            let typed_forest_payload = payload
                .get(crate::session_forest::TYPED_SCHEMA_MARKER)
                .and_then(serde_json::Value::as_u64)
                == Some(crate::session_forest::TYPED_SCHEMA_VERSION);
            let legacy_agent_envelope = !typed_forest_payload
                && payload
                    .get("protocolVersion")
                    .and_then(serde_json::Value::as_i64)
                    .is_some();
            Ok(AgentEvent {
                id: entry.sequence,
                session_id: entry.session_id,
                sequence: entry.sequence,
                protocol_version: if legacy_agent_envelope {
                    payload
                        .get("protocolVersion")
                        .and_then(serde_json::Value::as_i64)
                        .unwrap_or(1)
                } else {
                    1
                },
                kind: entry.kind,
                item_id: field("itemId"),
                role: field("role"),
                status: field("status"),
                title: field("title"),
                text: field("text"),
                data: if legacy_agent_envelope {
                    payload
                        .get("data")
                        .cloned()
                        .unwrap_or_else(|| serde_json::json!({}))
                } else {
                    payload.clone()
                },
                provider_meta: payload
                    .get("providerMeta")
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!({})),
                created_at: entry.created_at,
            })
        })
        .collect()
}

pub fn session_entries(
    db: &Connection,
    session_id: &str,
) -> Result<Vec<SessionEntry>, BridgeError> {
    query_with_params(
        db,
        "SELECT id,session_id,parent_entry_id,sequence,semantic_schema_version,kind,payload,provider_event_id,context_visibility,token_estimate,created_at
         FROM session_entries WHERE session_id=?1 ORDER BY sequence",
        params![session_id],
        |row| {
            Ok(SessionEntry {
                id: row.get(0)?,
                session_id: row.get(1)?,
                parent_entry_id: row.get(2)?,
                sequence: row.get(3)?,
                semantic_schema_version: row.get(4)?,
                kind: row.get(5)?,
                payload: parse_json_column(row, 6),
                provider_event_id: row.get(7)?,
                context_visibility: row.get(8)?,
                token_estimate: row.get(9)?,
                created_at: row.get(10)?,
            })
        },
    )
}

/// The display ceiling for one string inside a snapshot payload.
///
/// A transcript row renders a preview, never a 50 KB heredoc. The whole forest
/// travels to the UI as a single JSON frame, and untrimmed payloads made that
/// frame unopenable: one real chat's `command.started` entries alone held
/// 115 MB, because each one carries the full command text as its `title` and
/// the full command output under `data`. Past the daemon's 64 MB frame ceiling
/// the read fails and takes the whole connection down, so an old chat did not
/// load slowly — it did not load at all.
const SNAPSHOT_STRING_BYTES: usize = 4 * 1024;

/// The number of newest entries a snapshot carries.
///
/// Safe to truncate from the front: branch projection walks parent links from
/// the head backwards and stops at the first parent it cannot find, so a tail
/// window shortens the *start* of the transcript and never orphans the rest.
pub const SNAPSHOT_ENTRY_WINDOW: usize = 1500;

/// A bounded read of one session's entries, with the totals the UI needs to
/// tell the difference between "this is the whole conversation" and "this is
/// the tail of a longer one".
#[derive(Debug, Clone)]
pub struct SessionEntryWindow {
    pub entries: Vec<SessionEntry>,
    pub total: i64,
    pub trimmed_payloads: i64,
}

/// Shorten every oversized string in `value` in place, reporting whether
/// anything was cut. Structure is preserved: the transcript codec reads named
/// fields (`text`, `title`, nested `data`), so trimming has to leave those
/// fields present and merely shorter.
fn trim_snapshot_strings(value: &mut serde_json::Value) -> bool {
    match value {
        serde_json::Value::String(text) => {
            if text.len() <= SNAPSHOT_STRING_BYTES {
                return false;
            }
            let mut end = SNAPSHOT_STRING_BYTES;
            while end > 0 && !text.is_char_boundary(end) {
                end -= 1;
            }
            let dropped = text.len() - end;
            text.truncate(end);
            text.push_str(&format!("\n… {dropped} more bytes not shown"));
            true
        }
        serde_json::Value::Array(items) => items.iter_mut().fold(false, |trimmed, item| {
            trim_snapshot_strings(item) || trimmed
        }),
        serde_json::Value::Object(fields) => {
            fields.iter_mut().fold(false, |trimmed, (_, field)| {
                trim_snapshot_strings(field) || trimmed
            })
        }
        _ => false,
    }
}

/// The newest `limit` entries of a session, oldest-first, with oversized
/// payload strings trimmed for display. `session_entries` stays the untrimmed
/// read for callers that need real payloads (compaction, context projection);
/// this one exists only to make the snapshot a bounded frame.
pub fn session_entry_window(
    db: &Connection,
    session_id: &str,
    limit: usize,
) -> Result<SessionEntryWindow, BridgeError> {
    let total: i64 = db.query_row(
        "SELECT count(*) FROM session_entries WHERE session_id=?1",
        params![session_id],
        |row| row.get(0),
    )?;
    let mut trimmed_payloads = 0i64;
    let mut entries = query_with_params(
        db,
        "SELECT id,session_id,parent_entry_id,sequence,semantic_schema_version,kind,payload,provider_event_id,context_visibility,token_estimate,created_at
         FROM session_entries WHERE session_id=?1 ORDER BY sequence DESC LIMIT ?2",
        params![session_id, limit as i64],
        |row| {
            let mut payload = parse_json_column(row, 6);
            if trim_snapshot_strings(&mut payload) {
                trimmed_payloads += 1;
            }
            Ok(SessionEntry {
                id: row.get(0)?,
                session_id: row.get(1)?,
                parent_entry_id: row.get(2)?,
                sequence: row.get(3)?,
                semantic_schema_version: row.get(4)?,
                kind: row.get(5)?,
                payload,
                provider_event_id: row.get(7)?,
                context_visibility: row.get(8)?,
                token_estimate: row.get(9)?,
                created_at: row.get(10)?,
            })
        },
    )?;
    entries.reverse();
    Ok(SessionEntryWindow {
        entries,
        total,
        trimmed_payloads,
    })
}

pub fn session_head(db: &Connection, session_id: &str) -> Result<Option<SessionHead>, BridgeError> {
    db.query_row(
        "SELECT session_id,active_entry_id,native_provider_session_id,restoration_mode,resume_eligibility,latest_checkpoint_entry_id,updated_at
         FROM session_heads WHERE session_id=?1",
        params![session_id],
        |row| {
            Ok(SessionHead {
                session_id: row.get(0)?,
                active_entry_id: row.get(1)?,
                native_provider_session_id: row.get(2)?,
                restoration_mode: restoration_mode(&row.get::<_, String>(3)?),
                resume_eligibility: resume_eligibility(&row.get::<_, String>(4)?),
                latest_checkpoint_entry_id: row.get(5)?,
                updated_at: row.get(6)?,
                            })
        },
    )
    .optional()
    .map_err(BridgeError::from)
}

pub fn upsert_worker_lease(db: &Connection, lease: &WorkerLease) -> Result<(), BridgeError> {
    db.execute(
        "INSERT INTO worker_leases(session_id,workspace_id,role,capability_tier,task_family,owned_paths,write_mode,lease_status,expires_at,created_at,updated_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)
         ON CONFLICT(session_id) DO UPDATE SET workspace_id=excluded.workspace_id,role=excluded.role,capability_tier=excluded.capability_tier,task_family=excluded.task_family,owned_paths=excluded.owned_paths,write_mode=excluded.write_mode,lease_status=excluded.lease_status,expires_at=excluded.expires_at,updated_at=excluded.updated_at",
        params![
            lease.session_id,
            lease.workspace_id,
            lease.role,
            lease.capability_tier,
            lease.task_family,
            lease.owned_paths.to_string(),
            lease.write_mode,
            lease.lease_status,
            lease.expires_at,
            lease.created_at,
            lease.updated_at,
        ],
    )?;
    Ok(())
}

pub fn worker_leases(db: &Connection, workspace_id: &str) -> Result<Vec<WorkerLease>, BridgeError> {
    query_with_params(
        db,
        "SELECT session_id,workspace_id,role,capability_tier,task_family,owned_paths,write_mode,lease_status,expires_at,created_at,updated_at
         FROM worker_leases WHERE workspace_id=?1 ORDER BY created_at,session_id",
        params![workspace_id],
        |row| {
            Ok(WorkerLease {
                session_id: row.get(0)?,
                workspace_id: row.get(1)?,
                role: row.get(2)?,
                capability_tier: row.get(3)?,
                task_family: row.get(4)?,
                owned_paths: parse_json_column(row, 5),
                write_mode: row.get(6)?,
                lease_status: row.get(7)?,
                expires_at: row.get(8)?,
                created_at: row.get(9)?,
                updated_at: row.get(10)?,
                            })
        },
    )
}

pub fn claim_delegation_receipt(
    db: &Connection,
    session_id: &str,
    item_id: &str,
) -> Result<bool, BridgeError> {
    let dedupe_key = format!("{session_id}::{item_id}");
    Ok(db.execute(
        "INSERT OR IGNORE INTO delegation_receipts(dedupe_key,session_id,item_id,created_at) VALUES(?1,?2,?3,?4)",
        params![dedupe_key, session_id, item_id, Utc::now().to_rfc3339()],
    )? == 1)
}

pub fn upsert_worker_runtime(
    db: &Connection,
    runtime: &WorkerRuntimeRecord,
) -> Result<(), BridgeError> {
    db.execute(
        "INSERT INTO worker_runtime(session_id,parent_session_id,lifecycle_state,task_family,compatibility_key,result_status,retry_count,warm_until,worktree_path,worktree_branch,last_result,last_activity_at,updated_at,failure_class)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)
         ON CONFLICT(session_id) DO UPDATE SET parent_session_id=excluded.parent_session_id,lifecycle_state=excluded.lifecycle_state,task_family=excluded.task_family,compatibility_key=excluded.compatibility_key,result_status=excluded.result_status,retry_count=excluded.retry_count,warm_until=excluded.warm_until,worktree_path=excluded.worktree_path,worktree_branch=excluded.worktree_branch,last_result=excluded.last_result,last_activity_at=excluded.last_activity_at,updated_at=excluded.updated_at,failure_class=excluded.failure_class",
        params![runtime.session_id,runtime.parent_session_id,runtime.lifecycle_state,runtime.task_family,runtime.compatibility_key,runtime.result_status,runtime.retry_count,runtime.warm_until,runtime.worktree_path,runtime.worktree_branch,runtime.last_result.as_ref().map(serde_json::Value::to_string),runtime.last_activity_at,runtime.updated_at,runtime.failure_class],
    )?;
    Ok(())
}

pub fn worker_runtime(
    db: &Connection,
    session_id: &str,
) -> Result<Option<WorkerRuntimeRecord>, BridgeError> {
    db.query_row(
        "SELECT session_id,parent_session_id,lifecycle_state,task_family,compatibility_key,result_status,retry_count,warm_until,worktree_path,worktree_branch,last_result,last_activity_at,waiting_since,waiting_reason,progress_summary,updated_at,failure_class FROM worker_runtime WHERE session_id=?1",
        params![session_id],
        |row| Ok(WorkerRuntimeRecord { session_id:row.get(0)?, parent_session_id:row.get(1)?, lifecycle_state:row.get(2)?, task_family:row.get(3)?, compatibility_key:row.get(4)?, result_status:row.get(5)?, retry_count:row.get(6)?, warm_until:row.get(7)?, worktree_path:row.get(8)?, worktree_branch:row.get(9)?, last_result:row.get::<_,Option<String>>(10)?.and_then(|value| serde_json::from_str(&value).ok()), last_activity_at:row.get(11)?, waiting_since:row.get(12)?, waiting_reason:row.get(13)?, progress_summary:row.get(14)?, updated_at:row.get(15)?, failure_class:row.get(16)? }),
    ).optional().map_err(BridgeError::from)
}

pub fn worker_runtimes(
    db: &Connection,
    workspace_id: &str,
) -> Result<Vec<WorkerRuntimeRecord>, BridgeError> {
    query_with_params(
        db,
        "SELECT r.session_id,r.parent_session_id,r.lifecycle_state,r.task_family,r.compatibility_key,r.result_status,r.retry_count,r.warm_until,r.worktree_path,r.worktree_branch,r.last_result,r.last_activity_at,r.waiting_since,r.waiting_reason,r.progress_summary,r.updated_at,r.failure_class
         FROM worker_runtime r JOIN sessions s ON s.id=r.session_id
         WHERE s.workspace_id=?1 ORDER BY s.rowid",
        params![workspace_id],
        |row| {
            Ok(WorkerRuntimeRecord {
                session_id: row.get(0)?,
                parent_session_id: row.get(1)?,
                lifecycle_state: row.get(2)?,
                task_family: row.get(3)?,
                compatibility_key: row.get(4)?,
                result_status: row.get(5)?,
                retry_count: row.get(6)?,
                warm_until: row.get(7)?,
                worktree_path: row.get(8)?,
                worktree_branch: row.get(9)?,
                last_result: row
                    .get::<_, Option<String>>(10)?
                    .and_then(|value| serde_json::from_str(&value).ok()),
                last_activity_at: row.get(11)?,
                waiting_since: row.get(12)?,
                waiting_reason: row.get(13)?,
                progress_summary: row.get(14)?,
                updated_at: row.get(15)?,
                failure_class: row.get(16)?,
            })
        },
    )
}

pub fn outstanding_children(db: &Connection, parent_session_id: &str) -> Result<i64, BridgeError> {
    Ok(db.query_row(
        "SELECT COUNT(*) FROM worker_runtime WHERE parent_session_id=?1 AND result_status!='reported'",
        params![parent_session_id],
        |row| row.get(0),
    )?)
}

pub fn enqueue_worker_request(
    db: &Connection,
    request: &QueuedWorkerRequest,
) -> Result<(), BridgeError> {
    db.execute(
        "INSERT INTO worker_queue(id,parent_session_id,workspace_id,turn_id,request,actual_model,queue_status,dispatched_session_id,attempt_count,expires_at,blocked_at,claimed_at,last_error,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)",
        params![request.id,request.parent_session_id,request.workspace_id,request.turn_id,request.request.to_string(),request.actual_model,request.queue_status,request.dispatched_session_id,request.attempt_count,request.expires_at,request.blocked_at,request.claimed_at,request.last_error,request.created_at,request.updated_at],
    )?;
    Ok(())
}

pub fn queued_worker_requests(
    db: &Connection,
    workspace_id: &str,
) -> Result<Vec<QueuedWorkerRequest>, BridgeError> {
    query_with_params(
        db,
        "SELECT id,parent_session_id,workspace_id,turn_id,request,actual_model,queue_status,sequence,dispatched_session_id,attempt_count,expires_at,blocked_at,claimed_at,last_error,created_at,updated_at FROM worker_queue WHERE workspace_id=?1 AND queue_status='queued' ORDER BY sequence",
        params![workspace_id],
        |row| Ok(QueuedWorkerRequest { id:row.get(0)?, parent_session_id:row.get(1)?, workspace_id:row.get(2)?, turn_id:row.get(3)?, request:parse_json_column(row,4), actual_model:row.get(5)?, queue_status:row.get(6)?, sequence:row.get(7)?, dispatched_session_id:row.get(8)?, attempt_count:row.get(9)?, expires_at:row.get(10)?, blocked_at:row.get(11)?, claimed_at:row.get(12)?, last_error:row.get(13)?, created_at:row.get(14)?, updated_at:row.get(15)? }),
    )
}

pub fn worker_queue_requests(
    db: &Connection,
    workspace_id: &str,
) -> Result<Vec<QueuedWorkerRequest>, BridgeError> {
    query_with_params(
        db,
        "SELECT id,parent_session_id,workspace_id,turn_id,request,actual_model,queue_status,sequence,dispatched_session_id,attempt_count,expires_at,blocked_at,claimed_at,last_error,created_at,updated_at
         FROM worker_queue WHERE workspace_id=?1 ORDER BY sequence",
        params![workspace_id],
        |row| {
            Ok(QueuedWorkerRequest {
                id: row.get(0)?,
                parent_session_id: row.get(1)?,
                workspace_id: row.get(2)?,
                turn_id: row.get(3)?,
                request: parse_json_column(row, 4),
                actual_model: row.get(5)?,
                queue_status: row.get(6)?,
                sequence: row.get(7)?,
                dispatched_session_id: row.get(8)?,
                attempt_count: row.get(9)?, expires_at: row.get(10)?, blocked_at: row.get(11)?, claimed_at: row.get(12)?, last_error: row.get(13)?, created_at: row.get(14)?, updated_at: row.get(15)?,
            })
        },
    )
}

pub fn fair_queued_workspaces(db: &Connection) -> Result<Vec<String>, BridgeError> {
    query_with_params(db, "SELECT workspace_id FROM worker_queue WHERE queue_status='queued' GROUP BY workspace_id ORDER BY MIN(sequence),workspace_id", [], |row| row.get(0))
}

pub fn enqueue_outbox(
    transaction: &Transaction<'_>,
    message: &OutboxMessage,
) -> Result<(), BridgeError> {
    transaction.execute("INSERT INTO durable_outbox(id,destination,event_type,payload,idempotency_key,status,attempt_count,next_attempt_at,last_error,created_at,delivered_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11) ON CONFLICT(idempotency_key) DO NOTHING", params![message.id,message.destination,message.event_type,message.payload.to_string(),message.idempotency_key,message.status,message.attempt_count,message.next_attempt_at,message.last_error,message.created_at,message.delivered_at])?;
    Ok(())
}

/// The number of newest reason events a snapshot carries. The panel reads as a
/// recent-activity feed and always did — but the query had no `LIMIT`, so
/// opening any chat in a busy workspace pulled every event ever recorded for
/// every session in it (26,430 rows in one real workspace).
pub const SNAPSHOT_REASON_WINDOW: usize = 500;

pub fn workspace_reason_events(
    db: &Connection,
    workspace_id: &str,
    limit: usize,
) -> Result<Vec<BridgeEvent>, BridgeError> {
    query_with_params(
        db,
        "SELECT id,source,kind,entity_id,body,created_at FROM events
         WHERE entity_id=?1
            OR entity_id IN (SELECT id FROM sessions WHERE workspace_id=?1)
            OR entity_id IN (SELECT id FROM worker_queue WHERE workspace_id=?1)
         ORDER BY id DESC LIMIT ?2",
        params![workspace_id, limit as i64],
        |row| {
            Ok(BridgeEvent {
                id: row.get(0)?,
                source: row.get(1)?,
                kind: row.get(2)?,
                entity_id: row.get(3)?,
                body: row.get(4)?,
                created_at: row.get(5)?,
            })
        },
    )
}

pub fn update_worker_queue(
    db: &Connection,
    id: &str,
    queue_status: &str,
    dispatched_session_id: Option<&str>,
) -> Result<bool, BridgeError> {
    Ok(db.execute(
        "UPDATE worker_queue SET queue_status=?2,dispatched_session_id=COALESCE(?3,dispatched_session_id),updated_at=?4 WHERE id=?1",
        params![id, queue_status, dispatched_session_id, Utc::now().to_rfc3339()],
    )? == 1)
}

pub fn append_usage_ledger(db: &Connection, usage: &UsageLedgerRow) -> Result<i64, BridgeError> {
    db.execute(
        "INSERT INTO usage_ledger(workspace_id,session_id,turn_id,input_tokens,output_tokens,cache_read_tokens,cache_write_tokens,uncached_input_tokens,context_percent,capability_units,runtime_ms,cost_microusd,cost_source,stable_prefix_id,stable_prefix_hash,prompt_schema_version,prefix_token_estimate,harness,model,role,task_family,restoration_mode,cross_harness_reuse,source,created_at,reasoning_tokens,serving_model,context_window_tokens,context_used_tokens,provider_record_id,cache_savings_microusd)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24,?25,?26,?27,?28,?29,?30,?31)",
        params![
            usage.workspace_id,
            usage.session_id,
            usage.turn_id,
            usage.input_tokens,
            usage.output_tokens,
            usage.cache_read_tokens,
            usage.cache_write_tokens,
            usage.uncached_input_tokens,
            usage.context_percent,
            usage.capability_units,
            usage.runtime_ms,
            usage.cost_microusd,
            usage.cost_source,
            usage.stable_prefix_id,
            usage.stable_prefix_hash,
            usage.prompt_schema_version,
            usage.prefix_token_estimate,
            usage.harness,
            usage.model,
            usage.role,
            usage.task_family,
            usage.restoration_mode,
            usage.cross_harness_reuse,
            usage.source,
            usage.created_at,
            usage.reasoning_tokens,
            usage.serving_model,
            usage.context_window_tokens,
            usage.context_used_tokens,
            usage.provider_record_id,
            usage.cache_savings_microusd,
        ],
    )?;
    Ok(db.last_insert_rowid())
}

pub fn usage_ledger(
    db: &Connection,
    workspace_id: &str,
    session_id: Option<&str>,
) -> Result<Vec<UsageLedgerRow>, BridgeError> {
    let sql = "SELECT id,workspace_id,session_id,turn_id,input_tokens,output_tokens,cache_read_tokens,cache_write_tokens,uncached_input_tokens,context_percent,capability_units,runtime_ms,cost_microusd,cost_source,stable_prefix_id,stable_prefix_hash,prompt_schema_version,prefix_token_estimate,harness,model,role,task_family,restoration_mode,cross_harness_reuse,source,created_at,reasoning_tokens,serving_model,context_window_tokens,context_used_tokens,provider_record_id,cache_savings_microusd
               FROM usage_ledger WHERE workspace_id=?1 AND (?2 IS NULL OR session_id=?2) ORDER BY id";
    query_with_params(db, sql, params![workspace_id, session_id], |row| {
        Ok(UsageLedgerRow {
            id: row.get(0)?,
            workspace_id: row.get(1)?,
            session_id: row.get(2)?,
            turn_id: row.get(3)?,
            input_tokens: row.get(4)?,
            output_tokens: row.get(5)?,
            cache_read_tokens: row.get(6)?,
            cache_write_tokens: row.get(7)?,
            uncached_input_tokens: row.get(8)?,
            context_percent: row.get(9)?,
            capability_units: row.get(10)?,
            runtime_ms: row.get(11)?,
            cost_microusd: row.get(12)?,
            cost_source: row.get(13)?,
            stable_prefix_id: row.get(14)?,
            stable_prefix_hash: row.get(15)?,
            prompt_schema_version: row.get(16)?,
            prefix_token_estimate: row.get(17)?,
            harness: row.get(18)?,
            model: row.get(19)?,
            role: row.get(20)?,
            task_family: row.get(21)?,
            restoration_mode: row.get(22)?,
            cross_harness_reuse: row.get(23)?,
            source: row.get(24)?,
            created_at: row.get(25)?,
            reasoning_tokens: row.get(26)?,
            serving_model: row.get(27)?,
            context_window_tokens: row.get(28)?,
            context_used_tokens: row.get(29)?,
            provider_record_id: row.get(30)?,
            cache_savings_microusd: row.get(31)?,
        })
    })
}

pub fn record_prompt_compilation(
    db: &Connection,
    record: &PromptCompilationRecord,
) -> Result<i64, BridgeError> {
    db.execute(
        "INSERT INTO prompt_compilations(session_id,turn_id,prefix_id,prefix_hash,schema_version,prefix_bytes,prefix_token_estimate,harness,model,role,task_family,restoration_mode,cross_harness_reuse,created_at,sections_json,stable_bytes,variable_bytes,stable_token_estimate,variable_token_estimate,token_estimate_source)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20)",
        params![record.session_id,record.turn_id,record.prefix_id,record.prefix_hash,record.schema_version,record.prefix_bytes,record.prefix_token_estimate,record.harness,record.model,record.role,record.task_family,record.restoration_mode,record.cross_harness_reuse,record.created_at,record.sections_json,record.stable_bytes,record.variable_bytes,record.stable_token_estimate,record.variable_token_estimate,record.token_estimate_source],
    )?;
    Ok(db.last_insert_rowid())
}

/// The harness and model a session is bound to.
///
/// Usage rows fall back to this when no prompt compilation matches their turn,
/// so a provider row is never written without a harness while the session row
/// exists.
pub fn session_harness_and_model(
    db: &Connection,
    session_id: &str,
) -> Result<Option<(String, Option<String>)>, BridgeError> {
    Ok(db
        .query_row(
            "SELECT harness,model FROM sessions WHERE id=?1",
            params![session_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?)
}

pub fn latest_prompt_compilation(
    db: &Connection,
    session_id: &str,
) -> Result<Option<PromptCompilationRecord>, BridgeError> {
    Ok(db.query_row(
        "SELECT id,session_id,turn_id,prefix_id,prefix_hash,schema_version,prefix_bytes,prefix_token_estimate,harness,model,role,task_family,restoration_mode,cross_harness_reuse,created_at,sections_json,stable_bytes,variable_bytes,stable_token_estimate,variable_token_estimate,token_estimate_source
         FROM prompt_compilations WHERE session_id=?1 ORDER BY id DESC LIMIT 1",
        params![session_id],
        |row| Ok(PromptCompilationRecord { id:row.get(0)?, session_id:row.get(1)?, turn_id:row.get(2)?, prefix_id:row.get(3)?, prefix_hash:row.get(4)?, schema_version:row.get(5)?, prefix_bytes:row.get(6)?, prefix_token_estimate:row.get(7)?, harness:row.get(8)?, model:row.get(9)?, role:row.get(10)?, task_family:row.get(11)?, restoration_mode:row.get(12)?, cross_harness_reuse:row.get(13)?, created_at:row.get(14)?, sections_json:row.get(15)?, stable_bytes:row.get(16)?, variable_bytes:row.get(17)?, stable_token_estimate:row.get(18)?, variable_token_estimate:row.get(19)?, token_estimate_source:row.get(20)? }),
    ).optional()?)
}

pub fn bind_latest_prompt_compilation_to_turn(
    db: &Connection,
    session_id: &str,
    turn_id: &str,
) -> Result<bool, BridgeError> {
    Ok(db.execute(
        "UPDATE prompt_compilations SET turn_id=?2 WHERE id=(SELECT id FROM prompt_compilations WHERE session_id=?1 AND turn_id IS NULL ORDER BY id DESC LIMIT 1)",
        params![session_id, turn_id],
    )? == 1)
}

pub fn prompt_compilation_for_turn(
    db: &Connection,
    session_id: &str,
    turn_id: &str,
) -> Result<Option<PromptCompilationRecord>, BridgeError> {
    Ok(db.query_row(
        "SELECT id,session_id,turn_id,prefix_id,prefix_hash,schema_version,prefix_bytes,prefix_token_estimate,harness,model,role,task_family,restoration_mode,cross_harness_reuse,created_at,sections_json,stable_bytes,variable_bytes,stable_token_estimate,variable_token_estimate,token_estimate_source
         FROM prompt_compilations WHERE session_id=?1 AND turn_id=?2 ORDER BY id DESC LIMIT 1",
        params![session_id, turn_id],
        |row| Ok(PromptCompilationRecord { id:row.get(0)?, session_id:row.get(1)?, turn_id:row.get(2)?, prefix_id:row.get(3)?, prefix_hash:row.get(4)?, schema_version:row.get(5)?, prefix_bytes:row.get(6)?, prefix_token_estimate:row.get(7)?, harness:row.get(8)?, model:row.get(9)?, role:row.get(10)?, task_family:row.get(11)?, restoration_mode:row.get(12)?, cross_harness_reuse:row.get(13)?, created_at:row.get(14)?, sections_json:row.get(15)?, stable_bytes:row.get(16)?, variable_bytes:row.get(17)?, stable_token_estimate:row.get(18)?, variable_token_estimate:row.get(19)?, token_estimate_source:row.get(20)? }),
    ).optional()?)
}

pub fn delete_prompt_compilation(db: &Connection, id: i64) -> Result<bool, BridgeError> {
    Ok(db.execute("DELETE FROM prompt_compilations WHERE id=?1", params![id])? == 1)
}

fn query_with_params<T, P, F>(
    db: &Connection,
    sql: &str,
    params: P,
    mut map: F,
) -> Result<Vec<T>, BridgeError>
where
    P: rusqlite::Params,
    F: FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
{
    let mut statement = db.prepare(sql)?;
    let rows = statement.query_map(params, |row| map(row))?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

fn parse_json_column(row: &rusqlite::Row<'_>, index: usize) -> serde_json::Value {
    row.get::<_, String>(index)
        .ok()
        .and_then(|value| serde_json::from_str(&value).ok())
        .unwrap_or(serde_json::Value::Null)
}

pub fn session_event(
    db: &Connection,
    session_id: &str,
    event: &crate::agent::NormalizedEvent,
    provider_meta: &serde_json::Value,
) -> Result<AgentEvent, BridgeError> {
    event.validate().map_err(BridgeError::Invalid)?;
    let transaction = db.unchecked_transaction()?;
    let stored = session_event_in_transaction(&transaction, session_id, event, provider_meta)?;
    transaction.commit()?;
    Ok(stored)
}

/// Append a durable agent event inside a caller-owned transaction.
///
/// Callers that update related session state use this helper so the state
/// change, audit records, and durable notification history commit together.
pub(crate) fn session_event_in_transaction(
    transaction: &Transaction<'_>,
    session_id: &str,
    event: &crate::agent::NormalizedEvent,
    provider_meta: &serde_json::Value,
) -> Result<AgentEvent, BridgeError> {
    event.validate().map_err(BridgeError::Invalid)?;
    let parent_entry_id: Option<String> = transaction
        .query_row(
            "SELECT active_entry_id FROM session_heads WHERE session_id=?1",
            params![session_id],
            |row| row.get(0),
        )
        .optional()?
        .flatten();
    let trace_id: String = transaction
        .query_row(
            "SELECT COALESCE(trace_id,id) FROM sessions WHERE id=?1",
            params![session_id],
            |row| row.get(0),
        )
        .unwrap_or_else(|_| session_id.to_owned());
    let payload = serde_json::json!({
        "protocolVersion": 1,
        "itemId": event.item_id,
        "role": event.role,
        "status": event.status,
        "title": event.title,
        "text": event.text,
        "data": event.data,
        "providerMeta": provider_meta,
        "traceId": trace_id,
    });
    let mut final_kind = event.kind.as_str();
    if final_kind == "message.completed" {
        final_kind = if event.role.as_deref() == Some("user") {
            "user.message"
        } else {
            "assistant.message"
        };
    } else if final_kind == "tool.started"
        || final_kind == "tool.completed"
        || final_kind == "approval.requested"
        || final_kind == "approval.resolved"
        || final_kind == "delegation.requested"
        || final_kind == "delegation.approved"
        || final_kind == "delegation.rejected"
        || final_kind == "worker.result"
    {
        // Keep as is, it maps directly.
    } else if final_kind.ends_with(".delta")
        || final_kind.ends_with(".progress")
        || final_kind == "turn.started"
        || final_kind == "turn.completed"
        || final_kind == "usage.updated"
        || final_kind == "plan.updated"
        || final_kind == "question.settled"
    {
        // Do not store transient or internal events in the immutable forest.
        // `question.settled` is a control signal telling `live_turn.rs` to
        // resolve an existing `approval.requested` row; it is not itself a
        // durable conversation item.
        return Ok(AgentEvent {
            id: 0,
            session_id: session_id.into(),
            sequence: 0,
            protocol_version: 1,
            kind: event.kind.clone(),
            item_id: event.item_id.clone(),
            role: event.role.clone(),
            status: event.status.clone(),
            title: event.title.clone(),
            text: event.text.clone(),
            data: event.data.clone(),
            provider_meta: provider_meta.clone(),
            created_at: Utc::now().to_rfc3339(),
        });
    }

    let entry = append_session_entry_tx(
        transaction,
        session_id,
        parent_entry_id.as_deref(),
        final_kind,
        &payload,
        event.item_id.as_deref(),
        "eligible",
        None,
    )?;
    Ok(AgentEvent {
        id: entry.sequence,
        session_id: session_id.into(),
        sequence: entry.sequence,
        protocol_version: 1,
        kind: event.kind.clone(),
        item_id: event.item_id.clone(),
        role: event.role.clone(),
        status: event.status.clone(),
        title: event.title.clone(),
        text: event.text.clone(),
        data: event.data.clone(),
        provider_meta: provider_meta.clone(),
        created_at: entry.created_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_native_compaction_is_durable_history() {
        // The harness's own compaction boundary must survive the process that
        // reported it: it is the only evidence a provider context actually
        // shrank, and the transcript card is drawn from the stored entry on
        // every later reload.
        let db = open(Path::new(":memory:")).unwrap();
        db.execute(
            "INSERT INTO sessions(id,harness,label,status,metric_source) VALUES('s','claude','Chat','idle','reported')",
            [],
        )
        .unwrap();
        let mut event = crate::agent::NormalizedEvent::new(crate::agent::NATIVE_COMPACTION_KIND);
        event.status = Some("completed".into());
        event.title = Some("Context compacted".into());
        event.data = json!({"harness":"claude","trigger":"auto","preTokens":184_000,"postTokens":22_500});

        let stored = session_event(&db, "s", &event, &json!({})).unwrap();
        assert!(
            stored.sequence > 0,
            "a transient event returns sequence 0 and writes no row"
        );

        // Reading it back runs `validate_stored_entry`, which is where an
        // entry kind the forest cannot account for would be rejected.
        let replayed = session_events_tail(&db, "s", 10).unwrap();
        assert_eq!(replayed.len(), 1);
        assert_eq!(replayed[0].kind, crate::agent::NATIVE_COMPACTION_KIND);
        assert_eq!(replayed[0].data["harness"], "claude");
        assert_eq!(replayed[0].data["postTokens"], 22_500);
        assert_eq!(replayed[0].title.as_deref(), Some("Context compacted"));

        // It is history, not a Bridge compaction: nothing in the controller's
        // vocabulary was written.
        let bridge_boundaries: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM session_entries WHERE kind IN ('compaction','compaction.requested','compaction.failed','checkpoint')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(bridge_boundaries, 0);

        // The head advances to it, exactly as it does for any conversation
        // entry: a native boundary joins history rather than replacing it.
        // What it must not do is become a projection boundary, and it does not,
        // because only a `compaction` entry is one. `context::tests::
        // a_native_compaction_stays_in_the_projection` holds that end.
        let head: Option<String> = db
            .query_row(
                "SELECT active_entry_id FROM session_heads WHERE session_id='s'",
                [],
                |row| row.get(0),
            )
            .ok()
            .flatten();
        assert!(
            head.is_some(),
            "a durable entry advances the head like any other conversation entry"
        );
    }

    #[test]
    fn newer_schema_is_rejected_before_recovery_or_backup_pruning() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bridge.db");
        let db = open(&path).unwrap();
        db.execute(
            "INSERT INTO sessions(id,harness,label,status,active_turn_id)
             VALUES('future-session','claude','Future session','working','future-turn')",
            [],
        )
        .unwrap();
        db.execute(
            "UPDATE schema_version SET version=?1 WHERE version=?2",
            params![LATEST_SCHEMA_VERSION + 1, LATEST_SCHEMA_VERSION],
        )
        .unwrap();
        drop(db);

        let before = std::fs::read(&path).unwrap();
        let backups = [
            dir.path().join("bridge.db.backup-20260101T000000000000000Z"),
            dir.path().join("bridge.db.backup-20260102T000000000000000Z"),
        ];
        for backup in &backups {
            std::fs::copy(&path, backup).unwrap();
        }

        let error = open(&path).unwrap_err();
        assert!(matches!(&error, BridgeError::Invalid(_)));
        let message = error.to_string();
        assert!(message.contains(&(LATEST_SCHEMA_VERSION + 1).to_string()));
        assert!(message.contains(&format!("supports up to {LATEST_SCHEMA_VERSION}")));
        assert!(message.contains("newer Bridge version"));
        assert!(
            std::fs::read(&path).unwrap() == before,
            "the newer database is unchanged"
        );
        for backup in &backups {
            assert!(
                std::fs::read(backup).unwrap() == before,
                "rollback copies are untouched"
            );
        }

        let inspected =
            Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let state: (String, Option<String>, Option<String>) = inspected
            .query_row(
                "SELECT status,active_turn_id,ended_at FROM sessions WHERE id='future-session'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(state, ("working".into(), Some("future-turn".into()), None));
    }

    #[test]
    fn newer_schema_is_rejected_without_assuming_current_application_tables() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bridge.db");
        let db = Connection::open(&path).unwrap();
        db.execute_batch("CREATE TABLE schema_version(version INTEGER PRIMARY KEY)")
            .unwrap();
        db.execute(
            "INSERT INTO schema_version(version) VALUES(?1)",
            params![LATEST_SCHEMA_VERSION + 1],
        )
        .unwrap();
        drop(db);

        let error = open(&path).unwrap_err();
        assert!(matches!(&error, BridgeError::Invalid(_)));
        assert!(error.to_string().contains("newer Bridge version"));
    }

    #[test]
    fn newer_schema_with_a_hot_journal_is_not_recovered_by_preflight() {
        const FIXTURE_PATH: &str = "BRIDGE_TEST_FUTURE_DATABASE_JOURNAL";
        if let Some(path) = std::env::var_os(FIXTURE_PATH) {
            let db = Connection::open(PathBuf::from(path)).unwrap();
            db.execute_batch(
                "PRAGMA journal_mode=DELETE;
                 PRAGMA synchronous=FULL;
                 PRAGMA cache_size=1;
                 PRAGMA cache_spill=ON;
                 CREATE TABLE schema_version(version INTEGER PRIMARY KEY);
                 CREATE TABLE journal_fixture(payload BLOB);
                 INSERT INTO journal_fixture VALUES(zeroblob(65536));",
            )
            .unwrap();
            db.execute(
                "INSERT INTO schema_version VALUES(?1)",
                params![LATEST_SCHEMA_VERSION + 1],
            )
            .unwrap();
            db.execute_batch(
                "BEGIN IMMEDIATE;
                 UPDATE journal_fixture SET payload=randomblob(65536);",
            )
            .unwrap();
            // Simulate a crash after dirty pages spill, skipping SQLite's
            // connection destructor and its rollback of the active transaction.
            std::process::exit(0);
        }

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bridge.db");
        let journal = dir.path().join("bridge.db-journal");
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "store::tests::newer_schema_with_a_hot_journal_is_not_recovered_by_preflight",
                "--nocapture",
            ])
            .env(FIXTURE_PATH, &path)
            .output()
            .unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let before = std::fs::read(&path).unwrap();
        let journal_before = std::fs::read(&journal).unwrap();
        assert_eq!(
            &journal_before[..8],
            &[0xd9, 0xd5, 0x05, 0xf9, 0x20, 0xa1, 0x63, 0xd7],
            "the child must leave a hot rollback journal"
        );

        assert!(open(&path).is_err(), "recovery must not run before version validation");
        assert!(std::fs::read(&path).unwrap() == before, "database bytes changed");
        assert!(
            std::fs::read(&journal).unwrap() == journal_before,
            "the rollback journal must be left intact"
        );

        // Prove the fixture requires recovery: normal SQLite access rolls it
        // back, returning the future version and removing its hot journal.
        let recovering = Connection::open(&path).unwrap();
        assert_eq!(current_schema_version(&recovering).unwrap(), LATEST_SCHEMA_VERSION + 1);
        assert!(!journal.exists());
        assert!(std::fs::read(&path).unwrap() != before);
    }

    #[test]
    fn private_chat_stamps_do_not_discover_an_ancestor_repository() {
        let dir = tempfile::tempdir().unwrap();
        let git = |path: &Path, args: &[&str]| {
            let output = crate::git::git_command(path).args(args).output().unwrap();
            assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        };
        git(dir.path(), &["init", "-q"]);
        git(dir.path(), &["-c", "user.name=Test", "-c", "user.email=test@example.com",
            "commit", "--allow-empty", "-qm", "parent"]);
        let data = dir.path().join("app-data");
        std::fs::create_dir(&data).unwrap();
        let db = open(&data.join("bridge.db")).unwrap();
        let scratch = data.join("chats").join("chat");
        std::fs::create_dir_all(&scratch).unwrap();
        db.execute(
            "INSERT INTO sessions(id,harness,label,status,metric_source,cwd)
             VALUES('chat','codex','Chat','idle','estimated',?1)",
            params![scratch.to_string_lossy()],
        ).unwrap();
        // Asides inherit their source chat's cwd rather than getting a
        // scratch directory named after their own session ID.
        db.execute(
            "INSERT INTO sessions(id,harness,label,status,metric_source,cwd)
             SELECT 'aside',harness,'Aside',status,metric_source,cwd FROM sessions WHERE id='chat'",
            [],
        ).unwrap();
        for session_id in ["chat", "aside"] {
            assert_eq!(repository_path_for_session(&db, session_id).unwrap(), None);
            assert_eq!(base_branch_path_for_session(&db, session_id).unwrap(), None);
            assert_eq!(repository_state_for_session(&db, session_id).unwrap(), json!({"status":"unavailable"}));
        }

        // Initializing a repository explicitly in the chat still works.
        git(&scratch, &["init", "-q"]);
        git(&scratch, &["-c", "user.name=Test", "-c", "user.email=test@example.com",
            "commit", "--allow-empty", "-qm", "chat"]);
        for session_id in ["chat", "aside"] {
            assert_eq!(repository_path_for_session(&db, session_id).unwrap(), Some(scratch.clone()));
            assert_eq!(base_branch_path_for_session(&db, session_id).unwrap(), Some(scratch.clone()));
            assert_eq!(repository_state_for_session(&db, session_id).unwrap()["status"], "clean");
        }

        // A connected/imported cwd in a repository subdirectory retains
        // ordinary Git discovery; only Bridge's own private scratch is special.
        let connected = dir.path().join("source");
        std::fs::create_dir(&connected).unwrap();
        db.execute("UPDATE sessions SET cwd=?1 WHERE id='chat'", params![connected.to_string_lossy()]).unwrap();
        assert_eq!(repository_path_for_session(&db, "chat").unwrap(), Some(connected));
        assert_eq!(repository_state_for_session(&db, "chat").unwrap()["status"], "dirty");
    }

    #[test]
    fn an_unborn_repository_never_runs_the_status_scan() {
        let scratch = tempfile::tempdir().unwrap();
        let root = scratch.path();
        assert!(crate::git::git_command(root).args(["init", "-q"]).status().unwrap().success());
        std::fs::write(root.join("tracked.txt"), "pending first commit").unwrap();
        let before = crate::git::git_processes_started_on_this_thread();
        assert_eq!(repository_state_for_path(root), json!({"status":"unavailable"}));
        assert_eq!(crate::git::git_processes_started_on_this_thread() - before, 1,
            "the failed HEAD lookup must not be followed by a status scan");
    }

    #[test]
    fn the_harness_column_round_trips_every_shape() {
        // The column is TEXT and always has been, so opening the identifier
        // needs no schema migration — only an honest reading of what is there.
        for stored in [
            "claude",
            "codex",
            "opencode",
            "shell",
            "gemini",
            "github-copilot-cli",
            "mistral-vibe",
        ] {
            let parsed = harness(stored);
            assert_eq!(
                harness_name(&parsed),
                stored,
                "{stored:?} did not survive the column round trip"
            );
        }
    }

    #[test]
    fn an_unreadable_harness_row_never_becomes_a_runnable_harness() {
        // Regression for `_ => Harness::Shell`: an id this build cannot
        // interpret used to load as Shell, which is a real, runnable harness.
        for stored in ["", "SHELL", "Claude", "acp:gemini", "gem ini"] {
            let parsed = harness(stored);
            assert_eq!(parsed, Harness::Unknown(stored.to_owned()), "{stored:?}");
            assert_ne!(parsed, Harness::Shell, "{stored:?} was read as Shell");
            assert_eq!(harness_name(&parsed), stored, "{stored:?} lost its raw id");
        }
    }

    /// Base-branch facts describe the workspace root; only a direct chat —
    /// which has no workspace row — falls back to the session's own cwd.
    /// The cwd-first order is `repository_path_for_session`'s job, and the
    /// two orders disagreeing is what let a drift fact measured at one
    /// directory fail its action in another (issue #306).
    #[test]
    fn base_branch_path_prefers_the_workspace_root_and_falls_back_to_cwd() {
        let dir = tempfile::tempdir().unwrap();
        let db = open(&dir.path().join("bridge.db")).unwrap();
        let workspace_root = dir.path().join("workspace-root");
        std::fs::create_dir(&workspace_root).unwrap();
        db.execute_batch(
            "INSERT INTO projects(id,name,path,created_at) VALUES('p','Demo','/tmp/demo','now');
             INSERT INTO workspaces(id,project_id,city,title,branch,path,status,created_at)
                 VALUES('w','p','Kyoto','Task','bridge/task',NULL,'idle','now');
             INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source)
                 VALUES('workspace-session','w','codex','Codex','idle','reported');
             INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,cwd)
                 VALUES('direct-chat',NULL,'codex','Codex','idle','reported','/direct/chat');",
        )
        .unwrap();
        db.execute(
            "UPDATE workspaces SET path=?1 WHERE id='w'",
            params![workspace_root.to_string_lossy()],
        )
        .unwrap();

        assert_eq!(
            base_branch_path_for_session(&db, "workspace-session").unwrap(),
            Some(workspace_root),
            "a workspace session measures its workspace root, wherever the session runs"
        );
        assert_eq!(
            base_branch_path_for_session(&db, "direct-chat").unwrap(),
            Some(PathBuf::from("/direct/chat")),
            "a direct chat has no workspace row, so its cwd is all there is"
        );
        assert_eq!(
            base_branch_path_for_session(&db, "missing").unwrap(),
            None,
            "an unknown session has no repository at all"
        );
    }

    #[test]
    fn a_session_of_an_uninstalled_harness_still_loads_with_its_history() {
        // The acceptance criterion in miniature: uninstalling an agent must
        // not make its sessions vanish or be re-attributed to another harness.
        let dir = tempfile::tempdir().unwrap();
        let db = open(&dir.path().join("bridge.db")).unwrap();
        seed_workspace(&db);
        db.execute(
            "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source) \
             VALUES('s-acp','w','gemini','Gemini','ready','estimated')",
            [],
        )
        .unwrap();
        append_session_entry(
            &db,
            "s-acp",
            None,
            "assistant.message",
            &json!({"text":"history that must survive"}),
            None,
            "eligible",
            Some(1),
        )
        .unwrap();

        let state = state(&db).unwrap();
        let session = state
            .sessions
            .iter()
            .find(|session| session.id == "s-acp")
            .expect("the session still loads");
        assert_eq!(session.harness, Harness::from_stored("gemini"));
        assert_eq!(
            serde_json::to_value(&session.harness).unwrap(),
            json!("gemini"),
            "it renders under its own id"
        );

        let events = session_events_after(&db, "s-acp", 0, 10).unwrap();
        assert_eq!(events.len(), 1, "replay still returns its history");
        assert_eq!(events[0].text.as_deref(), Some("history that must survive"));
    }

    fn seed_workspace(db: &Connection) {
        db.execute(
            "INSERT INTO projects(id,name,path,created_at) VALUES('p','Demo','/tmp/demo','now')",
            [],
        )
        .unwrap();
        db.execute("INSERT INTO workspaces(id,project_id,city,title,branch,path,status,created_at) VALUES('w','p','Kyoto','Task','bridge/task','/tmp/w','idle','now')", []).unwrap();
        db.execute("INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,provider_session_id) VALUES('s','w','codex','Codex','working','reported','native-s')", []).unwrap();
    }

    fn create_legacy_fixture(path: &Path) {
        let db = Connection::open(path).unwrap();
        db.execute_batch(
            "PRAGMA journal_mode=WAL;
            CREATE TABLE projects (id TEXT PRIMARY KEY, name TEXT NOT NULL, path TEXT NOT NULL UNIQUE, created_at TEXT NOT NULL);
            CREATE TABLE workspaces (id TEXT PRIMARY KEY, project_id TEXT NOT NULL REFERENCES projects(id), city TEXT NOT NULL, title TEXT NOT NULL, branch TEXT NOT NULL, path TEXT NOT NULL UNIQUE, status TEXT NOT NULL, dirty_files INTEGER NOT NULL DEFAULT 0, additions INTEGER NOT NULL DEFAULT 0, deletions INTEGER NOT NULL DEFAULT 0, created_at TEXT NOT NULL);
            CREATE TABLE sessions (id TEXT PRIMARY KEY, workspace_id TEXT NOT NULL REFERENCES workspaces(id), harness TEXT NOT NULL, label TEXT NOT NULL, status TEXT NOT NULL, started_at TEXT, ended_at TEXT, context_percent INTEGER, usage_percent INTEGER, metric_source TEXT NOT NULL DEFAULT 'estimated');
            CREATE TABLE events (id INTEGER PRIMARY KEY AUTOINCREMENT, source TEXT NOT NULL, kind TEXT NOT NULL, entity_id TEXT NOT NULL, body TEXT NOT NULL, created_at TEXT NOT NULL);
            CREATE TABLE agent_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL REFERENCES sessions(id),
                sequence INTEGER NOT NULL,
                protocol_version INTEGER NOT NULL DEFAULT 1,
                kind TEXT NOT NULL,
                item_id TEXT,
                role TEXT,
                status TEXT,
                title TEXT,
                text TEXT,
                data TEXT NOT NULL DEFAULT '{}',
                provider_meta TEXT NOT NULL DEFAULT '{}',
                created_at TEXT NOT NULL,
                UNIQUE(session_id, sequence)
            );
            CREATE INDEX idx_agent_events_session ON agent_events(session_id, sequence);
            ALTER TABLE sessions ADD COLUMN provider_session_id TEXT;
            ALTER TABLE sessions ADD COLUMN active_turn_id TEXT;
            ALTER TABLE sessions ADD COLUMN model TEXT;
            ALTER TABLE sessions ADD COLUMN effort TEXT;
            ALTER TABLE sessions ADD COLUMN parent_session_id TEXT;
            ALTER TABLE sessions ADD COLUMN depth INTEGER;",
        )
        .unwrap();
        seed_workspace(&db);
        db.execute(
            "INSERT INTO agent_events(session_id,sequence,kind,item_id,role,status,title,text,data,provider_meta,created_at)
             VALUES('s',1,'assistant.message','m1','assistant','inProgress','First','hello','{\"delta\":\"hello\"}','{\"provider\":\"codex\",\"rawId\":1}','t1')",
            [],
        )
        .unwrap();
        db.execute(
            "INSERT INTO agent_events(session_id,sequence,kind,item_id,role,status,title,text,data,provider_meta,created_at)
             VALUES('s',2,'assistant.message','m2','assistant','completed','Second','world','{\"delta\":\"world\"}','{\"provider\":\"codex\",\"rawId\":2}','t2')",
            [],
        )
        .unwrap();
    }

    fn schema_signature(db: &Connection) -> Vec<String> {
        let mut objects = {
            let mut statement = db
                .prepare(
                    "SELECT type,name,tbl_name,CASE WHEN type='index' THEN COALESCE(sql,'') ELSE '' END FROM sqlite_master
                     WHERE name NOT LIKE 'sqlite_%' ORDER BY type,name",
                )
                .unwrap();
            let rows = statement
                .query_map([], |row| {
                    Ok(format!(
                        "{}:{}:{}:{}",
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?
                    ))
                })
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            rows
        };
        let tables = [
            "schema_version",
            "projects",
            "workspaces",
            "sessions",
            "events",
            "agent_events",
            "session_entries",
            "session_heads",
            "memory_records",
            "prompt_section_revisions",
            "worker_leases",
            "worker_runtime",
            "delegation_receipts",
            "worker_queue",
            "usage_ledger",
            "routing_evaluations",
            "routing_evaluation_runs",
            "routing_evaluation_settings",
            "learning_trigger_events",
        ];
        for table in tables {
            let mut statement = db.prepare(&format!("PRAGMA table_info({table})")).unwrap();
            let columns = statement
                .query_map([], |row| {
                    Ok(format!(
                        "{}:{}:{}:{}:{:?}:{}",
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, i64>(5)?
                    ))
                })
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            objects.push(format!("columns:{table}:{}", columns.join("|")));
        }
        objects
    }

    fn migration_versions(db: &Connection) -> Vec<i64> {
        let mut statement = db
            .prepare("SELECT version FROM schema_version ORDER BY version")
            .unwrap();
        statement
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    }

    fn backup_paths(dir: &Path) -> Vec<PathBuf> {
        std::fs::read_dir(dir)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("bridge.db.backup-"))
            })
            .collect()
    }

    #[test]
    fn persists_and_replays_ordered_events() {
        let dir = tempfile::tempdir().unwrap();
        let db = open(&dir.path().join("bridge.db")).unwrap();
        db.execute(
            "INSERT INTO projects(id,name,path,created_at) VALUES('p','Demo','/tmp/demo','now')",
            [],
        )
        .unwrap();
        event(&db, "git", "first", "p", "one").unwrap();
        event(&db, "git", "second", "p", "two").unwrap();
        let snapshot = state(&db).unwrap();
        assert_eq!(snapshot.projects.len(), 1);
        assert_eq!(snapshot.events[0].kind, "second");
        assert_eq!(snapshot.events[1].kind, "first");
    }

    #[test]
    fn telemetry_batch_uses_an_independent_writer_and_failure_cannot_rollback_history() {
        let dir = tempfile::tempdir().unwrap();
        let primary = open(&dir.path().join("bridge.db")).unwrap();
        let telemetry = open_telemetry(&dir.path().join("bridge-telemetry.db")).unwrap();
        seed_workspace(&primary);
        let event = crate::agent::NormalizedEvent {
            kind: "message.completed".into(),
            item_id: Some("m1".into()),
            role: Some("assistant".into()),
            status: Some("completed".into()),
            title: None,
            text: Some("durable".into()),
            data: json!({}),
        };
        let committed = session_event(&primary, "s", &event, &json!({"adapter":"codex"})).unwrap();
        let write_lock = primary.unchecked_transaction().unwrap();
        write_lock
            .execute("UPDATE sessions SET label='locked' WHERE id='s'", [])
            .unwrap();
        let span = telemetry_span("trace", "s", "codex", &event, &committed.created_at);
        assert_eq!(
            append_telemetry_batch(&telemetry, std::slice::from_ref(&span)).unwrap(),
            1
        );
        write_lock.rollback().unwrap();
        assert_eq!(
            telemetry
                .query_row("SELECT COUNT(*) FROM telemetry_spans", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            primary
                .query_row("SELECT COUNT(*) FROM session_entries", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            primary
                .query_row("SELECT COUNT(*) FROM telemetry_spans", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );

        telemetry.execute("DROP TABLE telemetry_spans", []).unwrap();
        assert!(append_telemetry_batch(&telemetry, &[span]).is_err());
        assert_eq!(
            primary
                .query_row("SELECT COUNT(*) FROM session_entries", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
    }

    #[test]
    fn history_snapshot_is_consistent_and_checksum_detects_corruption() {
        let dir = tempfile::tempdir().unwrap();
        let primary_path = dir.path().join("bridge.db");
        let primary = open(&primary_path).unwrap();
        event(&primary, "test", "history.saved", "entity", "durable").unwrap();
        let readonly = Connection::open_with_flags(
            &primary_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .unwrap();
        let (snapshot, manifest) =
            export_history_snapshot(&readonly, &dir.path().join("snapshots")).unwrap();
        assert!(verify_history_snapshot(&snapshot, &manifest).unwrap());
        assert!(
            std::fs::read_dir(dir.path().join("snapshots"))
                .unwrap()
                .flatten()
                .all(|entry| !entry.file_name().to_string_lossy().ends_with(".tmp")),
            "a successful publication must not leave a partial snapshot behind"
        );
        let snapshot_db = Connection::open(&snapshot).unwrap();
        assert_eq!(
            snapshot_db
                .query_row(
                    "SELECT body FROM events WHERE kind='history.saved'",
                    [],
                    |row| row.get::<_, String>(0)
                )
                .unwrap(),
            "durable"
        );
        drop(snapshot_db);

        let mut bytes = std::fs::read(&snapshot).unwrap();
        bytes[0] ^= 0xff;
        std::fs::write(&snapshot, bytes).unwrap();
        assert!(!verify_history_snapshot(&snapshot, &manifest).unwrap());
    }

    #[test]
    fn opening_clears_active_turns_on_every_non_live_session() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bridge.db");
        {
            let db = open(&path).unwrap();
            for (id, status, turn) in [
                ("interrupted", "working", "turn-a"),
                ("crash-stopped", "stopped", "turn-b"),
                ("idle", "ready", "turn-c"),
            ] {
                db.execute(
                    "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,active_turn_id) VALUES(?1,NULL,'claude','Session',?2,'reported',?3)",
                    params![id, status, turn],
                )
                .unwrap();
            }
        }
        let db = open(&path).unwrap();
        let stale: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM sessions WHERE active_turn_id IS NOT NULL",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            stale, 0,
            "no non-live session may keep an active turn: the composer renders Stop/Steer from it"
        );
        let interrupted: String = db
            .query_row(
                "SELECT status FROM sessions WHERE id='interrupted'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(interrupted, "stopped");
    }

    #[test]
    fn streamed_hash_matches_whole_file_digest() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("blob");
        // Larger than the reader's buffer so chunking is actually exercised.
        let payload: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
        std::fs::write(&path, &payload).unwrap();
        assert_eq!(
            hash_file_streaming(&path).unwrap(),
            format!("{:x}", Sha256::digest(&payload)),
            "streaming and whole-file hashing must agree for manifest compatibility"
        );
    }

    fn fabricate_snapshot_pair(dir: &Path, stamp: &str) {
        let database_file = format!("bridge-history-{stamp}.sqlite");
        std::fs::write(dir.join(&database_file), b"snapshot-bytes").unwrap();
        let manifest = HistorySnapshotManifest {
            schema_version: 1,
            database_file,
            sha256: "unchecked-by-retention".into(),
            created_at: Utc::now().to_rfc3339(),
        };
        std::fs::write(
            dir.join(format!("bridge-history-{stamp}.manifest.json")),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
    }

    fn fabricate_sized_snapshot_pair(dir: &Path, stamp: &str, database_bytes: usize) {
        fabricate_snapshot_pair(dir, stamp);
        std::fs::write(
            dir.join(format!("bridge-history-{stamp}.sqlite")),
            vec![0_u8; database_bytes],
        )
        .unwrap();
    }

    /// Move a fixture's mtime two hours into the past so grace-period tests
    /// never depend on `now()` landing after the write within the same tick.
    fn backdate(path: &Path) {
        let two_hours_ago =
            std::time::SystemTime::now() - std::time::Duration::from_secs(2 * 60 * 60);
        std::fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(two_hours_ago)
            .unwrap();
    }

    fn database_names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .flatten()
            .filter_map(|entry| entry.file_name().into_string().ok())
            .filter(|name| name.ends_with(".sqlite"))
            .collect();
        names.sort();
        names.reverse();
        names
    }

    #[test]
    fn retention_keeps_recent_pairs_plus_a_daily_ladder_of_further_days() {
        let dir = tempfile::tempdir().unwrap();
        // Three snapshots on the newest day, two on the day before, one each
        // on two older days.
        for stamp in [
            "20260820T120000000000000Z-f1",
            "20260820T110000000000000Z-f2",
            "20260820T100000000000000Z-f3",
            "20260819T120000000000000Z-f4",
            "20260819T110000000000000Z-f5",
            "20260818T120000000000000Z-f6",
            "20260817T120000000000000Z-f7",
        ] {
            fabricate_snapshot_pair(dir.path(), stamp);
        }
        let retention = HistorySnapshotRetention {
            keep_recent: 2,
            keep_daily_days: 3,
            ..HistorySnapshotRetention::default()
        };
        let outcome = prune_history_snapshots(dir.path(), retention).unwrap();
        // The recent window covers the newest day; the ladder then keeps the
        // newest snapshot of each of the three days beyond it. Days already
        // represented by the recent window do not consume ladder slots, which
        // is what keeps the ladder reachable for any `keep_recent`.
        assert_eq!(
            database_names(dir.path()),
            [
                "bridge-history-20260820T120000000000000Z-f1.sqlite",
                "bridge-history-20260820T110000000000000Z-f2.sqlite",
                "bridge-history-20260819T120000000000000Z-f4.sqlite",
                "bridge-history-20260818T120000000000000Z-f6.sqlite",
                "bridge-history-20260817T120000000000000Z-f7.sqlite",
            ]
        );
        assert_eq!(outcome.removed_pairs, 2);
        assert_eq!(outcome.retained_pairs, 5);
        assert!(outcome.removed_bytes > 0);
        assert_eq!(outcome.over_budget_bytes, 0);
        // Deterministic: pruning again removes nothing.
        let second = prune_history_snapshots(dir.path(), retention).unwrap();
        assert_eq!(second.removed_pairs, 0);
        assert_eq!(second.removed_incomplete_files, 0);
        assert_eq!(second.retained_pairs, 5);
        assert!(second.is_quiet());
    }

    #[test]
    fn default_retention_keeps_a_daily_ladder_under_the_budget() {
        let dir = tempfile::tempdir().unwrap();
        // Four snapshots a day for three days, all far below the byte budget:
        // the daily rule must be reachable with the shipped defaults.
        for day in ["20260820", "20260819", "20260818"] {
            for hour in ["12", "11", "10", "09"] {
                fabricate_snapshot_pair(dir.path(), &format!("{day}T{hour}0000000000000Z-x"));
            }
        }
        let outcome =
            prune_history_snapshots(dir.path(), HistorySnapshotRetention::default()).unwrap();
        assert_eq!(
            database_names(dir.path()),
            [
                "bridge-history-20260820T120000000000000Z-x.sqlite",
                "bridge-history-20260820T110000000000000Z-x.sqlite",
                "bridge-history-20260820T100000000000000Z-x.sqlite",
                "bridge-history-20260820T090000000000000Z-x.sqlite",
                "bridge-history-20260819T120000000000000Z-x.sqlite",
                "bridge-history-20260818T120000000000000Z-x.sqlite",
            ],
            "four recent plus the newest of each earlier day"
        );
        assert_eq!(outcome.retained_pairs, 6);
        assert_eq!(outcome.removed_pairs, 6);
    }

    #[test]
    fn retention_budget_stops_at_the_ceiling_instead_of_packing_older_pairs() {
        let dir = tempfile::tempdir().unwrap();
        fabricate_sized_snapshot_pair(dir.path(), "20260820T120000000000000Z-a", 500);
        fabricate_sized_snapshot_pair(dir.path(), "20260820T110000000000000Z-b", 1900);
        fabricate_sized_snapshot_pair(dir.path(), "20260819T120000000000000Z-c", 400);
        fabricate_sized_snapshot_pair(dir.path(), "20260818T120000000000000Z-d", 400);
        let manifest_bytes = std::fs::metadata(
            dir.path()
                .join("bridge-history-20260820T120000000000000Z-a.manifest.json"),
        )
        .unwrap()
        .len();
        let outcome = prune_history_snapshots(
            dir.path(),
            HistorySnapshotRetention {
                keep_recent: 2,
                keep_daily_days: 3,
                max_total_bytes: 2000 + 2 * manifest_bytes,
                ..HistorySnapshotRetention::default()
            },
        )
        .unwrap();
        // `b` does not fit after `a`; `c` and `d` would, but retaining older
        // pairs after dropping a newer one leaves a hole in the middle of the
        // recovery window, so the ceiling ends retention outright.
        assert_eq!(
            database_names(dir.path()),
            ["bridge-history-20260820T120000000000000Z-a.sqlite"]
        );
        assert_eq!(outcome.retained_pairs, 1);
        assert_eq!(outcome.removed_pairs, 3);
        assert_eq!(outcome.over_budget_bytes, 0);
    }

    #[test]
    fn retention_enforces_a_total_snapshot_byte_budget() {
        let dir = tempfile::tempdir().unwrap();
        for stamp in [
            "20260820T120000000000000Z-f1",
            "20260820T110000000000000Z-f2",
            "20260820T100000000000000Z-f3",
        ] {
            fabricate_snapshot_pair(dir.path(), stamp);
        }
        let newest_pair_bytes = [
            "bridge-history-20260820T120000000000000Z-f1.sqlite",
            "bridge-history-20260820T120000000000000Z-f1.manifest.json",
        ]
        .iter()
        .map(|name| std::fs::metadata(dir.path().join(name)).unwrap().len())
        .sum();
        let outcome = prune_history_snapshots(
            dir.path(),
            HistorySnapshotRetention {
                keep_recent: 3,
                keep_daily_days: 1,
                max_total_bytes: newest_pair_bytes,
                ..HistorySnapshotRetention::default()
            },
        )
        .unwrap();
        assert_eq!(outcome.removed_pairs, 2);
        assert_eq!(outcome.retained_pairs, 1);
        assert_eq!(outcome.retained_bytes, newest_pair_bytes);
        assert_eq!(outcome.over_budget_bytes, 0);
        assert_eq!(
            database_names(dir.path()),
            ["bridge-history-20260820T120000000000000Z-f1.sqlite"]
        );
    }

    #[test]
    fn retention_reports_a_newest_pair_that_alone_exceeds_the_budget() {
        let dir = tempfile::tempdir().unwrap();
        fabricate_sized_snapshot_pair(dir.path(), "20260820T120000000000000Z-big", 3000);
        fabricate_sized_snapshot_pair(dir.path(), "20260820T110000000000000Z-old", 10);
        let outcome = prune_history_snapshots(
            dir.path(),
            HistorySnapshotRetention {
                keep_recent: 2,
                max_total_bytes: 1000,
                ..HistorySnapshotRetention::default()
            },
        )
        .unwrap();
        // The newest recovery point is never deleted to satisfy the budget,
        // but the breach is not silent: it is reported for the log line and
        // the ceiling still stops everything older.
        assert_eq!(
            database_names(dir.path()),
            ["bridge-history-20260820T120000000000000Z-big.sqlite"]
        );
        assert_eq!(outcome.retained_pairs, 1);
        assert_eq!(outcome.over_budget_bytes, outcome.retained_bytes - 1000);
        assert!(outcome.over_budget_bytes >= 2000);
        assert!(!outcome.is_quiet());
    }

    #[test]
    fn retention_reclaims_stale_incomplete_artifacts_without_touching_foreign_files() {
        let dir = tempfile::tempdir().unwrap();
        let debris = [
            "bridge-history-orphan.sqlite",
            "bridge-history-loner.manifest.json",
            ".bridge-history-torn.sqlite.tmp",
            ".bridge-history-torn.manifest.tmp",
        ];
        std::fs::write(dir.path().join(debris[0]), b"x").unwrap();
        std::fs::write(
            dir.path().join(debris[1]),
            serde_json::to_vec(&HistorySnapshotManifest {
                schema_version: 1,
                database_file: "bridge-history-gone.sqlite".into(),
                sha256: "x".into(),
                created_at: Utc::now().to_rfc3339(),
            })
            .unwrap(),
        )
        .unwrap();
        std::fs::write(dir.path().join(debris[2]), b"x").unwrap();
        std::fs::write(dir.path().join(debris[3]), b"x").unwrap();
        // Foreign files, also stale: a user's notes and another tool's
        // dotted temp file that merely shares the `.sqlite.tmp` suffix.
        let foreign = ["user-notes.txt", ".mydb.sqlite.tmp", ".backup.manifest.tmp"];
        for name in foreign {
            std::fs::write(dir.path().join(name), b"keep").unwrap();
        }
        for name in debris.iter().chain(foreign.iter()) {
            backdate(&dir.path().join(name));
        }
        let outcome =
            prune_history_snapshots(dir.path(), HistorySnapshotRetention::default()).unwrap();
        assert_eq!(outcome.removed_pairs, 0);
        assert_eq!(outcome.removed_incomplete_files, 4);
        assert!(outcome.removed_incomplete_bytes > 0);
        assert_eq!(outcome.skipped_files, 0);
        let mut remaining: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .filter_map(|entry| entry.file_name().into_string().ok())
            .collect();
        remaining.sort();
        let mut expected: Vec<String> = foreign.iter().map(|name| name.to_string()).collect();
        expected.sort();
        assert_eq!(remaining, expected);
    }

    #[test]
    fn retention_reclaim_never_touches_live_pairs() {
        let dir = tempfile::tempdir().unwrap();
        fabricate_snapshot_pair(dir.path(), "20260820T120000000000000Z-f1");
        fabricate_snapshot_pair(dir.path(), "20260820T110000000000000Z-f2");
        std::fs::write(dir.path().join("bridge-history-orphan.sqlite"), b"x").unwrap();
        for entry in std::fs::read_dir(dir.path()).unwrap().flatten() {
            backdate(&entry.path());
        }
        // Zero grace: every file is old enough, so only the classification
        // stands between the reclaim and the live snapshots.
        let outcome = prune_history_snapshots(
            dir.path(),
            HistorySnapshotRetention {
                keep_recent: 2,
                incomplete_grace: std::time::Duration::ZERO,
                ..HistorySnapshotRetention::default()
            },
        )
        .unwrap();
        assert_eq!(outcome.retained_pairs, 2);
        assert_eq!(outcome.removed_pairs, 0);
        assert_eq!(outcome.removed_incomplete_files, 1);
        assert_eq!(
            database_names(dir.path()),
            [
                "bridge-history-20260820T120000000000000Z-f1.sqlite",
                "bridge-history-20260820T110000000000000Z-f2.sqlite",
            ]
        );
        assert!(dir
            .path()
            .join("bridge-history-20260820T110000000000000Z-f2.manifest.json")
            .exists());
    }

    #[test]
    fn retention_leaves_fresh_incomplete_artifacts_for_an_active_export() {
        let dir = tempfile::tempdir().unwrap();
        let pending = dir.path().join(".bridge-history-active.sqlite.tmp");
        std::fs::write(&pending, b"still exporting").unwrap();
        let outcome = prune_history_snapshots(
            dir.path(),
            HistorySnapshotRetention {
                keep_recent: 0,
                keep_daily_days: 0,
                incomplete_grace: std::time::Duration::from_secs(60 * 60),
                ..HistorySnapshotRetention::default()
            },
        )
        .unwrap();
        assert_eq!(outcome, SnapshotPruneOutcome::default());
        assert!(pending.exists());
    }

    #[test]
    fn retention_preserves_unknown_manifests_across_a_downgrade() {
        let dir = tempfile::tempdir().unwrap();
        let newer_database = "bridge-history-newer.sqlite";
        let newer_manifest = "bridge-history-newer.manifest.json";
        std::fs::write(dir.path().join(newer_database), b"newer snapshot").unwrap();
        std::fs::write(
            dir.path().join(newer_manifest),
            serde_json::to_vec(&HistorySnapshotManifest {
                schema_version: 2,
                database_file: newer_database.into(),
                sha256: "newer-hash-format".into(),
                created_at: Utc::now().to_rfc3339(),
            })
            .unwrap(),
        )
        .unwrap();
        std::fs::write(
            dir.path().join("bridge-history-future-only.manifest.json"),
            br#"{"schema_version":2,"database_file":"future-layout.snapshot"}"#,
        )
        .unwrap();
        std::fs::write(dir.path().join("bridge-history-opaque.sqlite"), b"opaque").unwrap();
        std::fs::write(
            dir.path().join("bridge-history-opaque.manifest.json"),
            b"a future manifest encoding",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("bridge-history-opaque-only.manifest.json"),
            b"an unreadable manifest",
        )
        .unwrap();
        for entry in std::fs::read_dir(dir.path()).unwrap().flatten() {
            backdate(&entry.path());
        }

        let outcome = prune_history_snapshots(
            dir.path(),
            HistorySnapshotRetention {
                keep_recent: 0,
                keep_daily_days: 0,
                incomplete_grace: std::time::Duration::ZERO,
                ..HistorySnapshotRetention::default()
            },
        )
        .unwrap();

        // An intact database whose manifest this version cannot read is a
        // recovery point, not debris: nothing here is deleted.
        assert_eq!(outcome, SnapshotPruneOutcome::default());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 6);
    }

    #[cfg(unix)]
    #[test]
    fn retention_counts_undeletable_files_and_keeps_going() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        fabricate_snapshot_pair(dir.path(), "20260820T120000000000000Z-f1");
        fabricate_snapshot_pair(dir.path(), "20260820T110000000000000Z-f2");
        fabricate_snapshot_pair(dir.path(), "20260820T100000000000000Z-f3");
        // A read-only directory refuses every unlink, the same way a busy or
        // foreign-owned file would refuse one.
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o555)).unwrap();
        let outcome = prune_history_snapshots(
            dir.path(),
            HistorySnapshotRetention {
                keep_recent: 1,
                keep_daily_days: 0,
                ..HistorySnapshotRetention::default()
            },
        );
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
        let outcome = outcome.expect("an undeletable file is counted, not propagated");
        assert_eq!(outcome.retained_pairs, 1);
        assert_eq!(
            outcome.skipped_files, 2,
            "both stale databases were attempted"
        );
        assert_eq!(outcome.removed_pairs, 0);
        // Manifests stay with their databases, so the skipped pairs remain
        // valid snapshots rather than torn ones.
        assert_eq!(database_names(dir.path()).len(), 3);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 6);
    }

    #[test]
    fn export_publishes_only_final_names_and_prefixes_its_pending_files() {
        let dir = tempfile::tempdir().unwrap();
        let primary = open(&dir.path().join("bridge.db")).unwrap();
        let snapshots = dir.path().join("snapshots");
        let (database, manifest) = export_history_snapshot(&primary, &snapshots).unwrap();
        let names: Vec<String> = std::fs::read_dir(&snapshots)
            .unwrap()
            .flatten()
            .filter_map(|entry| entry.file_name().into_string().ok())
            .collect();
        assert_eq!(
            names.len(),
            2,
            "no pending files survive publication: {names:?}"
        );
        assert!(database.exists() && manifest.exists());
        assert!(verify_history_snapshot(&database, &manifest).unwrap());
        // The pending names retention recognises are the ones the export
        // actually writes.
        let stem = database.file_name().unwrap().to_str().unwrap();
        let stem = stem
            .strip_prefix("bridge-history-")
            .and_then(|rest| rest.strip_suffix(".sqlite"))
            .unwrap();
        for pending in [
            format!("{PENDING_PREFIX}{stem}.sqlite.tmp"),
            format!("{PENDING_PREFIX}{stem}.manifest.tmp"),
        ] {
            assert!(is_reclaimable_incomplete_snapshot_artifact(
                &snapshots.join(pending)
            ));
        }
        assert!(!is_reclaimable_incomplete_snapshot_artifact(
            &snapshots.join(".other-tool.sqlite.tmp")
        ));
    }

    #[test]
    fn boot_export_skips_while_fresh_and_exports_when_stale() {
        let dir = tempfile::tempdir().unwrap();
        let primary_path = dir.path().join("bridge.db");
        let primary = open(&primary_path).unwrap();
        let snapshots = dir.path().join("snapshots");
        let first = export_history_snapshot_if_stale(
            &primary,
            &snapshots,
            std::time::Duration::from_secs(900),
        )
        .unwrap();
        assert!(first.is_some(), "an empty directory exports");
        let (count, _) = history_snapshot_stats(&snapshots);
        let skipped = export_history_snapshot_if_stale(
            &primary,
            &snapshots,
            std::time::Duration::from_secs(900),
        )
        .unwrap();
        assert!(skipped.is_none(), "a fresh snapshot suppresses the boot export");
        assert_eq!(history_snapshot_stats(&snapshots).0, count);
        let again = export_history_snapshot_if_stale(
            &primary,
            &snapshots,
            std::time::Duration::ZERO,
        )
        .unwrap();
        assert!(again.is_some(), "a stale snapshot exports again");
    }

    #[test]
    fn migrates_current_schema_fixture_idempotently_and_creates_backup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bridge.db");
        create_legacy_fixture(&path);
        let db = open(&path).unwrap();
        assert_eq!(
            migration_versions(&db),
            (1..=LATEST_SCHEMA_VERSION).collect::<Vec<_>>(),
            "a legacy fixture must land on the current schema"
        );
        for table in [
            "model_profiles",
            "routing_policies",
            "routing_evaluations",
            "learning_jobs",
            "learning_triggers",
            "learning_job_runs",
            "learning_trigger_events",
            "routing_policy_promotions",
            "prompt_compilations",
            "learning_scope_cursors",
            "session_entry_fts",
            "memory_records",
            "memory_record_fts",
            "memory_retrieval_audits",
            "memory_injection_settings",
            "routing_catalogs",
            "learning_tunables",
            "prompt_section_revisions",
            "routing_evaluation_runs",
            "routing_evaluation_settings",
            "memory_consolidation_runs",
            "memory_consolidation_settings",
        ] {
            assert!(
                db.query_row(
                    "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
                    params![table],
                    |row| row.get::<_, bool>(0),
                )
                .unwrap(),
                "missing migration-15 table {table}"
            );
        }
        for (table, column) in [
            ("usage_ledger", "cost_microusd"),
            ("usage_ledger", "uncached_input_tokens"),
            ("usage_ledger", "stable_prefix_hash"),
            ("model_profiles", "profile_id"),
            ("router_decisions", "policy_version"),
            ("router_decisions", "trace_id"),
            ("router_decisions", "catalog_snapshot"),
            ("router_outcomes", "success_state"),
            ("router_outcomes", "confidence_bps"),
            ("learning_job_runs", "lease_expires_at"),
            ("learning_jobs", "last_evidence_boundary"),
            ("learning_jobs", "run_budget_tokens"),
            ("routing_policies", "learning_scope"),
            ("learning_job_runs", "learning_scope"),
            ("memory_records", "valid_from"),
            ("memory_records", "valid_to"),
            ("memory_records", "expires_at"),
            ("memory_records", "conflict_group"),
        ] {
            let exists = db
                .prepare(&format!("PRAGMA table_info({table})"))
                .unwrap()
                .query_map([], |row| row.get::<_, String>(1))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
                .iter()
                .any(|name| name == column);
            assert!(exists, "missing migration-15 column {table}.{column}");
        }
        {
            let transaction = db.unchecked_transaction().unwrap();
            migration_15_role_profiles_and_learning_jobs(&transaction).unwrap();
            transaction.commit().unwrap();
        }
        assert_eq!(
            db.query_row(
                "SELECT COUNT(*) FROM routing_policies WHERE status='active'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
        assert_eq!(db.query_row("SELECT COUNT(*) FROM learning_triggers WHERE kind IN ('manual','in_app') AND registration_id='built-in'", [], |row| row.get::<_, i64>(0)).unwrap(), 2);
        // Migration 22 adds the backend binding without inventing one. A row
        // that predates it stays readable and reads as unbound — a guessed
        // backend would be fabricated provenance for a session that never had
        // any, and the resume path treats null as "not recorded", not as
        // "changed".
        let (backend, version, installation): (
            Option<String>,
            Option<String>,
            Option<String>,
        ) = db
            .query_row(
                "SELECT backend_id,backend_version,backend_installation_id FROM sessions WHERE id='s'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!((backend, version, installation), (None, None, None));
        assert!(db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='backend_change_authorizations')",
                [],
                |row| row.get::<_, bool>(0),
            )
            .unwrap());
        {
            let transaction = db.unchecked_transaction().unwrap();
            migration_22_session_backend_binding(&transaction).unwrap();
            transaction.commit().unwrap();
        }

        // Legacy agent_events were backfilled into the immutable forest.
        assert_eq!(session_entries(&db, "s").unwrap().len(), 2);
        drop(db);
        let backups = backup_paths(dir.path());
        assert_eq!(backups.len(), 1);
        let backup = Connection::open(&backups[0]).unwrap();
        assert_eq!(
            backup
                .query_row("SELECT COUNT(*) FROM agent_events", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            2
        );
        drop(backup);
        let db = open(&path).unwrap();
        assert_eq!(
            migration_versions(&db),
            (1..=LATEST_SCHEMA_VERSION).collect::<Vec<_>>(),
            "a legacy fixture must land on the current schema"
        );
        assert_eq!(backup_paths(dir.path()).len(), 1);
    }

    #[test]
    fn migration_45_compacts_historical_bodies_and_keeps_old_writers_compatible() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bridge.db");
        let db = open(&path).unwrap();
        db.execute_batch(
            "DROP TRIGGER memory_retrieval_audits_compact_previous;
             DROP TRIGGER memory_retrieval_audits_delete_with_session;
             DELETE FROM schema_version WHERE version>=45;
             INSERT INTO sessions(id,workspace_id,harness,label,status,started_at,metric_source,kind)
             VALUES('session-1',NULL,'codex','Chat','ready','now','estimated','direct');
             INSERT INTO memory_retrieval_audits(
                 id,scope_key,recipient_session_id,objective_hash,candidate_count,
                 selected_ids,exclusions,token_estimate,created_at
             ) VALUES
                 ('a-old','account:local','session-1','old',1,'[{\"id\":\"old\",\"body\":\"old body\"}]','[]',1,'2026-01-01T00:00:00Z'),
                 ('z-new','account:local','session-1','new',1,'[{\"id\":\"new\",\"body\":\"new body\"}]','[]',1,'2026-01-01T00:00:00Z'),
                 ('orphan','account:local','deleted-session','orphan',1,'[{\"id\":\"orphan\",\"body\":\"orphan body\"}]','[]',1,'2026-01-02T00:00:00Z');",
        )
        .unwrap();
        drop(db);

        let db = open(&path).unwrap();
        let rows: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM memory_retrieval_audits WHERE recipient_session_id='session-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let historical: String = db
            .query_row(
                "SELECT selected_ids FROM memory_retrieval_audits WHERE id='a-old'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let newest: String = db
            .query_row(
                "SELECT selected_ids FROM memory_retrieval_audits WHERE id='z-new'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(rows, 2, "the delivery history remains append-only");
        assert_eq!(historical, "[\"old\"]", "equal timestamps use id as the tie-break");
        assert!(newest.contains("new body"), "the newest frozen packet survives intact");
        let orphan_rows: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM memory_retrieval_audits WHERE id='orphan'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(orphan_rows, 0, "upgrade purges audits for sessions already deleted");

        db.execute(
            "INSERT INTO memory_retrieval_audits(
                 id,scope_key,recipient_session_id,objective_hash,candidate_count,
                 selected_ids,exclusions,token_estimate,created_at
             ) VALUES('third','account:local','session-1','third',1,
                 '[{\"id\":\"third\",\"body\":\"third body\"}]','[]',1,'2026-01-03T00:00:00Z')",
            [],
        )
        .unwrap();
        let rows_after_old_writer: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM memory_retrieval_audits WHERE recipient_session_id='session-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let prior_body_copies: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM memory_retrieval_audits
                 WHERE recipient_session_id='session-1' AND selected_ids LIKE '%body%' AND id != 'third'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(rows_after_old_writer, 3, "a v44-style plain insert remains valid");
        assert_eq!(prior_body_copies, 0, "the trigger compacts prior full bodies");

        db.execute(
            "INSERT INTO memory_retrieval_audits(
                 id,scope_key,recipient_session_id,objective_hash,candidate_count,
                 selected_ids,exclusions,token_estimate,created_at
             ) VALUES('backdated','account:local','session-1','backdated',1,
                 '[{\"id\":\"backdated\",\"body\":\"backdated body\"}]','[]',1,'2025-01-01T00:00:00Z')",
            [],
        )
        .unwrap();
        let full_body_rows: Vec<String> = db
            .prepare(
                "SELECT id FROM memory_retrieval_audits
                 WHERE recipient_session_id='session-1' AND selected_ids LIKE '%body%'
                 ORDER BY created_at DESC, id DESC",
            )
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(
            full_body_rows,
            vec!["third"],
            "an out-of-order insert is compacted instead of displacing the newest payload"
        );
    }

    #[test]
    fn migration_49_preserves_existing_repair_spending_across_reopens() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bridge.db");
        let db = open(&path).unwrap();
        db.execute_batch(
            "INSERT INTO sessions(id,harness,label,status,depth) VALUES('parent','codex','Parent','ready',0);
             INSERT INTO sessions(id,harness,label,status,depth,parent_session_id)
                 VALUES('spent','codex','Spent','ready',1,'parent'),('fresh','codex','Fresh','ready',1,'parent');
             INSERT INTO worker_runtime(session_id,parent_session_id,lifecycle_state,task_family,compatibility_key,updated_at)
                 VALUES('spent','parent','working','implementation','key','now'),('fresh','parent','working','implementation','key','now');
             ALTER TABLE worker_runtime DROP COLUMN result_repair_count;
             DELETE FROM schema_version WHERE version>=49;"
        ).unwrap();
        event(
            &db,
            "delegation",
            "worker.result.repair_requested",
            "spent",
            "missing fence",
        )
        .unwrap();
        drop(db);
        for _ in 0..2 {
            let db = open(&path).unwrap();
            for (id, expected) in [("spent", 1), ("fresh", 0)] {
                let count: i64 = db
                    .query_row(
                        "SELECT result_repair_count FROM worker_runtime WHERE session_id=?1",
                        params![id],
                        |r| r.get(0),
                    )
                    .unwrap();
                assert_eq!(count, expected);
            }
        }
    }

    #[test]
    fn migration_47_maps_legacy_profile_pins_to_selection_modes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bridge.db");
        let db = open(&path).unwrap();
        db.execute_batch(
            "INSERT INTO model_profiles(
                 version,profile_id,purpose,canonical_role,provider,model,effort,
                 pinned,selection_mode,learning_enabled,created_at
             ) VALUES
                 (1,'planner','planner','planning','codex','strong','high',1,'track_standard',0,'now'),
                 (1,'research','research','research','codex','standard','medium',0,'pinned',1,'now');
             DELETE FROM schema_version WHERE version>=47;",
        )
        .unwrap();
        drop(db);

        let db = open(&path).unwrap();
        let modes = db
            .prepare("SELECT purpose,selection_mode FROM model_profiles ORDER BY purpose")
            .unwrap()
            .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(
            modes,
            vec![
                ("planner".into(), "pinned".into()),
                ("research".into(), "track_standard".into()),
            ]
        );
    }

    #[test]
    fn deleting_a_session_purges_its_memory_packet_audits() {
        let dir = tempfile::tempdir().unwrap();
        let db = open(&dir.path().join("bridge.db")).unwrap();
        db.execute(
            "INSERT INTO sessions(id,workspace_id,harness,label,status,started_at,metric_source,kind)
             VALUES('session-1',NULL,'codex','Chat','ready','now','estimated','direct')",
            [],
        )
        .unwrap();
        db.execute(
            "INSERT INTO memory_retrieval_audits(
                 id,scope_key,recipient_session_id,objective_hash,candidate_count,
                 selected_ids,exclusions,token_estimate,created_at
             ) VALUES('audit','account:local','session-1','objective',0,'[]','[]',0,'now')",
            [],
        )
        .unwrap();

        db.execute("DELETE FROM sessions WHERE id='session-1'", [])
            .unwrap();

        let rows: i64 = db
            .query_row("SELECT COUNT(*) FROM memory_retrieval_audits", [], |row| row.get(0))
            .unwrap();
        assert_eq!(rows, 0);
    }

    #[test]
    fn migration_backup_retention_keeps_only_the_newest_bridge_backup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bridge.db");
        drop(open(&path).unwrap());
        let old = dir
            .path()
            .join("bridge.db.backup-20260101T000000000000000Z");
        let newest = dir
            .path()
            .join("bridge.db.backup-20260102T000000000000000Z");
        std::fs::copy(&path, &old).unwrap();
        std::fs::copy(&path, &newest).unwrap();

        drop(open(&path).unwrap());

        assert!(!old.exists(), "superseded full database copies are reclaimed");
        assert!(newest.exists(), "the newest rollback point survives");
    }

    #[test]
    fn migration_backup_retention_prefers_the_just_created_rollback() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bridge.db");
        drop(open(&path).unwrap());
        let current = dir
            .path()
            .join("bridge.db.backup-20260101T000000000000000Z");
        let clock_skewed = dir
            .path()
            .join("bridge.db.backup-20990101T000000000000000Z");
        std::fs::copy(&path, &current).unwrap();
        std::fs::copy(&path, &clock_skewed).unwrap();

        let outcome = prune_migration_backups(&path, Some(&current)).unwrap();

        assert_eq!(outcome.removed_files, 1);
        assert!(current.exists(), "the rollback from this migration is protected by identity");
        assert!(!clock_skewed.exists(), "a future-dated older backup cannot displace it");
    }

    #[test]
    fn migration_backup_retention_rejects_a_truncated_newer_backup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bridge.db");
        drop(open(&path).unwrap());
        let recoverable = dir
            .path()
            .join("bridge.db.backup-20260101T000000000000000Z");
        let truncated = dir
            .path()
            .join("bridge.db.backup-20260102T000000000000000Z");
        std::fs::copy(&path, &recoverable).unwrap();
        std::fs::write(&truncated, b"SQLite format 3\0").unwrap();

        let outcome = prune_migration_backups(&path, None).unwrap();

        assert_eq!(outcome.removed_files, 1);
        assert!(recoverable.exists(), "the structurally readable rollback survives");
        assert!(!truncated.exists(), "a header-only newer file cannot displace it");
    }

    #[test]
    fn migration_backup_retention_preserves_unrelated_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bridge.db");
        drop(open(&path).unwrap());
        let foreign = dir.path().join("notes.txt");
        let malformed = dir.path().join("bridge.db.backup-not-a-timestamp");
        let other_database = dir
            .path()
            .join("other.db.backup-20260101T000000000000000Z");
        let interrupted = dir
            .path()
            .join("bridge.db.backup-20260101T000000000000000Z.pending");
        let malformed_pending = dir.path().join("bridge.db.backup-not-a-timestamp.pending");
        std::fs::write(&foreign, b"keep").unwrap();
        std::fs::write(&malformed, b"keep").unwrap();
        std::fs::write(&other_database, b"keep").unwrap();
        std::fs::write(&interrupted, b"partial database copy").unwrap();
        std::fs::write(&malformed_pending, b"keep").unwrap();

        drop(open(&path).unwrap());

        assert!(path.exists(), "the live database is never a retention candidate");
        assert!(foreign.exists());
        assert!(malformed.exists());
        assert!(other_database.exists());
        assert!(!interrupted.exists(), "crash-leftover backup staging files are reclaimed");
        assert!(malformed_pending.exists(), "only exact Bridge staging names are reclaimed");
    }

    #[test]
    fn upgraded_and_current_databases_have_identical_cache_telemetry_columns() {
        fn columns(db: &Connection, table: &str) -> Vec<String> {
            let mut values = db
                .prepare(&format!("PRAGMA table_info({table})"))
                .unwrap()
                .query_map([], |row| row.get::<_, String>(1))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            values.sort();
            values
        }

        let current = open(Path::new(":memory:")).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let legacy_path = dir.path().join("legacy.db");
        create_legacy_fixture(&legacy_path);
        let upgraded = open(&legacy_path).unwrap();
        assert_eq!(
            columns(&current, "usage_ledger"),
            columns(&upgraded, "usage_ledger")
        );
        assert_eq!(
            columns(&current, "prompt_compilations"),
            columns(&upgraded, "prompt_compilations")
        );
        for required in [
            "uncached_input_tokens",
            "stable_prefix_id",
            "stable_prefix_hash",
            "prompt_schema_version",
            "prefix_token_estimate",
            "cross_harness_reuse",
        ] {
            assert!(columns(&upgraded, "usage_ledger")
                .iter()
                .any(|column| column == required));
        }
        for required in [
            "sections_json",
            "stable_bytes",
            "variable_bytes",
            "stable_token_estimate",
            "variable_token_estimate",
            "token_estimate_source",
        ] {
            assert!(columns(&upgraded, "prompt_compilations")
                .iter()
                .any(|column| column == required));
        }
    }

    #[test]
    fn migration_16_repairs_databases_created_by_early_migration_15() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bridge.db");
        let db = open(&path).unwrap();
        db.execute_batch(
            "DROP INDEX idx_model_profiles_id;
             ALTER TABLE model_profiles DROP COLUMN profile_id;
             ALTER TABLE learning_jobs DROP COLUMN last_evidence_boundary;
             DROP TABLE configuration_entries;
             DELETE FROM schema_version WHERE version >= 16;",
        )
        .unwrap();
        drop(db);

        let db = open(&path).unwrap();
        assert_eq!(current_schema_version(&db).unwrap(), LATEST_SCHEMA_VERSION);
        for (table, column) in [
            ("model_profiles", "profile_id"),
            ("learning_jobs", "last_evidence_boundary"),
        ] {
            let exists = db
                .prepare(&format!("PRAGMA table_info({table})"))
                .unwrap()
                .query_map([], |row| row.get::<_, String>(1))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
                .iter()
                .any(|name| name == column);
            assert!(exists, "migration 16 did not restore {table}.{column}");
        }
        db.execute(
            "INSERT INTO model_profiles(version,profile_id,purpose,canonical_role,provider,model,effort,created_at)
             VALUES(1,'standard_orchestrator','standard_orchestrator','orchestrator','codex','test-model','medium','now')",
            [],
        )
        .unwrap();
    }

    #[test]
    fn migrations_19_and_20_repair_the_true_early_v15_shape() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bridge.db");
        let db = open(&path).unwrap();
        let late_columns = [
            ("router_decisions", "task_fingerprint"),
            ("router_decisions", "trace_id"),
            ("router_decisions", "repository_revision"),
            ("router_decisions", "profile_version"),
            ("router_decisions", "profile_purpose"),
            ("router_decisions", "policy_version"),
            ("router_decisions", "catalog_snapshot"),
            ("router_decisions", "selection_reason"),
            ("router_decisions", "actual_provider"),
            ("router_decisions", "actual_model"),
            ("router_decisions", "actual_effort"),
            ("router_outcomes", "success_state"),
            ("router_outcomes", "acceptance_state"),
            ("router_outcomes", "cost_microusd"),
            ("router_outcomes", "cost_source"),
            ("router_outcomes", "confidence_bps"),
            ("router_outcomes", "edit_count"),
            ("router_outcomes", "override_signal"),
            ("router_outcomes", "total_tokens"),
            ("router_outcomes", "latency_source"),
            ("learning_jobs", "run_budget_tokens"),
            ("learning_triggers", "auth_digest"),
            ("learning_triggers", "expires_at"),
            ("learning_triggers", "experimental"),
            ("learning_triggers", "updated_at"),
            ("learning_job_runs", "lease_owner"),
            ("learning_job_runs", "lease_expires_at"),
            ("learning_job_runs", "snapshot_frozen_at"),
            ("learning_job_runs", "evaluated_spend_microusd"),
            ("learning_job_runs", "evaluated_tokens"),
            ("learning_job_runs", "replay_passed"),
            ("learning_job_runs", "promotion_status"),
            ("routing_policies", "rollback_of"),
            ("routing_policies", "replay_report"),
            ("routing_policies", "promoted_at"),
            ("routing_policies", "activation_boundary"),
        ];
        for (table, column) in late_columns {
            db.execute_batch(&format!("ALTER TABLE {table} DROP COLUMN {column}"))
                .unwrap();
        }
        db.execute_batch(
            "DROP TABLE routing_policy_promotions;
             DROP TABLE routing_evaluations;
             DROP TABLE learning_trigger_events;
             DROP TABLE IF EXISTS learning_scope_cursors;
             DROP INDEX idx_routing_policy_active;
             ALTER TABLE routing_policies DROP COLUMN learning_scope;
             ALTER TABLE learning_job_runs DROP COLUMN learning_scope;
             CREATE UNIQUE INDEX idx_routing_policy_active
                ON routing_policies(status) WHERE status='active';
             CREATE TABLE routing_evaluations (
                id TEXT PRIMARY KEY,
                run_id TEXT NOT NULL,
                evaluator_kind TEXT NOT NULL,
                evaluator_version TEXT NOT NULL,
                score_bps INTEGER,
                confidence_bps INTEGER,
                evidence_entry_ids TEXT NOT NULL DEFAULT '[]',
                created_at TEXT NOT NULL
             );
             CREATE TABLE learning_trigger_events (
                id TEXT PRIMARY KEY,
                run_id TEXT NOT NULL REFERENCES learning_job_runs(id) ON DELETE CASCADE,
                trigger_kind TEXT NOT NULL,
                result TEXT NOT NULL,
                created_at TEXT NOT NULL
             );
             DELETE FROM schema_version WHERE version >= 19;",
        )
        .unwrap();
        drop(db);

        let db = open(&path).unwrap();
        assert_eq!(current_schema_version(&db).unwrap(), LATEST_SCHEMA_VERSION);
        for (table, column) in late_columns {
            let exists = db
                .prepare(&format!("PRAGMA table_info({table})"))
                .unwrap()
                .query_map([], |row| row.get::<_, String>(1))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
                .iter()
                .any(|name| name == column);
            assert!(exists, "migration 19 did not restore {table}.{column}");
        }
        assert!(db.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='routing_policy_promotions')",
            [], |row| row.get::<_, bool>(0),
        ).unwrap());
        let legacy_run_id: bool = db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM pragma_table_info('routing_evaluations') WHERE name='run_id')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(!legacy_run_id);
        db.execute(
            "INSERT INTO routing_evaluations(id,evaluator_kind,evaluator_version,created_at)
             VALUES('post-repair-eval','deterministic','test-v1','now')",
            [],
        )
        .unwrap();
        db.execute(
            "INSERT INTO learning_trigger_events(id,run_id,trigger_kind,result,created_at)
             VALUES('post-repair-trigger',NULL,'manual','accepted','now')",
            [],
        )
        .unwrap();
        let trigger_delete_action: String = db.query_row(
            "SELECT on_delete FROM pragma_foreign_key_list('learning_trigger_events') WHERE \"from\"='run_id'",
            [],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(trigger_delete_action, "SET NULL");
        let active_index_sql: String = db.query_row(
            "SELECT sql FROM sqlite_master WHERE type='index' AND name='idx_routing_policy_active'",
            [],
            |row| row.get(0),
        ).unwrap();
        assert!(active_index_sql.contains("status IN ('active','canary')"));
        assert!(
            active_index_sql.contains("learning_scope"),
            "the live-policy unique index must be per learning_scope: {active_index_sql}"
        );
        for (table, column) in [
            ("routing_policies", "learning_scope"),
            ("learning_job_runs", "learning_scope"),
            ("routing_policy_promotions", "learning_scope"),
        ] {
            let exists = db
                .prepare(&format!("PRAGMA table_info({table})"))
                .unwrap()
                .query_map([], |row| row.get::<_, String>(1))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
                .iter()
                .any(|name| name == column);
            assert!(exists, "migration 26 did not restore {table}.{column}");
        }
        assert!(db.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='learning_scope_cursors')",
            [], |row| row.get::<_, bool>(0),
        ).unwrap());
    }

    #[test]
    fn learning_scope_migration_backfills_legacy_global_and_allows_one_live_policy_per_scope() {
        let dir = tempfile::tempdir().unwrap();
        let db = open(&dir.path().join("bridge.db")).unwrap();
        let scope: String = db
            .query_row(
                "SELECT learning_scope FROM routing_policies WHERE version=1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(scope, "legacy:global");
        db.execute(
            "INSERT INTO routing_policies(version,status,learning_scope,weights,thresholds,created_reason,created_at)
             VALUES(2,'active','workspace:a','{}','{}','test','now')",
            [],
        )
        .unwrap();
        db.execute(
            "INSERT INTO routing_policies(version,status,learning_scope,weights,thresholds,created_reason,created_at)
             VALUES(3,'active','workspace:b','{}','{}','test','now')",
            [],
        )
        .unwrap();
        let error = db
            .execute(
                "INSERT INTO routing_policies(version,status,learning_scope,weights,thresholds,created_reason,created_at)
                 VALUES(4,'canary','workspace:a','{}','{}','test','now')",
                [],
            )
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("UNIQUE") || error.contains("unique"),
            "{error}"
        );
    }

    #[test]
    fn additive_repair_skips_tables_that_do_not_exist_yet() {
        let mut db = Connection::open_in_memory().unwrap();
        let transaction = db.transaction().unwrap();
        add_column_if_missing(&transaction, "future_table", "future_column", "TEXT").unwrap();
        assert!(!table_exists(&transaction, "future_table").unwrap());
        transaction.commit().unwrap();
    }

    #[test]
    fn fresh_and_upgraded_databases_have_identical_schema() {
        let fresh_dir = tempfile::tempdir().unwrap();
        let upgraded_dir = tempfile::tempdir().unwrap();
        let fresh = open(&fresh_dir.path().join("bridge.db")).unwrap();
        let upgraded_path = upgraded_dir.path().join("bridge.db");
        create_legacy_fixture(&upgraded_path);
        let upgraded = open(&upgraded_path).unwrap();
        assert_eq!(schema_signature(&fresh), schema_signature(&upgraded));
        assert_eq!(migration_versions(&fresh), migration_versions(&upgraded));
    }

    #[test]
    fn capability_tier_migration_preserves_actual_models_and_is_idempotent() {
        let mut db = Connection::open(":memory:").unwrap();
        {
            let transaction = db.transaction().unwrap();
            migration_1_current_schema(&transaction).unwrap();
            transaction
                .execute("INSERT INTO schema_version VALUES(1,'now')", [])
                .unwrap();
            transaction.execute("INSERT INTO projects(id,name,path,created_at) VALUES('p','Demo','/tmp/tier-migration','now')", []).unwrap();
            transaction.execute("INSERT INTO workspaces(id,project_id,city,title,branch,path,status,created_at) VALUES('w','p','Oslo','Task','bridge/task','/tmp/tier-workspace','idle','now')", []).unwrap();
            transaction.execute("INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,model) VALUES('s','w','codex','Worker','ready','reported','runtime-model')", []).unwrap();
            migration_2_session_forest(&transaction).unwrap();
            transaction
                .execute("INSERT INTO schema_version VALUES(2,'now')", [])
                .unwrap();
            transaction.commit().unwrap();
        }
        {
            let transaction = db.transaction().unwrap();
            migration_3_capability_tiers(&transaction).unwrap();
            migration_3_capability_tiers(&transaction).unwrap();
            transaction
                .execute("INSERT INTO schema_version VALUES(3,'now')", [])
                .unwrap();
            transaction.commit().unwrap();
        }
        let (model, tier): (String, Option<String>) = db
            .query_row(
                "SELECT model,requested_tier FROM sessions WHERE id='s'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(model, "runtime-model");
        assert_eq!(tier, None);
        let tier_columns: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('sessions') WHERE name='requested_tier'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(tier_columns, 1);
    }

    #[test]
    fn continuation_fidelity_migration_derives_conservative_history_markers() {
        let mut db = open(Path::new(":memory:")).unwrap();
        db.execute("INSERT INTO projects(id,name,path,created_at) VALUES('p','Demo','/tmp/fidelity','now')", []).unwrap();
        db.execute("INSERT INTO workspaces(id,project_id,city,title,branch,path,status,created_at) VALUES('w','p','Oslo','Task','bridge/task','/tmp/fidelity-w','idle','now')", []).unwrap();
        for (id, parent) in [
            ("root", None),
            ("boundary", Some("root")),
            ("mid", Some("root")),
            ("resumed", Some("root")),
        ] {
            db.execute("INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,parent_session_id,continuation_fidelity) VALUES(?1,'w','codex','Session','stopped','reported',?2,'native')", params![id,parent]).unwrap();
        }
        for (id, mode) in [
            ("root", "fresh"),
            ("boundary", "checkpoint_restored"),
            ("mid", "fresh"),
            ("resumed", "native"),
        ] {
            db.execute("INSERT INTO session_heads(session_id,restoration_mode,updated_at) VALUES(?1,?2,'now')", params![id,mode]).unwrap();
        }
        let transaction = db.transaction().unwrap();
        migration_10_continuation_fidelity(&transaction).unwrap();
        transaction.commit().unwrap();
        let values = query(&db, "SELECT continuation_fidelity FROM sessions ORDER BY CASE id WHEN 'root' THEN 1 WHEN 'boundary' THEN 2 WHEN 'mid' THEN 3 ELSE 4 END", |row| row.get::<_,String>(0)).unwrap();
        assert_eq!(
            values,
            vec![
                "native",
                "projected_at_boundary",
                "projected_mid_turn",
                "native"
            ]
        );
    }

    #[test]
    fn human_blocked_queue_migration_preserves_existing_rows() {
        let mut db = Connection::open(":memory:").unwrap();
        db.execute_batch("CREATE TABLE worker_queue(id TEXT PRIMARY KEY,queue_status TEXT NOT NULL,expires_at TEXT); INSERT INTO worker_queue VALUES('q','queued','2099-01-01T00:00:00+00:00');").unwrap();
        let transaction = db.transaction().unwrap();
        migration_11_human_blocked_queue(&transaction).unwrap();
        transaction.commit().unwrap();
        let row = db
            .query_row(
                "SELECT queue_status,expires_at,blocked_at FROM worker_queue WHERE id='q'",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(
            row,
            ("queued".into(), "2099-01-01T00:00:00+00:00".into(), None)
        );
    }

    #[test]
    fn adapter_process_claim_migration_preserves_existing_sessions() {
        let mut db = Connection::open(":memory:").unwrap();
        db.execute_batch("CREATE TABLE sessions(id TEXT PRIMARY KEY,status TEXT NOT NULL); INSERT INTO sessions VALUES('s','working');").unwrap();
        let transaction = db.transaction().unwrap();
        migration_12_adapter_process_claims(&transaction).unwrap();
        transaction.commit().unwrap();
        let row = db
            .query_row(
                "SELECT status,adapter_pid,adapter_process_identity FROM sessions WHERE id='s'",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<i64>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(row, ("working".into(), None, None));
    }

    #[test]
    fn resume_eligibility_migration_preserves_existing_restoration_state() {
        let mut db = Connection::open(":memory:").unwrap();
        {
            let transaction = db.transaction().unwrap();
            migration_1_current_schema(&transaction).unwrap();
            migration_2_session_forest(&transaction).unwrap();
            migration_3_capability_tiers(&transaction).unwrap();
            transaction.execute("INSERT INTO projects(id,name,path,created_at) VALUES('p','Demo','/tmp/resume-migration','now')", []).unwrap();
            transaction.execute("INSERT INTO workspaces(id,project_id,city,title,branch,path,status,created_at) VALUES('w','p','Oslo','Task','bridge/task','/tmp/resume-workspace','stopped','now')", []).unwrap();
            transaction.execute("INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,provider_session_id) VALUES('s','w','codex','Worker','stopped','reported','native-s')", []).unwrap();
            transaction.execute("INSERT INTO session_heads(session_id,native_provider_session_id,restoration_mode,updated_at) VALUES('s','native-s','native','now')", []).unwrap();
            transaction.commit().unwrap();
        }

        let transaction = db.transaction().unwrap();
        migration_4_resume_eligibility(&transaction).unwrap();
        migration_4_resume_eligibility(&transaction).unwrap();
        transaction.commit().unwrap();

        let head = session_head(&db, "s").unwrap().unwrap();
        assert_eq!(head.native_provider_session_id.as_deref(), Some("native-s"));
        assert_eq!(head.restoration_mode, RestorationMode::Native);
        assert_eq!(head.resume_eligibility, ResumeEligibility::Fresh);
    }

    /// Inserts one run and returns its id, satisfying every NOT NULL column.
    fn insert_work_run(db: &Connection, id: &str) {
        db.execute(
            "INSERT INTO work_brief_runs(id,trigger_kind,status,max_wall_seconds,max_turns,
                 max_tool_calls,started_at)
             VALUES(?1,'manual','running',600,12,24,'2026-08-19T09:00:00+00:00')",
            params![id],
        )
        .unwrap();
    }

    #[test]
    fn migration_24_adds_the_work_tables_to_an_existing_database() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bridge.db");
        create_legacy_fixture(&path);
        let db = open(&path).unwrap();
        for table in [
            "work_brief_runs",
            "work_brief_sources",
            "work_evidence",
            "work_tasks",
            "work_fact_cache",
        ] {
            assert!(
                db.query_row(
                    "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
                    params![table],
                    |row| row.get::<_, bool>(0),
                )
                .unwrap(),
                "missing migration-24 table {table}"
            );
        }
        // The upgrade is additive: the fixture's own rows are still there, which
        // is what "readable by the previous binary" rests on.
        let sessions: i64 = db
            .query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get(0))
            .unwrap();
        assert_eq!(sessions, 1);
        let entries: i64 = db
            .query_row("SELECT COUNT(*) FROM session_entries", [], |row| row.get(0))
            .unwrap();
        assert_eq!(entries, 2, "backfilled entries survive the Work migration");
    }

    /// The inventory has to arrive already knowing what the machine holds,
    /// because the worktrees that motivated it were created long before it
    /// existed. Backfill works off recorded paths — the data directory is not
    /// visible from this layer — so each source table contributes its own rows.
    #[test]
    fn migration_50_backfills_the_worktrees_it_can_recognise() {
        let db = open(Path::new(":memory:")).unwrap();
        db.execute(
            "INSERT INTO projects(id,name,path,created_at) VALUES('p','Demo','/repos/demo','now')",
            [],
        )
        .unwrap();
        db.execute(
            "INSERT INTO workspaces(id,project_id,title,branch,path,status,created_at)
             VALUES('w','p','Task','main','/repos/demo','idle','now')",
            [],
        )
        .unwrap();
        db.execute(
            "INSERT INTO workspaces(id,project_id,title,branch,path,status,created_at)
             VALUES('pr','p','PR #7','feat/x','/data/worktrees/github/pr-7-feat-x','idle','now')",
            [],
        )
        .unwrap();
        db.execute(
            "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,cwd,depth)
             VALUES('chat','w','codex','Chat','idle','estimated','/data/worktrees/orchestrators/task/chat',0)",
            [],
        )
        .unwrap();
        db.execute(
            "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,parent_session_id,depth)
             VALUES('child','w','claude','Worker','completed','reported','chat',1)",
            [],
        )
        .unwrap();
        db.execute(
            "INSERT INTO worker_worktree_adoptions(
                session_id,parent_session_id,workspace_id,worktree_path,worktree_branch,
                task_worktree_path,state,base_commit,created_at,updated_at)
             VALUES('child','chat','w','/data/worktrees/workers/task/child','main-worker-child',
                    '/repos/demo','pending_adoption','abc123','now','now')",
            [],
        )
        .unwrap();
        db.execute(
            "INSERT INTO worker_runtime(
                session_id,parent_session_id,lifecycle_state,task_family,compatibility_key,
                worktree_path,worktree_branch,updated_at)
             VALUES('child','chat','stopped','implementation','claude',
                    '/data/worktrees/workers/task/child','main-worker-child','now')",
            [],
        )
        .unwrap();

        // Re-run the migration against the populated database: `open` applied it
        // to an empty one, so this is what an upgrade in place actually does.
        let mut db = db;
        let transaction = db.transaction().unwrap();
        migration_50_worktree_inventory(&transaction).unwrap();
        transaction.commit().unwrap();

        let rows = |kind: &str| -> Vec<(String, String)> {
            let mut statement = db
                .prepare("SELECT path,repo_root FROM worktrees WHERE kind=?1 ORDER BY path")
                .unwrap();
            let mapped = statement
                .query_map(params![kind], |row| Ok((row.get(0)?, row.get(1)?)))
                .unwrap();
            mapped.map(Result::unwrap).collect()
        };

        assert_eq!(
            rows("worker"),
            vec![(
                "/data/worktrees/workers/task/child".to_owned(),
                "/repos/demo".to_owned()
            )],
            "the adoption row wins over the runtime row for the same path",
        );
        assert_eq!(
            rows("orchestrator"),
            vec![(
                "/data/worktrees/orchestrators/task/chat".to_owned(),
                "/repos/demo".to_owned()
            )],
            "the class that previously had no record of any kind",
        );
        assert_eq!(
            rows("github"),
            vec![("/data/worktrees/github/pr-7-feat-x".to_owned(), String::new())],
        );
        let base: Option<String> = db
            .query_row(
                "SELECT base_commit FROM worktrees WHERE kind='worker'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(base.as_deref(), Some("abc123"));

        // Idempotent: a second pass adds nothing.
        let transaction = db.transaction().unwrap();
        migration_50_worktree_inventory(&transaction).unwrap();
        transaction.commit().unwrap();
        let total: i64 = db
            .query_row("SELECT COUNT(*) FROM worktrees", [], |row| row.get(0))
            .unwrap();
        assert_eq!(total, 3);
    }

    #[test]
    fn work_tables_declare_their_unique_constraints() {
        let db = open(Path::new(":memory:")).unwrap();
        insert_work_run(&db, "run-1");
        insert_work_run(&db, "run-2");

        db.execute(
            "INSERT INTO work_brief_sources(run_id,connector_instance_id,connector_family,status)
             VALUES('run-1','github:acme','github','eligible')",
            [],
        )
        .unwrap();
        assert!(
            db.execute(
                "INSERT INTO work_brief_sources(run_id,connector_instance_id,connector_family,status)
                 VALUES('run-1','github:acme','github','failed')",
                [],
            )
            .is_err(),
            "one row per run and connector instance, or coverage can say two things at once"
        );
        db.execute(
            "INSERT INTO work_brief_sources(run_id,connector_instance_id,connector_family,status)
             VALUES('run-2','github:acme','github','eligible')",
            [],
        )
        .expect("the same connector in a different run is a different row");

        db.execute(
            "INSERT INTO work_evidence(run_id,evidence_ref,tool_call_id,connector_instance_id,
                 canonical_resource_id,source_kind,tool_definition_digest,result_digest,succeeded,observed_at)
             VALUES('run-1','ev-1','call-1','github:acme','acme/bridge#204','github_issue',
                    'sha256:tool','sha256:result',1,'2026-08-19T09:00:00+00:00')",
            [],
        )
        .unwrap();
        assert!(
            db.execute(
                "INSERT INTO work_evidence(run_id,evidence_ref,tool_call_id,connector_instance_id,
                     canonical_resource_id,source_kind,tool_definition_digest,result_digest,succeeded,observed_at)
                 VALUES('run-1','ev-1','call-2','github:acme','acme/bridge#9','github_issue',
                        'sha256:tool','sha256:other',1,'2026-08-19T09:00:00+00:00')",
                [],
            )
            .is_err(),
            "an evidence reference must mean one thing inside a run"
        );

        let insert_task = |id: &str, fingerprint: Option<&str>| {
            db.execute(
                "INSERT INTO work_tasks(id,fingerprint,connector_instance_id,canonical_resource_id,
                     source_kind,title,why,rank,confidence_bps,created_at,updated_at)
                 VALUES(?1,?2,'github:acme','acme/bridge#204','github_issue','Review','because',
                        1,8200,'now','now')",
                params![id, fingerprint],
            )
        };
        insert_task("t-1", Some("fp-1")).unwrap();
        assert!(
            insert_task("t-2", Some("fp-1")).is_err(),
            "two tasks cannot share a fingerprint"
        );
        insert_task("t-3", None).unwrap();
        insert_task("t-4", None)
            .expect("ephemeral tasks have no fingerprint, and SQLite counts NULLs as distinct");

        db.execute(
            "INSERT INTO work_fact_cache(kind,cache_key,status,observed_at)
             VALUES('workspace_behind_base','w','ok','2026-08-19T09:00:00+00:00')",
            [],
        )
        .unwrap();
        assert!(
            db.execute(
                "INSERT INTO work_fact_cache(kind,cache_key,status,observed_at)
                 VALUES('workspace_behind_base','w','failed','2026-08-19T09:05:00+00:00')",
                [],
            )
            .is_err(),
            "one observation per kind and key; a second row would make freshness ambiguous"
        );
    }

    #[test]
    fn work_tables_cascade_from_their_run() {
        let db = open(Path::new(":memory:")).unwrap();
        insert_work_run(&db, "run-1");
        db.execute(
            "INSERT INTO work_brief_sources(run_id,connector_instance_id,connector_family,status)
             VALUES('run-1','github:acme','github','succeeded')",
            [],
        )
        .unwrap();
        db.execute(
            "INSERT INTO work_evidence(run_id,evidence_ref,tool_call_id,connector_instance_id,
                 canonical_resource_id,source_kind,tool_definition_digest,result_digest,succeeded,observed_at)
             VALUES('run-1','ev-1','call-1','github:acme','acme/bridge#204','github_issue',
                    'sha256:tool','sha256:result',1,'2026-08-19T09:00:00+00:00')",
            [],
        )
        .unwrap();
        db.execute("DELETE FROM work_brief_runs WHERE id='run-1'", [])
            .unwrap();
        for table in ["work_brief_sources", "work_evidence"] {
            let remaining: i64 = db
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(remaining, 0, "{table} must not outlive its run");
        }
    }

    #[test]
    fn migration_24_rolls_back_every_object_when_one_fails() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bridge.db");
        let mut db = Connection::open(&path).unwrap();
        // A view cannot be indexed, and `CREATE TABLE IF NOT EXISTS` over a view
        // is a silent no-op — so this fails partway through the batch, after
        // `work_brief_runs` has already been created.
        db.execute_batch("CREATE VIEW work_brief_sources AS SELECT 1 AS run_id;")
            .unwrap();
        let transaction = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        assert!(migration_24_work_board(&transaction).is_err());
        drop(transaction);
        let created: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name LIKE 'work_%' AND type='table'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            created, 0,
            "a half-applied Work schema must not survive the failure"
        );
    }

    #[test]
    fn migration_failure_rolls_back_objects_and_version() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bridge.db");
        let mut db = Connection::open(&path).unwrap();
        let transaction = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        migration_1_current_schema(&transaction).unwrap();
        transaction
            .execute(
                "INSERT INTO schema_version(version,applied_at) VALUES(1,'now')",
                [],
            )
            .unwrap();
        transaction.commit().unwrap();
        db.execute("CREATE TABLE session_entries(blocker TEXT)", [])
            .unwrap();
        drop(db);
        assert!(open(&path).is_err());
        let db = Connection::open(&path).unwrap();
        assert_eq!(migration_versions(&db), vec![1]);
        let partial: bool = db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='session_heads')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(!partial);
    }

    #[test]
    fn backfills_agent_events_as_linear_session_entries() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bridge.db");
        create_legacy_fixture(&path);
        let db = open(&path).unwrap();
        let entries = session_entries(&db, "s").unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].sequence, 1);
        assert_eq!(entries[0].parent_entry_id, None);
        assert_eq!(entries[1].parent_entry_id.as_deref(), Some(&*entries[0].id));
        assert_eq!(entries[1].payload["providerMeta"]["rawId"], 2);
        assert_eq!(entries[1].provider_event_id.as_deref(), Some("m2"));
        assert!(entries
            .iter()
            .all(|entry| entry.semantic_schema_version == 1));
        let head = session_head(&db, "s").unwrap().unwrap();
        assert_eq!(head.active_entry_id.as_deref(), Some(&*entries[1].id));
        assert_eq!(head.native_provider_session_id.as_deref(), Some("native-s"));
        assert_eq!(head.restoration_mode, RestorationMode::Fresh);
        assert_eq!(head.resume_eligibility, ResumeEligibility::Native);
    }

    #[test]
    fn the_snapshot_window_keeps_the_newest_entries_oldest_first() {
        let dir = tempfile::tempdir().unwrap();
        let db = open(&dir.path().join("bridge.db")).unwrap();
        seed_workspace(&db);
        let mut parent: Option<String> = None;
        for index in 0..10 {
            let entry = append_session_entry(
                &db,
                "s",
                parent.as_deref(),
                "assistant.message",
                &json!({"text": format!("entry {index}")}),
                None,
                "eligible",
                Some(1),
            )
            .unwrap();
            parent = Some(entry.id);
        }

        let window = session_entry_window(&db, "s", 4).unwrap();
        assert_eq!(window.total, 10, "the total counts the whole session");
        assert_eq!(window.entries.len(), 4);
        // Oldest-first inside the window: branch projection walks parents from
        // the head, and the transcript renders in order.
        assert_eq!(
            window
                .entries
                .iter()
                .map(|entry| entry.sequence)
                .collect::<Vec<_>>(),
            vec![7, 8, 9, 10],
        );

        let whole = session_entry_window(&db, "s", 100).unwrap();
        assert_eq!(whole.entries.len(), 10);
        assert_eq!(whole.total, 10);
        assert_eq!(
            whole.trimmed_payloads, 0,
            "small payloads travel exactly as stored"
        );
    }

    #[test]
    fn the_snapshot_window_trims_oversized_payload_strings_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let db = open(&dir.path().join("bridge.db")).unwrap();
        seed_workspace(&db);
        // The real shape that made a chat unopenable: a `command.started`
        // whose title is the whole command and whose nested data carries the
        // whole output.
        let huge = "x".repeat(SNAPSHOT_STRING_BYTES * 3);
        append_session_entry(
            &db,
            "s",
            None,
            "command.started",
            &json!({
                "itemId": "call-1",
                "title": huge.clone(),
                "status": "inProgress",
                "data": {"state": {"metadata": {"output": huge.clone()}}},
            }),
            None,
            "eligible",
            Some(1),
        )
        .unwrap();

        let window = session_entry_window(&db, "s", 10).unwrap();
        assert_eq!(window.trimmed_payloads, 1);
        let payload = &window.entries[0].payload;
        // Structure survives: the codec reads these fields by name, so trimming
        // has to shorten them, never drop them.
        assert_eq!(payload["itemId"], json!("call-1"));
        assert_eq!(payload["status"], json!("inProgress"));
        for value in [
            &payload["title"],
            &payload["data"]["state"]["metadata"]["output"],
        ] {
            let text = value.as_str().expect("a trimmed string is still a string");
            assert!(
                text.len() < huge.len(),
                "an oversized string must be shortened"
            );
            assert!(
                text.contains("more bytes not shown"),
                "truncation must be visible rather than silent: {text:.80}"
            );
        }

        // Untrimmed reads are unaffected — compaction and context projection
        // still need the real payload.
        let full = session_entries(&db, "s").unwrap();
        assert_eq!(full[0].payload["title"].as_str().unwrap().len(), huge.len());
    }

    #[test]
    fn trimming_a_payload_never_splits_a_character() {
        // A multi-byte character straddling the cap: `String::truncate` panics
        // off a char boundary, so the cut walks back to one.
        let filler = "e".repeat(SNAPSHOT_STRING_BYTES - 1);
        let mut value = json!({"text": format!("{filler}\u{1f600}tail")});
        assert!(trim_snapshot_strings(&mut value));
        let text = value["text"].as_str().unwrap();
        assert!(text.starts_with(&filler));
        assert!(!text.contains('\u{fffd}'), "no replacement character");
        assert!(text.contains("more bytes not shown"));
    }

    #[test]
    fn reason_events_are_bounded_to_the_newest_window() {
        let dir = tempfile::tempdir().unwrap();
        let db = open(&dir.path().join("bridge.db")).unwrap();
        seed_workspace(&db);
        for index in 0..12 {
            event(&db, "policy", "worker.spawned", "s", &format!("body {index}")).unwrap();
        }
        let bounded = workspace_reason_events(&db, "w", 5).unwrap();
        assert_eq!(bounded.len(), 5, "the feed is capped");
        assert_eq!(
            bounded[0].body, "body 11",
            "the newest event is still first"
        );
    }

    /// The regression guard for the bug this window exists to fix.
    ///
    /// A forest snapshot crosses to the UI as **one** newline-delimited JSON
    /// frame, and `bridge_client::MAX_SERVER_FRAME_BYTES` caps that frame at
    /// 64 MB. Past the cap the read does not degrade — it errors, the reader
    /// thread exits, and every subscriber on that daemon connection is
    /// dropped. So an oversized snapshot is not a slow chat, it is a chat that
    /// cannot be opened and takes the connection with it.
    ///
    /// Shaped after the real payload that caused it: a `command.started` whose
    /// `title` is the entire command and whose nested `data` carries the
    /// entire output. One real session held 7,344 of these, 123 MB in total.
    #[test]
    fn a_snapshot_of_a_pathological_session_stays_inside_the_client_frame_limit() {
        const MAX_SERVER_FRAME_BYTES: usize = 64 * 1024 * 1024;
        let dir = tempfile::tempdir().unwrap();
        let db = open(&dir.path().join("bridge.db")).unwrap();
        seed_workspace(&db);

        let fat = "x".repeat(24 * 1024);
        let entries = 2_000i64;
        // Inserted directly rather than through `append_session_entry`: that
        // opens its own transaction per call, and 2,000 of them is the
        // difference between a test that runs in a second and one nobody
        // wants in CI.
        let transaction = db.unchecked_transaction().unwrap();
        for index in 0..entries {
            let payload = json!({
                "itemId": format!("call-{index}"),
                "status": "inProgress",
                "title": fat,
                "data": {"state": {"metadata": {"output": fat, "stderr": fat}}},
            });
            transaction
                .execute(
                    "INSERT INTO session_entries(id,session_id,parent_entry_id,sequence,semantic_schema_version,kind,payload,context_visibility,token_estimate,created_at)
                     VALUES(?1,'s',?2,?3,1,'command.started',?4,'eligible',1,'now')",
                    params![
                        format!("e-{index}"),
                        (index > 0).then(|| format!("e-{}", index - 1)),
                        index + 1,
                        payload.to_string(),
                    ],
                )
                .unwrap();
        }
        transaction
            .execute(
                "INSERT INTO session_heads(session_id,active_entry_id,restoration_mode,resume_eligibility,updated_at)
                 VALUES('s',?1,'fresh','fresh','now')",
                params![format!("e-{}", entries - 1)],
            )
            .unwrap();
        transaction.commit().unwrap();

        // What the snapshot used to carry: every entry, untrimmed.
        let before = serde_json::to_vec(&session_entries(&db, "s").unwrap())
            .unwrap()
            .len();
        let after = serde_json::to_vec(
            &crate::sessions::session_forest_snapshot_with_repository_state(
                &db,
                "s",
                json!({"status": "unavailable"}),
            )
            .unwrap(),
        )
        .unwrap()
        .len();

        assert!(
            before > MAX_SERVER_FRAME_BYTES,
            "the fixture has to actually reproduce the failure, or this test \
             proves nothing: {before} bytes did not exceed the {MAX_SERVER_FRAME_BYTES} byte frame limit"
        );
        assert!(
            after < MAX_SERVER_FRAME_BYTES,
            "a snapshot must fit in one frame: {after} bytes"
        );
        println!(
            "snapshot bytes: {before} -> {after} ({:.1}x smaller)",
            before as f64 / after as f64
        );
        // Deliberately generous. This fixture is uniformly worst-case — every
        // entry maximally fat — which understates the win: on a real database
        // the heaviest session measured 123.1 MB -> 6.6 MB. The assertion
        // guards the order of magnitude so ordinary payload churn cannot turn
        // it into a brittle failure.
        assert!(
            after * 5 < before,
            "the window must cut the payload by at least 5x: {before} -> {after}"
        );
        // The whole session is still counted, so the UI can say how much of it
        // is on screen instead of presenting the tail as the entire chat.
        let window = session_entry_window(&db, "s", SNAPSHOT_ENTRY_WINDOW).unwrap();
        assert_eq!(window.total, entries);
        assert_eq!(window.entries.len(), SNAPSHOT_ENTRY_WINDOW);
    }

    #[test]
    fn session_entry_sequence_is_unique_and_append_assigns_next_sequence() {
        let dir = tempfile::tempdir().unwrap();
        let db = open(&dir.path().join("bridge.db")).unwrap();
        seed_workspace(&db);
        let first = append_session_entry(
            &db,
            "s",
            None,
            "user.message",
            &json!({"text":"one"}),
            None,
            "eligible",
            Some(1),
        )
        .unwrap();
        let second = append_session_entry(
            &db,
            "s",
            Some(&first.id),
            "assistant.message",
            &json!({"text":"two"}),
            None,
            "eligible",
            Some(1),
        )
        .unwrap();
        assert_eq!((first.sequence, second.sequence), (1, 2));
        assert_eq!(first.semantic_schema_version, SEMANTIC_EVENT_SCHEMA_VERSION);
        assert_eq!(
            second.semantic_schema_version,
            SEMANTIC_EVENT_SCHEMA_VERSION
        );
        assert_eq!(
            session_head(&db, "s").unwrap().unwrap().active_entry_id,
            Some(second.id.clone())
        );
        let duplicate = db.execute(
            "INSERT INTO session_entries(id,session_id,sequence,kind,created_at) VALUES('duplicate','s',2,'user.message','now')",
            [],
        );
        assert!(duplicate.is_err());
    }

    #[test]
    fn foreign_keys_are_enforced() {
        let dir = tempfile::tempdir().unwrap();
        let db = open(&dir.path().join("bridge.db")).unwrap();
        assert_eq!(
            db.query_row("PRAGMA foreign_keys", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert!(append_session_entry(
            &db,
            "missing",
            None,
            "user.message",
            &json!({}),
            None,
            "eligible",
            None,
        )
        .is_err());
    }

    #[test]
    fn queries_leases_and_usage_ledger() {
        let dir = tempfile::tempdir().unwrap();
        let db = open(&dir.path().join("bridge.db")).unwrap();
        seed_workspace(&db);

        let lease = WorkerLease {
            session_id: "s".into(),
            workspace_id: "w".into(),
            role: "implementation".into(),
            capability_tier: "standard".into(),
            task_family: "implementation".into(),
            owned_paths: json!(["src/**"]),
            write_mode: "isolated".into(),
            lease_status: "active".into(),
            expires_at: None,
            created_at: "now".into(),
            updated_at: "now".into(),
                    };
        upsert_worker_lease(&db, &lease).unwrap();
        assert_eq!(worker_leases(&db, "w").unwrap(), vec![lease]);

        let compilation = PromptCompilationRecord {
            id: 0,
            session_id: "s".into(),
            turn_id: None,
            prefix_id: "prefix-1".into(),
            prefix_hash: "hash-1".into(),
            schema_version: 1,
            prefix_bytes: 400,
            prefix_token_estimate: 100,
            harness: "codex".into(),
            model: Some("gpt".into()),
            role: "worker:implementation".into(),
            task_family: "implementation".into(),
            restoration_mode: "fresh".into(),
            cross_harness_reuse: "not_applicable".into(),
            created_at: "now".into(),
            sections_json: Some(
                json!([{"region":"stable","kind":"role","name":"role","bytes":40,"tokenEstimate":10}])
                    .to_string(),
            ),
            stable_bytes: Some(400),
            variable_bytes: Some(120),
            stable_token_estimate: Some(100),
            variable_token_estimate: Some(30),
            token_estimate_source: Some("bytes_div4_v1".into()),
        };
        let compilation_id = record_prompt_compilation(&db, &compilation).unwrap();
        let stored_compilation = latest_prompt_compilation(&db, "s").unwrap().unwrap();
        assert_eq!(stored_compilation.id, compilation_id);
        assert_eq!(stored_compilation.prefix_hash, "hash-1");
        assert_eq!(stored_compilation.prefix_token_estimate, 100);
        assert_eq!(stored_compilation.sections_json, compilation.sections_json);
        assert_eq!(stored_compilation.stable_bytes, Some(400));
        assert_eq!(stored_compilation.variable_bytes, Some(120));
        assert_eq!(stored_compilation.stable_token_estimate, Some(100));
        assert_eq!(stored_compilation.variable_token_estimate, Some(30));
        assert_eq!(
            stored_compilation.token_estimate_source.as_deref(),
            Some("bytes_div4_v1")
        );
        assert!(bind_latest_prompt_compilation_to_turn(&db, "s", "turn-1").unwrap());
        assert_eq!(
            prompt_compilation_for_turn(&db, "s", "turn-1")
                .unwrap()
                .unwrap()
                .id,
            compilation_id
        );
        let mut replacement = compilation.clone();
        replacement.prefix_id = "prefix-unsent".into();
        replacement.prefix_hash = "hash-unsent".into();
        let replacement_id = record_prompt_compilation(&db, &replacement).unwrap();
        assert!(delete_prompt_compilation(&db, replacement_id).unwrap());
        assert!(!delete_prompt_compilation(&db, replacement_id).unwrap());
        assert_eq!(
            latest_prompt_compilation(&db, "s").unwrap().unwrap().id,
            compilation_id
        );

        let usage = UsageLedgerRow {
            id: 0,
            workspace_id: "w".into(),
            session_id: Some("s".into()),
            turn_id: Some("turn-1".into()),
            input_tokens: Some(10),
            output_tokens: Some(5),
            cache_read_tokens: Some(2),
            cache_write_tokens: None,
            uncached_input_tokens: Some(8),
            context_percent: Some(25),
            capability_units: 3,
            runtime_ms: Some(100),
            cost_microusd: Some(12_345),
            cost_source: Some("provider_reported".into()),
            stable_prefix_id: Some("prefix-1".into()),
            stable_prefix_hash: Some("hash-1".into()),
            prompt_schema_version: Some(1),
            prefix_token_estimate: Some(100),
            harness: Some("codex".into()),
            model: Some("gpt".into()),
            role: Some("worker:implementation".into()),
            task_family: Some("implementation".into()),
            restoration_mode: Some("fresh".into()),
            cross_harness_reuse: Some("not_applicable".into()),
            reasoning_tokens: None,
            serving_model: None,
            context_window_tokens: None,
            context_used_tokens: None,
            provider_record_id: None,
            cache_savings_microusd: None,
            source: "codex".into(),
            created_at: "now".into(),
        };
        let id = append_usage_ledger(&db, &usage).unwrap();
        let rows = usage_ledger(&db, "w", Some("s")).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, id);
        assert_eq!(rows[0].turn_id.as_deref(), Some("turn-1"));
        assert_eq!(rows[0].capability_units, 3);
        assert_eq!(rows[0].cost_microusd, Some(12_345));
        assert_eq!(rows[0].uncached_input_tokens, Some(8));
        assert_eq!(rows[0].stable_prefix_hash.as_deref(), Some("hash-1"));
    }

    #[test]
    fn legacy_prompt_compilation_row_with_null_accounting_reads_back_without_error() {
        let dir = tempfile::tempdir().unwrap();
        let db = open(&dir.path().join("bridge.db")).unwrap();
        seed_workspace(&db);

        let legacy = PromptCompilationRecord {
            id: 0,
            session_id: "s".into(),
            turn_id: None,
            prefix_id: "prefix-legacy".into(),
            prefix_hash: "hash-legacy".into(),
            schema_version: 1,
            prefix_bytes: 400,
            prefix_token_estimate: 100,
            harness: "codex".into(),
            model: Some("gpt".into()),
            role: "worker:implementation".into(),
            task_family: "implementation".into(),
            restoration_mode: "fresh".into(),
            cross_harness_reuse: "not_applicable".into(),
            created_at: "now".into(),
            sections_json: None,
            stable_bytes: None,
            variable_bytes: None,
            stable_token_estimate: None,
            variable_token_estimate: None,
            token_estimate_source: None,
        };
        record_prompt_compilation(&db, &legacy).unwrap();
        assert!(bind_latest_prompt_compilation_to_turn(&db, "s", "turn-legacy").unwrap());

        let stored = latest_prompt_compilation(&db, "s").unwrap().unwrap();
        assert_eq!(stored.sections_json, None);
        assert_eq!(stored.stable_bytes, None);
        assert_eq!(stored.variable_bytes, None);
        assert_eq!(stored.stable_token_estimate, None);
        assert_eq!(stored.variable_token_estimate, None);
        assert_eq!(stored.token_estimate_source, None);

        let for_turn = prompt_compilation_for_turn(&db, "s", "turn-legacy")
            .unwrap()
            .unwrap();
        assert_eq!(for_turn.variable_bytes, None);
    }

    #[test]
    fn durable_worker_bookkeeping_deduplicates_counts_and_queues_fifo() {
        let dir = tempfile::tempdir().unwrap();
        let db = open(&dir.path().join("bridge.db")).unwrap();
        seed_workspace(&db);
        db.execute("INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,parent_session_id,depth) VALUES('child','w','claude','Worker','working','reported','s',1)", []).unwrap();

        assert!(claim_delegation_receipt(&db, "s", "message-1").unwrap());
        assert!(!claim_delegation_receipt(&db, "s", "message-1").unwrap());
        let runtime = WorkerRuntimeRecord {
            session_id: "child".into(),
            parent_session_id: "s".into(),
            lifecycle_state: "working".into(),
            task_family: "implementation".into(),
            compatibility_key: "w|implementation|claude|standard|implementation|src/**".into(),
            result_status: "pending".into(),
            retry_count: 0,
            warm_until: None,
            worktree_path: None,
            worktree_branch: None,
            last_result: None,
            last_activity_at: Some("active-now".into()),
            waiting_since: None,
            waiting_reason: None,
            progress_summary: None,
            updated_at: "now".into(),
            failure_class: None,
        };
        let mut runtime = runtime;
        runtime.failure_class = Some("stalled".into());
        upsert_worker_runtime(&db, &runtime).unwrap();
        assert_eq!(worker_runtime(&db, "child").unwrap(), Some(runtime.clone()));
        runtime.failure_class = None;
        upsert_worker_runtime(&db, &runtime).unwrap();
        assert_eq!(worker_runtime(&db, "child").unwrap(), Some(runtime));
        assert_eq!(outstanding_children(&db, "s").unwrap(), 1);

        for id in ["q1", "q2"] {
            enqueue_worker_request(
                &db,
                &QueuedWorkerRequest {
                    id: id.into(),
                    parent_session_id: "s".into(),
                    workspace_id: "w".into(),
                    turn_id: "turn".into(),
                    request: json!({"role":"implementation"}),
                    actual_model: "runtime-model".into(),
                    queue_status: "queued".into(),
                    sequence: 0,
                    dispatched_session_id: None,
                    attempt_count: 0,
                    expires_at: "2099-01-01T00:00:00+00:00".into(),
                    blocked_at: None,
                    claimed_at: None,
                    last_error: None,
                    created_at: "now".into(),
                    updated_at: "now".into(),
                                    },
            )
            .unwrap();
        }
        let queued = queued_worker_requests(&db, "w").unwrap();
        assert_eq!(
            queued
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            vec!["q1", "q2"]
        );
        assert!(queued[0].sequence < queued[1].sequence);
    }

    #[test]
    fn busy_wal_aborts_before_migration_backup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bridge.db");
        create_legacy_fixture(&path);

        let reader = Connection::open(&path).unwrap();
        reader
            .execute_batch("PRAGMA journal_mode=WAL; BEGIN")
            .unwrap();
        let _: i64 = reader
            .query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get(0))
            .unwrap();
        let writer = Connection::open(&path).unwrap();
        writer
            .execute("UPDATE sessions SET label='Changed' WHERE id='s'", [])
            .unwrap();
        drop(writer);

        let error = open(&path).unwrap_err().to_string();
        assert!(error.contains("WAL is busy"), "unexpected error: {error}");
        assert!(backup_paths(dir.path()).is_empty());
        let raw = Connection::open(&path).unwrap();
        let has_versions: bool = raw
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='schema_version')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(!has_versions);
        drop(raw);
        reader.execute_batch("ROLLBACK").unwrap();
    }

    // Issue #400 foundation: a whole-Mac analytics index that is structurally
    // separate from the workspace-scoped usage ledger. These tests lock the
    // schema and its independence before any importer exists to fill it.
    #[test]
    fn the_ledger_repair_corrects_claude_costs_without_guessing_codex_provenance() {
        let mut db = open(Path::new(":memory:")).unwrap();
        db.execute_batch(
            "INSERT INTO projects(id,name,path,created_at) VALUES('p','Demo','/tmp/usage-repair','now');
             INSERT INTO workspaces(id,project_id,city,title,branch,path,status,created_at) VALUES('w','p','c','t','main','/tmp/usage-repair/w','ready','now');
             INSERT INTO sessions(id,workspace_id,harness,label,status) VALUES('c','w','claude','c','ready');
             INSERT INTO sessions(id,workspace_id,harness,label,status) VALUES('x-total','w','codex','x','ready');
             INSERT INTO sessions(id,workspace_id,harness,label,status) VALUES('x-last','w','codex','y','ready');
             INSERT INTO sessions(id,workspace_id,harness,label,status) VALUES('x-new','w','codex','z','ready');
             INSERT INTO sessions(id,workspace_id,harness,label,status) VALUES('x-fresh','w','codex','fresh','ready');
             -- Claude: a running total 0.50, 0.80, 1.10, then a fresh run at 0.20.
             INSERT INTO usage_ledger(workspace_id,session_id,turn_id,output_tokens,cost_microusd,cost_source,source,created_at) VALUES('w','c','t1',10,500000,'provider_reported','provider.claude','a');
             INSERT INTO usage_ledger(workspace_id,session_id,turn_id,output_tokens,cost_microusd,cost_source,source,created_at) VALUES('w','c','t2',10,800000,'provider_reported','provider.claude','b');
             INSERT INTO usage_ledger(workspace_id,session_id,turn_id,output_tokens,cost_microusd,cost_source,source,created_at) VALUES('w','c','t3',10,1100000,'provider_reported','provider.claude','c');
             INSERT INTO usage_ledger(workspace_id,session_id,turn_id,output_tokens,cost_microusd,cost_source,source,created_at) VALUES('w','c','t4',10,200000,'provider_reported','provider.claude','d');
             -- Codex, old monotonic rows: the stored figures do not prove cumulative semantics.
             INSERT INTO usage_ledger(workspace_id,session_id,turn_id,input_tokens,uncached_input_tokens,output_tokens,source,created_at) VALUES('w','x-total','t',1000,1000,50,'provider.codex','2026-09-01T10:00:00+00:00');
             INSERT INTO usage_ledger(workspace_id,session_id,turn_id,input_tokens,uncached_input_tokens,output_tokens,source,created_at) VALUES('w','x-total','t',2500,2500,120,'provider.codex','2026-09-02T10:00:00+00:00');
             INSERT INTO usage_ledger(workspace_id,session_id,turn_id,input_tokens,uncached_input_tokens,output_tokens,source,created_at) VALUES('w','x-total','t',4000,4000,120,'provider.codex','2026-09-03T10:00:00+00:00');
             -- Codex, per-request without a cache figure: left alone too.
             INSERT INTO usage_ledger(workspace_id,session_id,turn_id,input_tokens,uncached_input_tokens,output_tokens,source,created_at) VALUES('w','x-last','t',1000,1000,300,'provider.codex','2026-09-01T10:00:00+00:00');
             INSERT INTO usage_ledger(workspace_id,session_id,turn_id,input_tokens,uncached_input_tokens,output_tokens,source,created_at) VALUES('w','x-last','t',1500,1500,90,'provider.codex','2026-09-02T10:00:00+00:00');
             INSERT INTO usage_ledger(workspace_id,session_id,turn_id,input_tokens,uncached_input_tokens,output_tokens,source,created_at) VALUES('w','x-last','t',2100,2100,400,'provider.codex','2026-09-03T10:00:00+00:00');
             -- Codex, per-request with a cache figure: the new normalizer, never touched even when monotonic.
             INSERT INTO usage_ledger(workspace_id,session_id,turn_id,input_tokens,uncached_input_tokens,cache_read_tokens,output_tokens,source,created_at) VALUES('w','x-new','t',1000,100,900,10,'provider.codex','2026-09-01T10:00:00+00:00');
             INSERT INTO usage_ledger(workspace_id,session_id,turn_id,input_tokens,uncached_input_tokens,cache_read_tokens,output_tokens,source,created_at) VALUES('w','x-new','t',2000,100,1900,20,'provider.codex','2026-09-02T10:00:00+00:00');
             INSERT INTO usage_ledger(workspace_id,session_id,turn_id,input_tokens,uncached_input_tokens,cache_read_tokens,output_tokens,source,created_at) VALUES('w','x-new','t',3000,100,2900,30,'provider.codex','2026-09-03T10:00:00+00:00');
             -- Codex, monotonic with no cache figure but written after the per-request normalizer:
             -- a genuine per-request session the repair must never rewrite.
             INSERT INTO usage_ledger(workspace_id,session_id,turn_id,input_tokens,uncached_input_tokens,output_tokens,source,created_at) VALUES('w','x-fresh','t',100,100,10,'provider.codex','2026-09-07T10:00:00+00:00');
             INSERT INTO usage_ledger(workspace_id,session_id,turn_id,input_tokens,uncached_input_tokens,output_tokens,source,created_at) VALUES('w','x-fresh','t',200,200,20,'provider.codex','2026-09-07T11:00:00+00:00');
             INSERT INTO usage_ledger(workspace_id,session_id,turn_id,input_tokens,uncached_input_tokens,output_tokens,source,created_at) VALUES('w','x-fresh','t',300,300,30,'provider.codex','2026-09-07T12:00:00+00:00');",
        )
        .unwrap();

        let transaction = db.transaction().unwrap();
        migration_54_usage_ledger_repair(&transaction).unwrap();
        transaction.commit().unwrap();

        let costs: Vec<i64> = usage_ledger(&db, "w", Some("c")).unwrap().iter().map(|row| row.cost_microusd.unwrap()).collect();
        assert_eq!(costs, vec![500_000, 300_000, 300_000, 200_000]);

        let inputs = |session: &str| -> Vec<(Option<i64>, Option<i64>, Option<i64>)> {
            usage_ledger(&db, "w", Some(session)).unwrap().iter().map(|row| (row.input_tokens, row.uncached_input_tokens, row.output_tokens)).collect()
        };
        // Even rows before the old timestamp cutoff can be per-request.
        // No persisted provenance proves these were cumulative counters.
        assert_eq!(inputs("x-total"), vec![(Some(1000), Some(1000), Some(50)), (Some(2500), Some(2500), Some(120)), (Some(4000), Some(4000), Some(120))]);
        assert_eq!(inputs("x-last"), vec![(Some(1000), Some(1000), Some(300)), (Some(1500), Some(1500), Some(90)), (Some(2100), Some(2100), Some(400))]);
        assert_eq!(inputs("x-new"), vec![(Some(1000), Some(100), Some(10)), (Some(2000), Some(100), Some(20)), (Some(3000), Some(100), Some(30))]);
        assert_eq!(inputs("x-fresh"), vec![(Some(100), Some(100), Some(10)), (Some(200), Some(200), Some(20)), (Some(300), Some(300), Some(30))]);
    }

    #[test]
    fn the_ledger_repair_skips_codex_sessions_with_unparsable_timestamps() {
        let mut db = open(Path::new(":memory:")).unwrap();
        db.execute_batch(
            "INSERT INTO projects(id,name,path,created_at) VALUES('p','Demo','/tmp/usage-repair-ts','now');
             INSERT INTO workspaces(id,project_id,city,title,branch,path,status,created_at) VALUES('w','p','c','t','main','/tmp/usage-repair-ts/w','ready','now');
             INSERT INTO sessions(id,workspace_id,harness,label,status) VALUES('x-bad-ts','w','codex','x','ready');
             INSERT INTO usage_ledger(workspace_id,session_id,turn_id,input_tokens,uncached_input_tokens,output_tokens,source,created_at) VALUES('w','x-bad-ts','t',100,100,10,'provider.codex','not-a-timestamp');
             INSERT INTO usage_ledger(workspace_id,session_id,turn_id,input_tokens,uncached_input_tokens,output_tokens,source,created_at) VALUES('w','x-bad-ts','t',200,200,20,'provider.codex','also-not-a-timestamp');
             INSERT INTO usage_ledger(workspace_id,session_id,turn_id,input_tokens,uncached_input_tokens,output_tokens,source,created_at) VALUES('w','x-bad-ts','t',300,300,30,'provider.codex','still-not-a-timestamp');",
        )
        .unwrap();

        let transaction = db.transaction().unwrap();
        migration_54_usage_ledger_repair(&transaction).unwrap();
        transaction.commit().unwrap();

        let rows = usage_ledger(&db, "w", Some("x-bad-ts")).unwrap();
        let inputs: Vec<Option<i64>> = rows.iter().map(|row| row.input_tokens).collect();
        assert_eq!(inputs, vec![Some(100), Some(200), Some(300)], "unprovable provenance is left alone");
    }

    #[test]
    fn the_usage_tracking_migration_is_additive_and_idempotent() {
        let mut db = open(Path::new(":memory:")).unwrap();
        db.execute_batch(
            "INSERT INTO projects(id,name,path,created_at) VALUES('p','Demo','/tmp/usage-migration','now');
             INSERT INTO workspaces(id,project_id,city,title,branch,path,status,created_at) VALUES('w','p','c','t','main','/tmp/usage-migration/w','ready','now');
             INSERT INTO sessions(id,workspace_id,harness,label,status) VALUES('s','w','codex','s','ready');",
        )
        .unwrap();
        // A row written with the pre-migration column set only.
        db.execute(
            "INSERT INTO usage_ledger(workspace_id,session_id,turn_id,input_tokens,output_tokens,cache_read_tokens,uncached_input_tokens,cost_microusd,cost_source,source,created_at)
             VALUES('w','s','t',100,10,40,60,250,'provider_reported','provider.codex','then')",
            [],
        )
        .unwrap();

        // Running the migration again is a no-op rather than an error.
        let transaction = db.transaction().unwrap();
        migration_53_usage_tracking(&transaction).unwrap();
        transaction.commit().unwrap();

        let rows = usage_ledger(&db, "w", Some("s")).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].input_tokens, Some(100));
        assert_eq!(rows[0].cache_read_tokens, Some(40));
        assert_eq!(rows[0].uncached_input_tokens, Some(60));
        assert_eq!(rows[0].cost_microusd, Some(250));
        assert_eq!(rows[0].cost_source.as_deref(), Some("provider_reported"));
        for missing in [
            rows[0].reasoning_tokens,
            rows[0].context_window_tokens,
            rows[0].context_used_tokens,
            rows[0].cache_savings_microusd,
        ] {
            assert_eq!(missing, None, "a pre-migration row gains no invented figure");
        }
        assert_eq!(rows[0].serving_model, None);
        assert_eq!(rows[0].provider_record_id, None);

        for table in ["usage_price_overrides", "usage_rate_cache"] {
            let exists: bool = db
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
                    params![table],
                    |row| row.get(0),
                )
                .unwrap();
            assert!(exists, "{table} is missing after migration");
        }
        // The rate cache holds one row by construction.
        db.execute(
            "INSERT INTO usage_rate_cache(id,fetched_at,source_url,body) VALUES(1,'now','url','{}')",
            [],
        )
        .unwrap();
        assert!(db
            .execute(
                "INSERT INTO usage_rate_cache(id,fetched_at,source_url,body) VALUES(2,'now','url','{}')",
                [],
            )
            .is_err());
    }

    #[test]
    fn opening_a_database_creates_the_analytics_schema_with_dedupe_indexes() {
        let dir = tempfile::tempdir().unwrap();
        let db = open(&dir.path().join("bridge.db")).unwrap();

        let version: i64 = db
            .query_row("SELECT MAX(version) FROM schema_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, LATEST_SCHEMA_VERSION);

        for table in [
            "agent_usage_sources",
            "agent_usage_sessions",
            "agent_usage_observations",
            "agent_usage_attributions",
        ] {
            let exists: bool = db
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
                    params![table],
                    |row| row.get(0),
                )
                .unwrap();
            assert!(exists, "{table} is missing after migration");
        }

        // Dedupe constraints: the same native record cannot be indexed twice,
        // and a source location is admitted once under one fingerprint.
        db.execute_batch(
            "INSERT INTO agent_usage_sources(id,agent,provider,location_fingerprint,coverage_state,importer_version,created_at,updated_at)
                 VALUES('s1','codex','openai','sha256:loc','empty','test','now','now');
             INSERT INTO agent_usage_observations(id,source_id,native_record_id,occurred_at,input_semantics,output_semantics,exact_total_formula,importer_version,created_at)
                 VALUES('o1','s1','rec-1','t','inclusive','delta','provider_reported','test','now');",
        )
        .unwrap();
        let duplicate_record = db.execute(
            "INSERT INTO agent_usage_observations(id,source_id,native_record_id,occurred_at,input_semantics,output_semantics,exact_total_formula,importer_version,created_at)
                 VALUES('o2','s1','rec-1','t','inclusive','delta','provider_reported','test','now');",
            [],
        );
        assert!(
            duplicate_record.is_err(),
            "a duplicate (source_id, native_record_id) was accepted"
        );
        let duplicate_location = db.execute(
            "INSERT INTO agent_usage_sources(id,agent,provider,location_fingerprint,coverage_state,importer_version,created_at,updated_at)
                 VALUES('s2','claude','anthropic','sha256:loc','empty','test','now','now');",
            [],
        );
        assert!(
            duplicate_location.is_err(),
            "a duplicate location fingerprint was accepted"
        );
    }

    #[test]
    fn workspace_deletion_leaves_whole_mac_analytics_indexed() {
        let dir = tempfile::tempdir().unwrap();
        let db = open(&dir.path().join("bridge.db")).unwrap();
        db.execute_batch(
            "INSERT INTO projects(id,name,path,created_at) VALUES('p','Demo','/tmp/demo','now');
             INSERT INTO workspaces(id,project_id,city,title,branch,path,status,created_at)
                 VALUES('w','p','Kyoto','Task','bridge/task',NULL,'idle','now');
             INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source)
                 VALUES('ws-session','w','codex','Codex','idle','reported');
             INSERT INTO usage_ledger(session_id,workspace_id,source,created_at)
                 VALUES('ws-session','w','test','now');
             INSERT INTO agent_usage_sources(id,agent,provider,location_fingerprint,coverage_state,importer_version,created_at,updated_at)
                 VALUES('src','codex','openai','sha256:loc','complete','test','now','now');
             INSERT INTO agent_usage_observations(id,source_id,native_record_id,occurred_at,input_semantics,output_semantics,exact_total_formula,importer_version,created_at)
                 VALUES('obs','src','rec-1','t','inclusive','delta','provider_reported','test','now');",
        )
        .unwrap();

        // The same cleanup a workspace deletion performs on the legacy ledger:
        // usage rows and their sessions are workspace-owned and go with it.
        let tx = db.unchecked_transaction().unwrap();
        tx.execute("DELETE FROM usage_ledger WHERE workspace_id='w'", [])
            .unwrap();
        tx.execute("DELETE FROM sessions WHERE workspace_id='w'", [])
            .unwrap();
        tx.execute("DELETE FROM workspaces WHERE id='w'", []).unwrap();
        tx.commit().unwrap();

        let ledger_rows: i64 = db
            .query_row("SELECT COUNT(*) FROM usage_ledger", [], |row| row.get(0))
            .unwrap();
        assert_eq!(ledger_rows, 0, "legacy ledger rows follow their workspace");

        let analytics_rows: i64 = db
            .query_row("SELECT COUNT(*) FROM agent_usage_observations", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(
            analytics_rows, 1,
            "device-wide analytics never belong to a workspace and must survive deletion"
        );
    }

    #[test]
    fn agent_usage_observation_unique_constraint_holds_under_insert_or_ignore() {
        let dir = tempfile::tempdir().unwrap();
        let db = open(&dir.path().join("bridge.db")).unwrap();
        db.execute_batch(
            "INSERT INTO agent_usage_sources(id,agent,provider,location_fingerprint,coverage_state,importer_version,created_at,updated_at)
                 VALUES('s1','claude','anthropic','sha256:loc','partial','test','now','now');",
        )
        .unwrap();
        let insert = |id: &str, native: &str, tokens: i64| -> usize {
            db.execute(
                "INSERT OR IGNORE INTO agent_usage_observations(id,source_id,native_record_id,occurred_at,input_semantics,output_semantics,exact_total_formula,output_tokens,importer_version,created_at)
                 VALUES(?1,'s1',?2,'t','exclusive','delta','anthropic_exclusive_input_plus_cache_and_output',?3,'test','now')",
                params![id, native, tokens],
            )
            .unwrap()
        };
        assert_eq!(insert("o1", "msg_1:req_1", 10), 1);
        // Same native record under a different row id: refused, silently.
        assert_eq!(insert("o2", "msg_1:req_1", 999), 0);
        // A different source may hold the same native id.
        db.execute_batch(
            "INSERT INTO agent_usage_sources(id,agent,provider,location_fingerprint,coverage_state,importer_version,created_at,updated_at)
                 VALUES('s2','claude','anthropic','sha256:other','partial','test','now','now');
             INSERT INTO agent_usage_observations(id,source_id,native_record_id,occurred_at,input_semantics,output_semantics,exact_total_formula,importer_version,created_at)
                 VALUES('o3','s2','msg_1:req_1','t','exclusive','delta','anthropic_exclusive_input_plus_cache_and_output','test','now');",
        )
        .unwrap();
        let (rows, kept): (i64, i64) = db
            .query_row(
                "SELECT COUNT(*), (SELECT output_tokens FROM agent_usage_observations WHERE id='o1') FROM agent_usage_observations",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(rows, 2);
        assert_eq!(kept, 10, "the first observation wins");
    }
}
