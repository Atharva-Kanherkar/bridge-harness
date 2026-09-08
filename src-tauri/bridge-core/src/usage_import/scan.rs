//! Incremental scanning and persistence shared by every importer.
//!
//! JSONL sources are walked file by file. Each file's cursor is the byte offset
//! just past the last newline-terminated line consumed, a small hash of the
//! bytes immediately before that offset, and the size and mtime seen at the
//! time. An unchanged file is skipped on the stat alone; an appended file
//! resumes from its offset when the guard still matches; anything else —
//! truncation, rotation, an in-place rewrite — fails the guard and the file is
//! read from the start, where `INSERT OR IGNORE` on `(source_id,
//! native_record_id)` keeps the rescan idempotent.

use super::{
    claude, codex, cursor, opencode, sha256_hex, source_id_for, ParsedUsage, SourceEnv,
    IMPORTER_VERSION,
};
use crate::analytics::{
    AgentUsageObservation, AnalyticsScanRequest, AnalyticsScanResult, CoverageState,
    DiscoveredAnalyticsSource, ImporterCapability,
};
use crate::BridgeError;
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// 64 bytes of JSONL tail is ample to tell a replaced file from an appended one.
pub const GUARD_LENGTH: u64 = 64;

/// Where a parse of one transcript stopped.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FileCursor {
    /// Byte offset just past the last newline-terminated line consumed.
    pub offset: u64,
    /// Number of lines consumed so far; Codex record ids are line-addressed.
    pub line_no: u64,
    /// FNV-1a 64 over the `min(64, offset)` bytes ending at `offset`, hex.
    pub guard_hash: String,
    /// Size and mtime observed when the file was last read to its end. Only a
    /// cursor that reached the end can short-circuit a later pass on stat.
    pub size: u64,
    pub mtime_ms: i64,
    #[serde(default)]
    pub complete: bool,
    /// Codex reducer state as of `offset`; absent for stateless parsers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codex: Option<codex::CodexScanState>,
}

/// The JSON stored in `agent_usage_sources.scan_cursor` for JSONL sources,
/// keyed by path relative to the source root.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct JsonlCursor {
    pub files: BTreeMap<String, FileCursor>,
}

impl JsonlCursor {
    pub fn parse(text: Option<&str>) -> Self {
        text.and_then(|value| serde_json::from_str(value).ok())
            .unwrap_or_default()
    }

    pub fn to_json(&self) -> Result<String, BridgeError> {
        serde_json::to_string(self).map_err(|error| BridgeError::Invalid(error.to_string()))
    }
}

pub(crate) fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn guard_hash_at(file: &mut File, offset: u64) -> Result<String, BridgeError> {
    let length = offset.min(GUARD_LENGTH);
    if length == 0 {
        return Ok(String::new());
    }
    let mut window = vec![0_u8; length as usize];
    file.seek(SeekFrom::Start(offset - length))?;
    file.read_exact(&mut window)?;
    Ok(format!("{:016x}", fnv1a64(&window)))
}

fn guard_matches(file: &mut File, cursor: &FileCursor, size: u64) -> bool {
    if cursor.offset == 0 || cursor.offset > size || cursor.guard_hash.is_empty() {
        return false;
    }
    matches!(guard_hash_at(file, cursor.offset), Ok(hash) if hash == cursor.guard_hash)
}

/// What a line parser tells the scanner about one line.
#[derive(Debug)]
pub(crate) enum LineOutcome {
    Record(ParsedUsage),
    /// A usage-bearing line that was deliberately dropped (duplicate, no
    /// model, synthetic, fork copy). Counted as skipped.
    Skipped,
    /// Not a usage line at all.
    Ignored,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum JsonlKind {
    Claude,
    Codex,
}

/// Lists `.jsonl` files beneath `root` with their relative path, size and
/// mtime, sorted by relative path so batches are deterministic. Entries that
/// vanish mid-walk are skipped; a partial listing beats a failed scan.
pub(crate) fn list_jsonl_files(root: &Path) -> Vec<(String, PathBuf, u64, i64)> {
    let mut found = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().and_then(|ext| ext.to_str()) != Some("jsonl") {
                continue;
            }
            let Ok(meta) = entry.metadata() else { continue };
            let mtime_ms = meta
                .modified()
                .ok()
                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
                .unwrap_or(0);
            let relative = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .into_owned();
            found.push((relative, path, meta.len(), mtime_ms));
        }
    }
    found.sort_by(|a, b| a.0.cmp(&b.0));
    found
}

struct FileScan {
    cursor: FileCursor,
    records: Vec<ParsedUsage>,
    skipped: usize,
    /// True when the record budget stopped the read before end of file.
    truncated: bool,
}

/// Reads one transcript from its cursor (or from the start when the guard
/// fails) until end of file or until `budget` records are collected.
fn scan_jsonl_file(
    path: &Path,
    kind: JsonlKind,
    previous: Option<&FileCursor>,
    size: u64,
    mtime_ms: i64,
    file_fingerprint: &str,
    budget: usize,
) -> Result<FileScan, BridgeError> {
    let mut file = File::open(path)?;
    let file_stem = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut offset = 0_u64;
    let mut line_no = 0_u64;
    let mut codex_state = codex::CodexScanState::default();
    if let Some(previous) = previous {
        let resumable = match kind {
            JsonlKind::Codex => previous.codex.is_some(),
            JsonlKind::Claude => true,
        };
        if resumable && guard_matches(&mut file, previous, size) {
            offset = previous.offset;
            line_no = previous.line_no;
            if let Some(state) = &previous.codex {
                codex_state = state.clone();
            }
        }
    }
    file.seek(SeekFrom::Start(offset))?;
    let mut reader = BufReader::with_capacity(256 * 1024, file);
    let mut records = Vec::new();
    let mut skipped = 0_usize;
    let mut truncated = false;
    let mut line = Vec::new();
    loop {
        if records.len() >= budget {
            truncated = true;
            break;
        }
        line.clear();
        let read = reader.read_until(b'\n', &mut line)?;
        if read == 0 {
            break;
        }
        if line.last() != Some(&b'\n') {
            // A writer may still be appending to this line; leave it for the
            // next pass so a half record is never counted twice.
            break;
        }
        offset += read as u64;
        line_no += 1;
        let text = line.strip_suffix(b"\n").unwrap_or(&line);
        let text = text.strip_suffix(b"\r").unwrap_or(text);
        let Ok(text) = std::str::from_utf8(text) else {
            continue;
        };
        let outcome = match kind {
            JsonlKind::Claude => claude::parse_line(text),
            JsonlKind::Codex => codex::parse_line(
                text,
                &mut codex_state,
                file_fingerprint,
                &file_stem,
                line_no,
            ),
        };
        match outcome {
            LineOutcome::Record(record) => records.push(record),
            LineOutcome::Skipped => skipped += 1,
            LineOutcome::Ignored => {}
        }
    }
    let mut file = reader.into_inner();
    let guard_hash = guard_hash_at(&mut file, offset)?;
    Ok(FileScan {
        cursor: FileCursor {
            offset,
            line_no,
            guard_hash,
            size,
            mtime_ms,
            complete: !truncated,
            codex: (kind == JsonlKind::Codex).then_some(codex_state),
        },
        records,
        skipped,
        truncated,
    })
}

/// Dispatches one bounded batch for whatever source the request names.
pub(crate) fn run(
    db: &Connection,
    request: &AnalyticsScanRequest,
    env: &SourceEnv,
) -> Result<AnalyticsScanResult, BridgeError> {
    let source = &request.source;
    let source_id = source_id_for(source);
    let tx = db.unchecked_transaction()?;
    let stored_cursor = ensure_source_row(&tx, &source_id, source)?;
    if source.capability == ImporterCapability::Unsupported {
        let reason = source
            .reason
            .clone()
            .or_else(|| Some(cursor::UNSUPPORTED_REASON.to_string()));
        finish_source(
            &tx,
            &source_id,
            None,
            CoverageState::Unsupported,
            reason.as_deref(),
            None,
        )?;
        tx.commit()?;
        return Ok(AnalyticsScanResult {
            observations: vec![],
            attributions: vec![],
            next_cursor: None,
            records_imported: 0,
            records_skipped: 0,
            coverage: CoverageState::Unsupported,
            warning: reason,
        });
    }
    let cursor_text = request.cursor.clone().or(stored_cursor);
    let result = match source.agent.as_str() {
        "claude" => scan_jsonl_source(
            &tx,
            &source_id,
            source,
            JsonlKind::Claude,
            cursor_text.as_deref(),
            request.max_records,
        ),
        "codex" => scan_jsonl_source(
            &tx,
            &source_id,
            source,
            JsonlKind::Codex,
            cursor_text.as_deref(),
            request.max_records,
        ),
        "opencode" => opencode::scan(
            &tx,
            &source_id,
            source,
            cursor_text.as_deref(),
            request.max_records,
            env,
        ),
        other => Err(BridgeError::Invalid(format!(
            "no usage importer for agent {other:?}"
        ))),
    };
    match result {
        Ok(result) => {
            tx.commit()?;
            result.validate_for(request)?;
            Ok(result)
        }
        Err(error) => {
            // Record the failure on the source row and keep the cursor.
            let _ = finish_source(
                &tx,
                &source_id,
                cursor_text.as_deref(),
                CoverageState::Unreadable,
                Some(&error.to_string()),
                None,
            );
            let _ = tx.commit();
            Err(error)
        }
    }
}

fn scan_jsonl_source(
    tx: &Transaction<'_>,
    source_id: &str,
    source: &DiscoveredAnalyticsSource,
    kind: JsonlKind,
    cursor_text: Option<&str>,
    max_records: usize,
) -> Result<AnalyticsScanResult, BridgeError> {
    let root = &source.location;
    let mut cursor = JsonlCursor::parse(cursor_text);
    let files = list_jsonl_files(root);
    let present: HashSet<&str> = files.iter().map(|f| f.0.as_str()).collect();
    cursor
        .files
        .retain(|relative, _| present.contains(relative.as_str()));

    let mut records: Vec<(ParsedUsage, String)> = Vec::new();
    let mut skipped = 0_usize;
    let mut truncated = false;
    let mut seen_ids: HashSet<String> = HashSet::new();
    for (relative, path, size, mtime_ms) in &files {
        let previous = cursor.files.get(relative);
        if let Some(previous) = previous {
            if previous.complete && previous.size == *size && previous.mtime_ms == *mtime_ms {
                continue;
            }
        }
        let budget = max_records - records.len();
        if budget == 0 {
            truncated = true;
            break;
        }
        let file_fingerprint = sha256_hex(relative);
        let scan = match scan_jsonl_file(
            path,
            kind,
            previous,
            *size,
            *mtime_ms,
            &file_fingerprint,
            budget,
        ) {
            Ok(scan) => scan,
            // A file that vanished or cannot be opened is left for the next
            // pass; its cursor entry is dropped so it is read fresh then.
            Err(_) => {
                cursor.files.remove(relative);
                continue;
            }
        };
        skipped += scan.skipped;
        for record in scan.records {
            if !seen_ids.insert(record.native_record_id.clone()) {
                skipped += 1;
                continue;
            }
            records.push((record, file_fingerprint.clone()));
        }
        cursor.files.insert(relative.clone(), scan.cursor);
        if scan.truncated {
            truncated = true;
            break;
        }
    }

    let persisted = persist_records(tx, source_id, &source.agent, records)?;
    skipped += persisted.ignored;
    let coverage = if truncated {
        CoverageState::Partial
    } else if persisted.observations.is_empty() && files.is_empty() {
        CoverageState::Empty
    } else {
        CoverageState::Complete
    };
    let cursor_json = cursor.to_json()?;
    finish_source(
        tx,
        source_id,
        Some(&cursor_json),
        coverage,
        None,
        Some((persisted.observations.len(), skipped)),
    )?;
    Ok(AnalyticsScanResult {
        records_imported: persisted.observations.len(),
        observations: persisted.observations,
        attributions: vec![],
        next_cursor: Some(cursor_json),
        records_skipped: skipped,
        coverage,
        warning: None,
    })
}

pub(crate) struct Persisted {
    pub observations: Vec<AgentUsageObservation>,
    /// Rows the unique constraint refused: already imported on an earlier pass.
    pub ignored: usize,
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// Inserts or refreshes the source row and returns its stored cursor.
pub(crate) fn ensure_source_row(
    tx: &Transaction<'_>,
    source_id: &str,
    source: &DiscoveredAnalyticsSource,
) -> Result<Option<String>, BridgeError> {
    let now = now();
    tx.execute(
        "INSERT INTO agent_usage_sources(id,agent,provider,location_fingerprint,detected_version,coverage_state,coverage_reason,importer_version,created_at,updated_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?9)
         ON CONFLICT(location_fingerprint) DO UPDATE SET
            detected_version=COALESCE(excluded.detected_version, agent_usage_sources.detected_version),
            updated_at=excluded.updated_at",
        params![
            source_id,
            source.agent,
            source.provider,
            source.location_fingerprint,
            source.detected_version,
            source.coverage.as_str(),
            source.reason,
            IMPORTER_VERSION,
            now,
        ],
    )?;
    let cursor: Option<String> = tx
        .query_row(
            "SELECT scan_cursor FROM agent_usage_sources WHERE id=?1",
            params![source_id],
            |row| row.get(0),
        )
        .optional()?
        .flatten();
    Ok(cursor)
}

/// Writes the batch outcome onto the source row: cursor, coverage, counters
/// and the coverage window widened by what was just imported.
pub(crate) fn finish_source(
    tx: &Transaction<'_>,
    source_id: &str,
    cursor_json: Option<&str>,
    coverage: CoverageState,
    reason: Option<&str>,
    counts: Option<(usize, usize)>,
) -> Result<(), BridgeError> {
    let now = now();
    let (imported, skipped) = counts.unwrap_or((0, 0));
    let failed = matches!(coverage, CoverageState::Unreadable);
    tx.execute(
        "UPDATE agent_usage_sources SET
            scan_cursor=COALESCE(?2, scan_cursor),
            coverage_state=?3,
            coverage_reason=?4,
            records_imported=records_imported+?5,
            records_skipped=records_skipped+?6,
            last_successful_scan_at=CASE WHEN ?7 THEN last_successful_scan_at ELSE ?8 END,
            last_error=CASE WHEN ?7 THEN ?4 ELSE NULL END,
            importer_version=?9,
            coverage_start_at=(SELECT MIN(occurred_at) FROM agent_usage_observations WHERE source_id=?1),
            coverage_end_at=(SELECT MAX(occurred_at) FROM agent_usage_observations WHERE source_id=?1),
            updated_at=?8
         WHERE id=?1",
        params![
            source_id,
            cursor_json,
            coverage.as_str(),
            reason,
            imported as i64,
            skipped as i64,
            failed,
            now,
            IMPORTER_VERSION,
        ],
    )?;
    Ok(())
}

fn session_row_id(source_id: &str, native_session_id: &str) -> String {
    sha256_hex(&format!("{source_id}|session|{native_session_id}"))[..32].to_string()
}

fn observation_row_id(source_id: &str, native_record_id: &str) -> String {
    sha256_hex(&format!("{source_id}|record|{native_record_id}"))[..32].to_string()
}

fn project_label(path: &str) -> Option<String> {
    Path::new(path)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
}

/// Persists a batch: session rows first, then observations with
/// `INSERT OR IGNORE` so a rescan of a rewritten file cannot double count.
pub(crate) fn persist_records(
    tx: &Transaction<'_>,
    source_id: &str,
    agent: &str,
    records: Vec<(ParsedUsage, String)>,
) -> Result<Persisted, BridgeError> {
    let now = now();
    let mut observations = Vec::with_capacity(records.len());
    let mut ignored = 0_usize;
    for (record, file_fingerprint) in records {
        let session_row = match &record.native_session_id {
            Some(native_session_id) => {
                let id = session_row_id(source_id, native_session_id);
                tx.execute(
                    "INSERT INTO agent_usage_sessions(id,source_id,native_session_id,parent_native_session_id,agent,provider,model,project_label,project_path_fingerprint,session_type,started_at,ended_at,created_at,updated_at)
                     VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?11,?12,?12)
                     ON CONFLICT(source_id,native_session_id) DO UPDATE SET
                        parent_native_session_id=COALESCE(agent_usage_sessions.parent_native_session_id, excluded.parent_native_session_id),
                        model=COALESCE(excluded.model, agent_usage_sessions.model),
                        project_label=COALESCE(agent_usage_sessions.project_label, excluded.project_label),
                        project_path_fingerprint=COALESCE(agent_usage_sessions.project_path_fingerprint, excluded.project_path_fingerprint),
                        session_type=COALESCE(agent_usage_sessions.session_type, excluded.session_type),
                        started_at=CASE WHEN agent_usage_sessions.started_at IS NULL OR excluded.started_at < agent_usage_sessions.started_at THEN excluded.started_at ELSE agent_usage_sessions.started_at END,
                        ended_at=CASE WHEN agent_usage_sessions.ended_at IS NULL OR excluded.ended_at > agent_usage_sessions.ended_at THEN excluded.ended_at ELSE agent_usage_sessions.ended_at END,
                        updated_at=excluded.updated_at",
                    params![
                        id,
                        source_id,
                        native_session_id,
                        record.parent_native_session_id,
                        agent,
                        record.provider,
                        record.model,
                        record.project_path.as_deref().and_then(project_label),
                        record.project_path.as_deref().map(sha256_hex),
                        record.session_type,
                        record.occurred_at,
                        now,
                    ],
                )?;
                Some(id)
            }
            None => None,
        };
        let observation = AgentUsageObservation {
            source_id: source_id.to_string(),
            native_record_id: record.native_record_id.clone(),
            native_session_id: record.native_session_id.clone(),
            occurred_at: record.occurred_at.clone(),
            model: record.model.clone(),
            input_semantics: record.input_semantics.to_string(),
            output_semantics: record.output_semantics.to_string(),
            usage: record.usage.clone(),
            numeric_usage: record.numeric_usage.clone(),
            importer_version: IMPORTER_VERSION.to_string(),
        };
        observation.validate()?;
        let cost_source = record.reported_cost_microusd.map(|_| "provider_reported");
        let changed = tx.execute(
            "INSERT OR IGNORE INTO agent_usage_observations(
                id,source_id,session_id,native_record_id,occurred_at,model,input_semantics,output_semantics,
                total_input_tokens,uncached_input_tokens,cache_read_tokens,cache_write_tokens,output_tokens,reasoning_tokens,tool_use_tokens,
                provider_reported_total_tokens,exact_total_formula,reported_cost_microusd,cost_source,
                numeric_usage_json,source_file_fingerprint,importer_version,created_at)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23)",
            params![
                observation_row_id(source_id, &record.native_record_id),
                source_id,
                session_row,
                record.native_record_id,
                record.occurred_at,
                record.model,
                record.input_semantics,
                record.output_semantics,
                record.usage.total_input_tokens,
                record.usage.uncached_input_tokens,
                record.usage.cache_read_tokens,
                record.usage.cache_write_tokens,
                record.usage.output_tokens,
                record.usage.reasoning_tokens,
                record.usage.tool_use_tokens,
                record.usage.provider_reported_total_tokens,
                record.usage.exact_total_formula.as_str(),
                record.reported_cost_microusd,
                cost_source,
                record.numeric_usage.to_json()?,
                file_fingerprint,
                IMPORTER_VERSION,
                now,
            ],
        )?;
        if changed == 0 {
            ignored += 1;
        } else {
            observations.push(observation);
        }
    }
    Ok(Persisted {
        observations,
        ignored,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fnv1a_is_stable() {
        assert_eq!(fnv1a64(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a64(b"a"), 0xaf63_dc4c_8601_ec8c);
    }

    #[test]
    fn cursor_round_trips_and_tolerates_garbage() {
        let mut cursor = JsonlCursor::default();
        cursor.files.insert(
            "a/b.jsonl".into(),
            FileCursor {
                offset: 10,
                line_no: 2,
                guard_hash: "abc".into(),
                size: 10,
                mtime_ms: 5,
                complete: true,
                codex: None,
            },
        );
        let json = cursor.to_json().unwrap();
        assert_eq!(JsonlCursor::parse(Some(&json)), cursor);
        assert_eq!(JsonlCursor::parse(Some("not json")), JsonlCursor::default());
        assert_eq!(JsonlCursor::parse(None), JsonlCursor::default());
    }
}
