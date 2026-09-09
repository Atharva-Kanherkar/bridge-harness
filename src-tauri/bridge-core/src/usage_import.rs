//! History importers for usage the machine's agent CLIs already recorded.
//!
//! Claude Code and Codex keep append-only JSONL transcripts, OpenCode keeps a
//! SQLite database, and Cursor keeps stores that hold no token counts at all.
//! Each importer reads exact, provider-reported numbers out of those files and
//! nothing else: no prompt, no completion, no tool output ever crosses into
//! `agent_usage_*`. Scans are incremental — a per-file byte offset with a
//! guard hash over the bytes just before it, or a high-water mark for the
//! database source — so a warm scan over a gigabyte of history costs only the
//! bytes appended since the last one.

pub mod claude;
pub mod codex;
pub mod cursor;
pub mod opencode;
pub mod scan;

use crate::analytics::{
    AnalyticsScanRequest, AnalyticsScanResult, CoverageState, DiscoveredAnalyticsSource,
    ImporterCapability, NumericUsagePayload, TokenUsage,
};
use crate::BridgeError;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::time::Instant;

/// Bumped whenever a parser's interpretation of a source changes; stored on
/// every observation so a later rescan can tell which rows predate a fix.
pub const IMPORTER_VERSION: &str = "usage-import/1";

/// Where the importers look. Carried explicitly so tests point every source at
/// a temporary directory instead of the developer's real history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceEnv {
    pub home: PathBuf,
    /// `CLAUDE_CONFIG_DIR`; transcripts live under `<dir>/projects`.
    pub claude_config_dir: Option<PathBuf>,
    /// `CODEX_HOME`; rollouts live under `<dir>/sessions`.
    pub codex_home: Option<PathBuf>,
    /// `XDG_DATA_HOME`; OpenCode's database is `<dir>/opencode/opencode.db`.
    pub xdg_data_home: Option<PathBuf>,
}

impl SourceEnv {
    /// Reads the process environment the way each CLI does.
    pub fn from_process() -> Self {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/"));
        Self {
            home,
            claude_config_dir: non_empty_path("CLAUDE_CONFIG_DIR"),
            codex_home: non_empty_path("CODEX_HOME"),
            xdg_data_home: non_empty_path("XDG_DATA_HOME"),
        }
    }

    /// Every source resolved beneath one directory, with no overrides.
    pub fn for_home(home: impl Into<PathBuf>) -> Self {
        Self {
            home: home.into(),
            claude_config_dir: None,
            codex_home: None,
            xdg_data_home: None,
        }
    }

    pub fn claude_projects_dir(&self) -> PathBuf {
        match &self.claude_config_dir {
            Some(dir) => dir.join("projects"),
            None => self.home.join(".claude").join("projects"),
        }
    }

    pub fn codex_sessions_dir(&self) -> PathBuf {
        match &self.codex_home {
            Some(dir) => dir.join("sessions"),
            None => self.home.join(".codex").join("sessions"),
        }
    }

    pub fn opencode_db_path(&self) -> PathBuf {
        let data_home = match &self.xdg_data_home {
            Some(dir) => dir.clone(),
            None => self.home.join(".local").join("share"),
        };
        data_home.join("opencode").join("opencode.db")
    }

    pub fn cursor_dir(&self) -> PathBuf {
        self.home.join(".cursor")
    }
}

fn non_empty_path(name: &str) -> Option<PathBuf> {
    let value = std::env::var_os(name)?;
    let text = value.to_string_lossy();
    if text.trim().is_empty() {
        None
    } else {
        Some(PathBuf::from(text.trim()))
    }
}

/// Lists every history source this machine could hold, whether or not it is
/// present. An absent directory is reported as `empty` with a reason so a
/// caller can show "no Codex history here" instead of silently listing three
/// sources on one machine and four on another.
pub fn discover_sources(env: &SourceEnv) -> Vec<DiscoveredAnalyticsSource> {
    vec![
        claude::discover(env),
        codex::discover(env),
        opencode::discover(env),
        cursor::discover(env),
    ]
}

/// Runs one bounded, incremental batch against a source and persists it.
///
/// `request.cursor` overrides the cursor stored on the source row; when it is
/// `None` the stored cursor is used, which is what a caller wants for "scan
/// whatever is new". The returned `next_cursor` is always the position to
/// resume from, and `coverage` is `partial` when `max_records` stopped the
/// batch before the source was exhausted.
pub fn scan(
    db: &Connection,
    request: &AnalyticsScanRequest,
    env: &SourceEnv,
) -> Result<AnalyticsScanResult, BridgeError> {
    request.validate()?;
    scan::run(db, request, env)
}

/// Plain-data summary of one `scan_all` pass, mirrored on the wire later.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ScanReport {
    pub sources: Vec<SourceScanOutcome>,
    pub records_imported: usize,
    pub records_skipped: usize,
    pub duration_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SourceScanOutcome {
    pub source_id: String,
    pub agent: String,
    pub provider: String,
    pub location: String,
    pub capability: ImporterCapability,
    pub coverage: CoverageState,
    pub records_imported: usize,
    pub records_skipped: usize,
    pub next_cursor: Option<String>,
    pub warning: Option<String>,
}

/// Discovers every source and runs one bounded batch against each supported
/// one. Unsupported sources are reported, never scanned. A source that fails
/// mid-scan is reported as `unreadable` with the error and does not stop the
/// others.
pub fn scan_all(
    db: &Connection,
    env: &SourceEnv,
    max_records_per_source: usize,
) -> Result<ScanReport, BridgeError> {
    let started = Instant::now();
    let max_records = max_records_per_source.clamp(1, 10_000);
    let mut report = ScanReport {
        sources: Vec::new(),
        records_imported: 0,
        records_skipped: 0,
        duration_ms: 0,
    };
    for source in discover_sources(env) {
        let source_id = source_id_for(&source);
        let mut outcome = SourceScanOutcome {
            source_id,
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
            match scan(db, &request, env) {
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

/// Stable row id for a source: the agent name plus a prefix of its location
/// fingerprint, so the same directory maps to the same row across runs.
pub fn source_id_for(source: &DiscoveredAnalyticsSource) -> String {
    let digest = source
        .location_fingerprint
        .strip_prefix("sha256:")
        .unwrap_or(&source.location_fingerprint);
    format!("{}-{}", source.agent, &digest[..digest.len().min(16)])
}

/// `sha256:` over the agent name and the absolute location; the path itself
/// never leaves the machine through this value.
pub(crate) fn location_fingerprint(agent: &str, location: &Path) -> String {
    let mut digest = Sha256::new();
    digest.update(agent.as_bytes());
    digest.update(b"\0");
    digest.update(location.to_string_lossy().as_bytes());
    format!("sha256:{:x}", digest.finalize())
}

pub(crate) fn sha256_hex(input: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(input.as_bytes());
    format!("{:x}", digest.finalize())
}

/// Integer micro-USD, rounded half up. Provider costs arrive as floats; the
/// store keeps integers so sums stay exact.
pub(crate) fn microusd_from_usd(cost: f64) -> Option<i64> {
    if !cost.is_finite() || cost < 0.0 {
        return None;
    }
    let scaled = (cost * 1_000_000.0 + 0.5).floor();
    if scaled > i64::MAX as f64 {
        return None;
    }
    Some(scaled as i64)
}

/// A non-negative integer read out of provider JSON; anything else is "not
/// reported". Floats are truncated, negatives dropped.
pub(crate) fn json_count(value: Option<&serde_json::Value>) -> Option<i64> {
    let value = value?;
    if let Some(number) = value.as_i64() {
        return (number >= 0).then_some(number);
    }
    if let Some(number) = value.as_u64() {
        return i64::try_from(number).ok();
    }
    let float = value.as_f64()?;
    if float.is_finite() && float >= 0.0 {
        Some(float.trunc() as i64)
    } else {
        None
    }
}

/// One provider-reported usage event, normalized but not yet persisted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ParsedUsage {
    pub native_record_id: String,
    pub native_session_id: Option<String>,
    pub parent_native_session_id: Option<String>,
    pub session_type: Option<String>,
    pub occurred_at: String,
    pub model: Option<String>,
    /// Provider on the record; OpenCode routes to many, the others to one.
    pub provider: String,
    pub input_semantics: &'static str,
    pub output_semantics: &'static str,
    pub usage: TokenUsage,
    pub numeric_usage: NumericUsagePayload,
    pub reported_cost_microusd: Option<i64>,
    /// Working directory the session ran in; stored only as a fingerprint
    /// plus its final path component.
    pub project_path: Option<String>,
}

impl ParsedUsage {
    pub(crate) fn has_tokens(&self) -> bool {
        [
            self.usage.total_input_tokens,
            self.usage.uncached_input_tokens,
            self.usage.cache_read_tokens,
            self.usage.cache_write_tokens,
            self.usage.output_tokens,
        ]
        .into_iter()
        .flatten()
        .any(|count| count > 0)
    }
}

pub(crate) fn parse_rfc3339(value: Option<&serde_json::Value>) -> Option<String> {
    let text = value?.as_str()?;
    chrono::DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|parsed| parsed.with_timezone(&chrono::Utc).to_rfc3339())
}

pub(crate) fn millis_to_rfc3339(millis: i64) -> Option<String> {
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(millis).map(|time| time.to_rfc3339())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn micro_usd_rounds_half_up_and_refuses_nonsense() {
        assert_eq!(microusd_from_usd(0.0257322), Some(25_732));
        assert_eq!(microusd_from_usd(0.0000005), Some(1));
        assert_eq!(microusd_from_usd(0.0000004), Some(0));
        assert_eq!(microusd_from_usd(3.0), Some(3_000_000));
        assert_eq!(microusd_from_usd(-1.0), None);
        assert_eq!(microusd_from_usd(f64::NAN), None);
    }

    #[test]
    fn source_env_honours_overrides() {
        let env = SourceEnv {
            home: PathBuf::from("/home/u"),
            claude_config_dir: Some(PathBuf::from("/cfg/claude")),
            codex_home: Some(PathBuf::from("/cfg/codex")),
            xdg_data_home: Some(PathBuf::from("/xdg")),
        };
        assert_eq!(
            env.claude_projects_dir(),
            PathBuf::from("/cfg/claude/projects")
        );
        assert_eq!(
            env.codex_sessions_dir(),
            PathBuf::from("/cfg/codex/sessions")
        );
        assert_eq!(
            env.opencode_db_path(),
            PathBuf::from("/xdg/opencode/opencode.db")
        );
        let plain = SourceEnv::for_home("/home/u");
        assert_eq!(
            plain.claude_projects_dir(),
            PathBuf::from("/home/u/.claude/projects")
        );
        assert_eq!(
            plain.opencode_db_path(),
            PathBuf::from("/home/u/.local/share/opencode/opencode.db")
        );
    }

    #[test]
    fn discovery_lists_all_four_agents_and_marks_cursor_unsupported() {
        let dir = tempfile::tempdir().unwrap();
        let env = SourceEnv::for_home(dir.path());
        let sources = discover_sources(&env);
        let agents: Vec<&str> = sources.iter().map(|s| s.agent.as_str()).collect();
        assert_eq!(agents, ["claude", "codex", "opencode", "cursor"]);
        for source in &sources {
            source.validate().unwrap();
        }
        let cursor = sources.iter().find(|s| s.agent == "cursor").unwrap();
        assert_eq!(cursor.capability, ImporterCapability::Unsupported);
        assert!(cursor.reason.is_some());
    }

    use super::claude::fixtures::assistant_line;
    use super::codex::fixtures::{forked_session_meta, session_meta, token_count, turn_context};
    use super::opencode::fixtures as oc;
    use std::fs;
    use std::io::Write;

    struct Harness {
        _dir: tempfile::TempDir,
        db: Connection,
        env: SourceEnv,
    }

    fn harness() -> Harness {
        let dir = tempfile::tempdir().unwrap();
        let db = crate::store::open(&dir.path().join("bridge.db")).unwrap();
        let env = SourceEnv::for_home(dir.path().join("home"));
        fs::create_dir_all(env.claude_projects_dir()).unwrap();
        fs::create_dir_all(env.codex_sessions_dir()).unwrap();
        Harness { _dir: dir, db, env }
    }

    fn source_for(env: &SourceEnv, agent: &str) -> DiscoveredAnalyticsSource {
        discover_sources(env)
            .into_iter()
            .find(|source| source.agent == agent)
            .unwrap()
    }

    fn scan_agent(h: &Harness, agent: &str, max_records: usize) -> AnalyticsScanResult {
        let request = AnalyticsScanRequest {
            source: source_for(&h.env, agent),
            cursor: None,
            max_records,
        };
        scan(&h.db, &request, &h.env).unwrap()
    }

    fn write_lines(path: &Path, lines: &[String]) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut file = fs::File::create(path).unwrap();
        for line in lines {
            writeln!(file, "{line}").unwrap();
        }
    }

    fn append_lines(path: &Path, lines: &[String]) {
        let mut file = fs::OpenOptions::new().append(true).open(path).unwrap();
        for line in lines {
            writeln!(file, "{line}").unwrap();
        }
    }

    fn bump_mtime(path: &Path) {
        // Same-second appends would otherwise look unchanged to a coarse
        // filesystem clock; the prefilter must see a different mtime.
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(5);
        fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(later)
            .unwrap();
    }

    fn count(db: &Connection, sql: &str) -> i64 {
        db.query_row(sql, [], |row| row.get(0)).unwrap()
    }

    fn claude_lines(session: &str, n: usize, start: usize) -> Vec<String> {
        (start..start + n)
            .map(|i| {
                assistant_line(
                    session,
                    &format!("msg_{i}"),
                    &format!("req_{i}"),
                    "claude-sonnet-5",
                    &format!("2026-08-24T11:{:02}:03.000Z", i % 60),
                    10,
                    100,
                    1000,
                    50,
                    "",
                )
            })
            .collect()
    }

    #[test]
    fn claude_scan_persists_observations_and_drops_repeats() {
        let h = harness();
        let file = h
            .env
            .claude_projects_dir()
            .join("-Users-x-proj/sess-1.jsonl");
        let mut lines = claude_lines("sess-1", 3, 0);
        // The same message repeated for a second content block.
        lines.push(lines[0].clone());
        lines.push(assistant_line(
            "sess-1",
            "msg_syn",
            "req_syn",
            "<synthetic>",
            "2026-08-24T11:59:00Z",
            1,
            0,
            0,
            1,
            "",
        ));
        lines.push(r#"{"type":"user","message":{"role":"user","content":"SECRET"}}"#.to_string());
        write_lines(&file, &lines);

        let result = scan_agent(&h, "claude", 100);
        assert_eq!(result.records_imported, 3);
        assert_eq!(result.records_skipped, 2, "one repeat and one synthetic");
        assert_eq!(result.coverage, CoverageState::Complete);
        assert!(result.next_cursor.is_some());
        assert_eq!(
            count(&h.db, "SELECT COUNT(*) FROM agent_usage_observations"),
            3
        );
        assert_eq!(count(&h.db, "SELECT COUNT(*) FROM agent_usage_sessions"), 1);
        let (semantics, formula, cost_source): (String, String, Option<String>) = h
            .db
            .query_row(
                "SELECT input_semantics, exact_total_formula, cost_source FROM agent_usage_observations LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(semantics, "exclusive");
        assert_eq!(formula, "anthropic_exclusive_input_plus_cache_and_output");
        assert_eq!(cost_source, None);
        let leaked = count(
            &h.db,
            "SELECT COUNT(*) FROM agent_usage_observations WHERE numeric_usage_json LIKE '%SECRET%'",
        );
        assert_eq!(leaked, 0);
        let (state, imported, cursor): (String, i64, Option<String>) = h
            .db
            .query_row(
                "SELECT coverage_state, records_imported, scan_cursor FROM agent_usage_sources WHERE agent='claude'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(state, "complete");
        assert_eq!(imported, 3);
        assert!(cursor.unwrap().contains("sess-1.jsonl"));
    }

    #[test]
    fn unchanged_file_imports_nothing_on_the_second_pass() {
        let h = harness();
        let file = h.env.claude_projects_dir().join("p/sess.jsonl");
        write_lines(&file, &claude_lines("sess", 4, 0));
        assert_eq!(scan_agent(&h, "claude", 100).records_imported, 4);
        let second = scan_agent(&h, "claude", 100);
        assert_eq!(second.records_imported, 0);
        assert_eq!(
            second.records_skipped, 0,
            "an unchanged file is never re-read"
        );
        assert_eq!(second.coverage, CoverageState::Complete);
    }

    #[test]
    fn appended_file_imports_only_the_new_lines() {
        let h = harness();
        let file = h.env.claude_projects_dir().join("p/sess.jsonl");
        write_lines(&file, &claude_lines("sess", 4, 0));
        assert_eq!(scan_agent(&h, "claude", 100).records_imported, 4);
        append_lines(&file, &claude_lines("sess", 2, 4));
        bump_mtime(&file);
        let second = scan_agent(&h, "claude", 100);
        assert_eq!(second.records_imported, 2);
        assert_eq!(
            second.records_skipped, 0,
            "resumed reads never revisit old lines"
        );
        assert_eq!(
            count(&h.db, "SELECT COUNT(*) FROM agent_usage_observations"),
            6
        );
    }

    #[test]
    fn rewritten_file_fails_the_guard_and_rescans_idempotently() {
        let h = harness();
        let file = h.env.claude_projects_dir().join("p/sess.jsonl");
        write_lines(&file, &claude_lines("sess", 4, 0));
        assert_eq!(scan_agent(&h, "claude", 100).records_imported, 4);

        // Truncate to two lines, then append a new one: the old offset is
        // past the end so the guard cannot match.
        write_lines(&file, &claude_lines("sess", 2, 0));
        append_lines(&file, &claude_lines("sess", 1, 10));
        bump_mtime(&file);
        let after_truncate = scan_agent(&h, "claude", 100);
        assert_eq!(after_truncate.records_imported, 1);
        assert_eq!(
            after_truncate.records_skipped, 2,
            "the two surviving lines were re-read and ignored"
        );

        // Rewrite in place to a longer file with different bytes at the old
        // offset: same length class as an append, but the guard differs.
        let mut rewritten = claude_lines("other-session", 3, 20);
        rewritten.extend(claude_lines("sess", 1, 30));
        write_lines(&file, &rewritten);
        bump_mtime(&file);
        let after_rewrite = scan_agent(&h, "claude", 100);
        assert_eq!(after_rewrite.records_imported, 4);
        assert_eq!(
            count(&h.db, "SELECT COUNT(*) FROM agent_usage_observations"),
            4 + 1 + 4
        );
    }

    #[test]
    fn batches_respect_max_records_and_resume_from_the_cursor() {
        let h = harness();
        write_lines(
            &h.env.claude_projects_dir().join("p/a.jsonl"),
            &claude_lines("a", 3, 0),
        );
        write_lines(
            &h.env.claude_projects_dir().join("p/b.jsonl"),
            &claude_lines("b", 3, 10),
        );
        let first = scan_agent(&h, "claude", 4);
        assert_eq!(first.records_imported, 4);
        assert_eq!(first.observations.len(), 4);
        assert_eq!(first.coverage, CoverageState::Partial);
        let cursor = first.next_cursor.clone().unwrap();

        let second = scan(
            &h.db,
            &AnalyticsScanRequest {
                source: source_for(&h.env, "claude"),
                cursor: Some(cursor),
                max_records: 4,
            },
            &h.env,
        )
        .unwrap();
        assert_eq!(second.records_imported, 2);
        assert_eq!(
            second.records_skipped, 0,
            "the partially read file resumed mid-way"
        );
        assert_eq!(second.coverage, CoverageState::Complete);
        assert_eq!(
            count(&h.db, "SELECT COUNT(*) FROM agent_usage_observations"),
            6
        );
        assert_eq!(scan_agent(&h, "claude", 4).records_imported, 0);
    }

    #[test]
    fn codex_scan_resumes_with_reducer_state_and_suppresses_forks() {
        let h = harness();
        let day = h.env.codex_sessions_dir().join("2026/05/27");
        let main = day.join("rollout-2026-05-27T12-39-05-sess-main.jsonl");
        write_lines(
            &main,
            &[
                session_meta("2026-05-27T07:09:16.540Z", "sess-main", ""),
                turn_context("2026-05-27T07:09:16.542Z", "gpt-5.5"),
                token_count("2026-05-27T07:09:23.814Z", 19707, 3456, 312, 40),
                token_count("2026-05-27T07:09:23.814Z", 19707, 3456, 312, 40),
            ],
        );
        let fork = day.join("rollout-2026-05-27T12-40-00-sess-fork.jsonl");
        write_lines(
            &fork,
            &[
                forked_session_meta("2026-05-27T07:10:00.000Z", "sess-fork", "sess-main"),
                session_meta("2026-05-27T07:10:00.001Z", "sess-main", ""),
                turn_context("2026-05-27T07:10:00.002Z", "gpt-5.5"),
                token_count("2026-05-27T07:10:00.010Z", 19707, 3456, 312, 40),
                token_count("2026-05-27T07:10:00.040Z", 25000, 19000, 100, 10),
                token_count("2026-05-27T07:10:09.000Z", 30000, 24000, 200, 20),
            ],
        );
        let first = scan_agent(&h, "codex", 100);
        assert_eq!(first.records_imported, 2, "one real turn per rollout");
        assert_eq!(first.records_skipped, 3, "one duplicate, two fork copies");
        let (parent, kind): (Option<String>, Option<String>) = h
            .db
            .query_row(
                "SELECT parent_native_session_id, session_type FROM agent_usage_sessions WHERE native_session_id='sess-fork'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(parent.as_deref(), Some("sess-main"));
        assert_eq!(kind.as_deref(), Some("fork"));
        let formula: String =
            h.db.query_row(
                "SELECT DISTINCT exact_total_formula FROM agent_usage_observations",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(formula, "input_includes_cache_plus_output");

        // Appending a turn without a fresh turn_context still attributes to
        // the carried model, and the cursor's signature drops a re-emit.
        append_lines(
            &main,
            &[
                token_count("2026-05-27T07:09:23.814Z", 19707, 3456, 312, 40),
                token_count("2026-05-27T07:11:00.000Z", 500, 0, 50, 5),
            ],
        );
        bump_mtime(&main);
        let second = scan_agent(&h, "codex", 100);
        assert_eq!(second.records_imported, 1);
        assert_eq!(second.records_skipped, 1);
        assert_eq!(second.observations[0].model.as_deref(), Some("gpt-5.5"));
        assert_eq!(second.observations[0].usage.total_input_tokens, Some(500));
    }

    #[test]
    fn opencode_scan_uses_a_high_water_mark_and_reports_cost() {
        let h = harness();
        let path = h.env.opencode_db_path();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let history = oc::create_db(&path);
        oc::insert_session(&history, "ses_parent", None, "/Users/x/voicey");
        oc::insert_session(&history, "ses_child", Some("ses_parent"), "/Users/x/voicey");
        oc::insert_message(
            &history,
            "msg_1",
            "ses_parent",
            1_778_842_293_500,
            &oc::assistant_data(
                "kimi-k2.6",
                "opencode-go",
                25036,
                230,
                257,
                0,
                0,
                0.0257322,
                1_778_842_278_253,
            ),
        );
        oc::insert_message(
            &history,
            "msg_2",
            "ses_child",
            1_778_842_300_000,
            &oc::assistant_data(
                "claude-sonnet-5",
                "anthropic",
                1000,
                100,
                0,
                800,
                50,
                0.01,
                1_778_842_299_000,
            ),
        );
        oc::insert_message(
            &history,
            "msg_u",
            "ses_parent",
            1_778_842_300_001,
            r#"{"role":"user","time":{"created":1778842300001}}"#,
        );
        drop(history);

        let first = scan_agent(&h, "opencode", 100);
        assert_eq!(first.records_imported, 2);
        assert_eq!(first.coverage, CoverageState::Complete);
        let (cost, cost_source, provider, model): (i64, String, String, String) =
            h.db.query_row(
                "SELECT o.reported_cost_microusd, o.cost_source, s.provider, o.model
                 FROM agent_usage_observations o JOIN agent_usage_sessions s ON s.id=o.session_id
                 WHERE o.native_record_id='msg_1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(cost, 25_732);
        assert_eq!(cost_source, "provider_reported");
        assert_eq!(provider, "opencode-go");
        assert_eq!(model, "kimi-k2.6");
        let (uncached, read, write): (i64, i64, i64) = h
            .db
            .query_row(
                "SELECT uncached_input_tokens, cache_read_tokens, cache_write_tokens FROM agent_usage_observations WHERE native_record_id='msg_2'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!((uncached, read, write), (150, 800, 50));
        let parent: Option<String> = h
            .db
            .query_row(
                "SELECT parent_native_session_id FROM agent_usage_sessions WHERE native_session_id='ses_child'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(parent.as_deref(), Some("ses_parent"));
        assert_eq!(
            count(
                &h.db,
                "SELECT COUNT(*) FROM agent_usage_sessions WHERE project_label='SECRET TITLE'"
            ),
            0,
            "session titles never enter the store"
        );
        assert_eq!(
            count(
                &h.db,
                "SELECT COUNT(*) FROM agent_usage_sessions WHERE project_label='voicey'"
            ),
            2
        );

        assert_eq!(scan_agent(&h, "opencode", 100).records_imported, 0);

        let history = Connection::open(&path).unwrap();
        oc::insert_message(
            &history,
            "msg_3",
            "ses_parent",
            1_778_842_400_000,
            &oc::assistant_data(
                "kimi-k2.6",
                "opencode-go",
                10,
                5,
                0,
                0,
                0,
                0.0001,
                1_778_842_399_000,
            ),
        );
        drop(history);
        let third = scan_agent(&h, "opencode", 100);
        assert_eq!(third.records_imported, 1);
        assert_eq!(third.observations[0].native_record_id, "msg_3");
    }

    #[test]
    fn cursor_source_scans_nothing_and_stays_unsupported() {
        let h = harness();
        fs::create_dir_all(h.env.cursor_dir().join("ai-tracking")).unwrap();
        let result = scan_agent(&h, "cursor", 100);
        assert_eq!(result.coverage, CoverageState::Unsupported);
        assert_eq!(result.records_imported, 0);
        assert!(result.warning.unwrap().contains("no token counts"));
        let state: String =
            h.db.query_row(
                "SELECT coverage_state FROM agent_usage_sources WHERE agent='cursor'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(state, "unsupported");
    }

    #[test]
    fn scan_all_reports_every_source_and_is_idempotent() {
        let h = harness();
        write_lines(
            &h.env.claude_projects_dir().join("p/s.jsonl"),
            &claude_lines("s", 2, 0),
        );
        let report = scan_all(&h.db, &h.env, 500).unwrap();
        assert_eq!(report.sources.len(), 4);
        assert_eq!(report.records_imported, 2);
        let claude = report.sources.iter().find(|s| s.agent == "claude").unwrap();
        assert_eq!(claude.coverage, CoverageState::Complete);
        assert_eq!(claude.records_imported, 2);
        let codex = report.sources.iter().find(|s| s.agent == "codex").unwrap();
        assert_eq!(codex.coverage, CoverageState::Empty);
        assert!(codex.warning.is_some());
        let cursor = report.sources.iter().find(|s| s.agent == "cursor").unwrap();
        assert_eq!(cursor.capability, ImporterCapability::Unsupported);
        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains("\"recordsImported\":2"));
        assert!(json.contains("\"durationMs\""));

        let again = scan_all(&h.db, &h.env, 500).unwrap();
        assert_eq!(again.records_imported, 0);
    }

    /// Scans this machine's real history into a throwaway database. Gated:
    /// it reads gigabytes and depends on what the developer has installed.
    #[test]
    fn smoke_real_sources_import_once() {
        if std::env::var_os("BRIDGE_USAGE_IMPORT_SMOKE").is_none() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let db = crate::store::open(&dir.path().join("bridge.db")).unwrap();
        let env = SourceEnv::from_process();
        let test_started = chrono::Utc::now().to_rfc3339();
        let mut total = 0;
        loop {
            let report = scan_all(&db, &env, 10_000).unwrap();
            total += report.records_imported;
            for source in &report.sources {
                eprintln!(
                    "smoke pass: {} imported={} skipped={} coverage={:?} warning={:?} ({} ms total)",
                    source.agent,
                    source.records_imported,
                    source.records_skipped,
                    source.coverage,
                    source.warning,
                    report.duration_ms
                );
            }
            if report
                .sources
                .iter()
                .all(|source| source.coverage != CoverageState::Partial)
            {
                break;
            }
        }
        assert!(
            total > 0,
            "expected this machine to hold some agent history"
        );
        let second = scan_all(&db, &env, 10_000).unwrap();
        eprintln!(
            "smoke second pass: imported={} skipped={} in {} ms",
            second.records_imported, second.records_skipped, second.duration_ms
        );
        // A live agent session (often the one running this test) may append
        // a record between passes. That is new usage, not a re-import: every
        // second-pass record must post-date the first pass.
        for source in &second.sources {
            if source.records_imported > 0 {
                eprintln!(
                    "smoke second pass source: {} imported={} coverage={:?}",
                    source.agent, source.records_imported, source.coverage
                );
            }
        }
        let late: Vec<(String, Option<String>, String)> = db
            .prepare("SELECT s.agent, o.native_record_id, o.occurred_at FROM agent_usage_observations o JOIN agent_usage_sources s ON s.id=o.source_id ORDER BY o.created_at DESC, o.occurred_at DESC LIMIT ?1")
            .unwrap()
            .query_map([second.records_imported as i64], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        for (agent, record, occurred_at) in &late {
            eprintln!(
                "smoke second pass record: {agent} record={record:?} occurred_at={occurred_at}"
            );
            assert!(
                occurred_at.as_str() >= test_started.as_str(),
                "a second pass re-imported history that predates the first pass"
            );
        }
        let per_source: Vec<(String, i64, i64, Option<String>, Option<String>)> = db
            .prepare("SELECT agent, records_imported, records_skipped, coverage_start_at, coverage_end_at FROM agent_usage_sources ORDER BY agent")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        eprintln!("smoke totals per source: {per_source:?}");
    }
}
