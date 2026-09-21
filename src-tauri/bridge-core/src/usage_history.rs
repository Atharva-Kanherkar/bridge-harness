//! The `usage/list_history_sources` and `usage/scan_history` bodies: what the
//! history importers can see on this machine, joined to what they have
//! already indexed, and one bounded incremental pass over them.

use crate::analytics::{AnalyticsScanRequest, CoverageState, ImporterCapability};
use crate::usage_import::{self, ScanReport, SourceEnv, SourceScanOutcome};
use crate::{BridgeCore, BridgeError};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::time::Instant;

/// The most records one scan call will import per source.
pub const MAX_RECORDS_PER_SCAN: usize = 10_000;

/// A discovered source with the importer's standing on it. Discovery says
/// what is on disk and whether the importer understands it; the
/// `agent_usage_sources` row, when one exists, says what has been indexed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageHistorySource {
    pub id: String,
    #[serde(default)]
    pub origin: bridge_protocol::messages::UsageHistoryOrigin,
    pub agent: String,
    pub provider: String,
    pub location: String,
    pub detected_version: Option<String>,
    pub capability: ImporterCapability,
    pub coverage_state: CoverageState,
    pub coverage_reason: Option<String>,
    pub coverage_start_at: Option<String>,
    pub coverage_end_at: Option<String>,
    pub records_imported: i64,
    pub records_skipped: i64,
    pub last_successful_scan_at: Option<String>,
    pub last_error: Option<String>,
}

struct IndexedSource {
    coverage_state: Option<CoverageState>,
    coverage_reason: Option<String>,
    coverage_start_at: Option<String>,
    coverage_end_at: Option<String>,
    records_imported: i64,
    records_skipped: i64,
    last_successful_scan_at: Option<String>,
    last_error: Option<String>,
}

fn indexed_source(db: &Connection, id: &str) -> Result<Option<IndexedSource>, BridgeError> {
    Ok(db
        .query_row(
            "SELECT coverage_state,coverage_reason,coverage_start_at,coverage_end_at,records_imported,records_skipped,last_successful_scan_at,last_error
             FROM agent_usage_sources WHERE id=?1",
            params![id],
            |row| {
                let state: String = row.get(0)?;
                Ok(IndexedSource {
                    coverage_state: serde_json::from_value(serde_json::Value::String(state)).ok(),
                    coverage_reason: row.get(1)?,
                    coverage_start_at: row.get(2)?,
                    coverage_end_at: row.get(3)?,
                    records_imported: row.get(4)?,
                    records_skipped: row.get(5)?,
                    last_successful_scan_at: row.get(6)?,
                    last_error: row.get(7)?,
                })
            },
        )
        .optional()?)
}

/// Every source the importers know how to look for, present or not.
pub fn list_history_sources(
    db: &Connection,
    env: &SourceEnv,
) -> Result<Vec<UsageHistorySource>, BridgeError> {
    let mut sources = Vec::new();
    for discovered in usage_import::discover_sources(env) {
        let id = usage_import::source_id_for(&discovered);
        let indexed = indexed_source(db, &id)?;
        let mut source = UsageHistorySource {
            id,
            origin: Default::default(),
            agent: discovered.agent.clone(),
            provider: discovered.provider.clone(),
            location: discovered.location.to_string_lossy().into_owned(),
            detected_version: discovered.detected_version.clone(),
            capability: discovered.capability,
            coverage_state: discovered.coverage,
            coverage_reason: discovered.reason.clone(),
            coverage_start_at: None,
            coverage_end_at: None,
            records_imported: 0,
            records_skipped: 0,
            last_successful_scan_at: None,
            last_error: None,
        };
        if let Some(indexed) = indexed {
            // A source the disk no longer holds stays `empty`/`unsupported` as
            // discovered; otherwise the last scan knows the coverage best.
            if discovered.capability == ImporterCapability::Supported
                && discovered.coverage != CoverageState::Empty
            {
                if let Some(state) = indexed.coverage_state {
                    source.coverage_state = state;
                }
                source.coverage_reason = indexed.coverage_reason.or(source.coverage_reason);
            }
            source.coverage_start_at = indexed.coverage_start_at;
            source.coverage_end_at = indexed.coverage_end_at;
            source.records_imported = indexed.records_imported;
            source.records_skipped = indexed.records_skipped;
            source.last_successful_scan_at = indexed.last_successful_scan_at;
            source.last_error = indexed.last_error;
        }
        sources.push(source);
    }
    Ok(sources)
}

/// One bounded pass over the chosen sources (all of them when `source_ids`
/// is `None`). Discovery runs without the database lock; the lock is held
/// only for each source's own scan so other callers are not stalled behind
/// a large transcript directory.
pub fn scan_history(
    core: &BridgeCore,
    env: &SourceEnv,
    max_records: Option<usize>,
    source_ids: Option<&[String]>,
) -> Result<ScanReport, BridgeError> {
    let started = Instant::now();
    let max_records = max_records.unwrap_or(MAX_RECORDS_PER_SCAN).clamp(1, MAX_RECORDS_PER_SCAN);
    let discovered = usage_import::discover_sources(env);
    let selected: Vec<_> = match source_ids {
        None => discovered,
        Some(ids) => {
            let wanted: Vec<&str> = ids.iter().map(String::as_str).collect();
            let selected: Vec<_> = discovered
                .into_iter()
                .filter(|source| wanted.contains(&usage_import::source_id_for(source).as_str()))
                .collect();
            if selected.len() != wanted.len() {
                let known: Vec<String> = selected.iter().map(usage_import::source_id_for).collect();
                let unknown: Vec<&str> = wanted
                    .iter()
                    .copied()
                    .filter(|id| !known.iter().any(|k| k == id))
                    .collect();
                return Err(BridgeError::Invalid(format!(
                    "unknown usage history source ids: {}",
                    unknown.join(", ")
                )));
            }
            selected
        }
    };
    let mut report = ScanReport {
        sources: Vec::new(),
        records_imported: 0,
        records_skipped: 0,
        duration_ms: 0,
    };
    for source in selected {
        let mut outcome = SourceScanOutcome {
            source_id: usage_import::source_id_for(&source),
            agent: source.agent.clone(),
            provider: source.provider.clone(),
            location: source.location.to_string_lossy().into_owned(),
            capability: source.capability,
            coverage: source.coverage,
            records_imported: 0,
            records_skipped: 0,
            next_cursor: None,
            warning: source.reason.clone(),
        };
        if source.capability == ImporterCapability::Supported
            && source.coverage != CoverageState::Empty
        {
            let request = AnalyticsScanRequest {
                source: source.clone(),
                cursor: None,
                max_records,
            };
            let result = {
                let db = core.db.lock().unwrap();
                usage_import::scan(&db, &request, env)
            };
            match result {
                Ok(result) => {
                    outcome.coverage = result.coverage;
                    outcome.records_imported = result.records_imported;
                    outcome.records_skipped = result.records_skipped;
                    outcome.next_cursor = result.next_cursor;
                    outcome.warning = result.warning;
                }
                Err(error) => {
                    outcome.coverage = CoverageState::Unreadable;
                    outcome.warning = Some(error.to_string());
                }
            }
        }
        report.records_imported += outcome.records_imported;
        report.records_skipped += outcome.records_skipped;
        report.sources.push(outcome);
    }
    report.duration_ms = i64::try_from(started.elapsed().as_millis()).unwrap_or(i64::MAX);
    Ok(report)
}
