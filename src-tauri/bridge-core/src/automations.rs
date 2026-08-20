//! Unified automations: the scheduled jobs each harness already keeps on
//! disk, read from their native stores and shown in one catalog. Bridge is
//! not a scheduler here — Claude Code and Codex own execution; Bridge only
//! lists what they will run and applies the few mutations their formats
//! support (pause/resume for Codex, delete for both).
//!
//! Sources:
//! * Claude Code — `~/.claude/scheduled_tasks.json`, guarded by a sibling
//!   `scheduled_tasks.lock`. Its own reader skips malformed entries rather
//!   than failing the file, and this module mirrors that tolerance.
//! * Codex — `~/.codex/sqlite/*.db`, whichever databases carry an
//!   `automations` table (the app has shipped both `codex.db` and
//!   `codex-dev.db`). Read-only connections for the catalog; a short-lived
//!   writable connection for actions.
//! * OpenCode has no automations feature; the catalog reports it absent so
//!   the UI can say so instead of guessing.

use crate::BridgeError;
use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

/// How many run-history rows ride along per Codex automation.
const MAX_RUNS_PER_AUTOMATION: usize = 20;
/// How long an action waits for Claude Code's schedule lock before giving up.
const CLAUDE_LOCK_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum AutomationProvider {
    Claude,
    Codex,
    OpenCode,
}

impl AutomationProvider {
    fn display_name(self) -> &'static str {
        match self {
            Self::Claude => "Claude Code",
            Self::Codex => "Codex",
            Self::OpenCode => "OpenCode",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AutomationAction {
    Pause,
    Resume,
    Delete,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AutomationStatus {
    Active,
    Paused,
    Unknown,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutomationSchedule {
    /// `cron` (Claude, 5-field local-time cron) or `rrule` (Codex).
    pub kind: String,
    pub expression: String,
    /// Best-effort human phrasing; falls back to the raw expression.
    pub human: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutomationRun {
    pub id: String,
    pub status: String,
    pub title: Option<String>,
    pub summary: Option<String>,
    pub created_at: Option<i64>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnifiedAutomation {
    pub id: String,
    pub provider: AutomationProvider,
    pub name: String,
    pub prompt: String,
    pub schedule: AutomationSchedule,
    pub status: AutomationStatus,
    pub recurring: bool,
    /// Millisecond epochs, straight from the native store.
    pub created_at: Option<i64>,
    pub next_run_at: Option<i64>,
    pub last_run_at: Option<i64>,
    pub cwds: Vec<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub can_pause: bool,
    pub runs: Vec<AutomationRun>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutomationProviderState {
    pub provider: AutomationProvider,
    pub available: bool,
    /// Why the provider is unavailable, or where its store was found.
    pub detail: String,
    pub count: usize,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutomationCatalog {
    pub automations: Vec<UnifiedAutomation>,
    pub providers: Vec<AutomationProviderState>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutomationActionResult {
    pub provider: AutomationProvider,
    pub id: String,
    pub action: AutomationAction,
    pub success: bool,
    pub message: String,
}

// ---- Claude Code: ~/.claude/scheduled_tasks.json ---------------------------

fn claude_tasks_path(home: &Path) -> PathBuf {
    home.join(".claude").join("scheduled_tasks.json")
}

fn claude_lock_path(home: &Path) -> PathBuf {
    home.join(".claude").join("scheduled_tasks.lock")
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct ClaudeTaskFile {
    #[serde(default)]
    tasks: Vec<serde_json::Value>,
}

/// The fields Claude Code's own reader requires; everything else on the raw
/// value is preserved verbatim when the file is rewritten.
struct ClaudeTask {
    id: String,
    cron: String,
    prompt: String,
    created_at: i64,
    last_fired_at: Option<i64>,
    recurring: bool,
}

fn parse_claude_task(raw: &serde_json::Value) -> Option<ClaudeTask> {
    Some(ClaudeTask {
        id: raw.get("id")?.as_str()?.to_string(),
        cron: raw.get("cron")?.as_str()?.to_string(),
        prompt: raw.get("prompt")?.as_str()?.to_string(),
        created_at: raw.get("createdAt")?.as_i64()?,
        last_fired_at: raw.get("lastFiredAt").and_then(|value| value.as_i64()),
        recurring: raw
            .get("recurring")
            .and_then(|value| value.as_bool())
            .unwrap_or(true),
    })
}

fn claude_automations(home: &Path) -> (AutomationProviderState, Vec<UnifiedAutomation>) {
    let path = claude_tasks_path(home);
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return (
                AutomationProviderState {
                    provider: AutomationProvider::Claude,
                    available: true,
                    detail: "No scheduled tasks yet".to_string(),
                    count: 0,
                },
                Vec::new(),
            );
        }
        Err(error) => {
            return (
                AutomationProviderState {
                    provider: AutomationProvider::Claude,
                    available: false,
                    detail: format!("Cannot read {}: {error}", path.display()),
                    count: 0,
                },
                Vec::new(),
            );
        }
    };
    let file: ClaudeTaskFile = match serde_json::from_slice(&bytes) {
        Ok(file) => file,
        Err(error) => {
            return (
                AutomationProviderState {
                    provider: AutomationProvider::Claude,
                    available: false,
                    detail: format!("{} is unreadable: {error}", path.display()),
                    count: 0,
                },
                Vec::new(),
            );
        }
    };
    let automations: Vec<UnifiedAutomation> = file
        .tasks
        .iter()
        .filter_map(parse_claude_task)
        .map(|task| UnifiedAutomation {
            name: derive_name(&task.prompt),
            schedule: AutomationSchedule {
                kind: "cron".to_string(),
                human: cron_human(&task.cron),
                expression: task.cron,
            },
            // The file format has no paused state: a task is either present
            // and live or deleted.
            status: AutomationStatus::Active,
            recurring: task.recurring,
            created_at: Some(task.created_at),
            next_run_at: None,
            last_run_at: task.last_fired_at,
            cwds: Vec::new(),
            model: None,
            effort: None,
            can_pause: false,
            runs: Vec::new(),
            id: task.id,
            provider: AutomationProvider::Claude,
            prompt: task.prompt,
        })
        .collect();
    (
        AutomationProviderState {
            provider: AutomationProvider::Claude,
            available: true,
            detail: path.display().to_string(),
            count: automations.len(),
        },
        automations,
    )
}

/// Rewrite Claude's schedule file with `mutate` applied to the raw task list,
/// holding the same lock file Claude Code itself uses. Unparseable entries are
/// preserved untouched — Bridge never launders another writer's data.
fn edit_claude_tasks(
    home: &Path,
    mutate: impl FnOnce(&mut Vec<serde_json::Value>) -> Result<(), BridgeError>,
) -> Result<(), BridgeError> {
    let lock_path = claude_lock_path(home);
    let deadline = std::time::Instant::now() + CLAUDE_LOCK_TIMEOUT;
    let lock = loop {
        match fs::OpenOptions::new().write(true).create_new(true).open(&lock_path) {
            Ok(file) => break file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                if std::time::Instant::now() >= deadline {
                    return Err(BridgeError::Invalid(
                        "Claude Code is updating its schedules right now; try again in a moment"
                            .to_string(),
                    ));
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(error) => return Err(BridgeError::Io(error)),
        }
    };
    drop(lock);
    let result = (|| {
        let path = claude_tasks_path(home);
        let mut file: ClaudeTaskFile = match fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|error| BridgeError::Invalid(format!("{} is unreadable: {error}", path.display())))?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => ClaudeTaskFile { tasks: Vec::new() },
            Err(error) => return Err(BridgeError::Io(error)),
        };
        mutate(&mut file.tasks)?;
        let tmp = path.with_extension("json.bridge-tmp");
        fs::write(&tmp, serde_json::to_vec_pretty(&file).expect("schedule file serializes"))?;
        fs::rename(&tmp, &path)?;
        Ok(())
    })();
    let _ = fs::remove_file(&lock_path);
    result
}

// ---- Codex: ~/.codex/sqlite/*.db -------------------------------------------

fn codex_sqlite_dir(home: &Path) -> PathBuf {
    home.join(".codex").join("sqlite")
}

/// Every database under `~/.codex/sqlite` that carries an `automations`
/// table. The app has shipped differently named files across channels, so
/// membership is decided by schema, not filename.
fn codex_automation_dbs(home: &Path) -> Vec<PathBuf> {
    let dir = codex_sqlite_dir(home);
    let Ok(entries) = fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut found: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "db"))
        .filter(|path| {
            Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
                .and_then(|db| {
                    db.query_row(
                        "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='automations'",
                        [],
                        |row| row.get::<_, i64>(0),
                    )
                })
                .map(|count| count > 0)
                .unwrap_or(false)
        })
        .collect();
    found.sort();
    found
}

fn codex_automations(home: &Path) -> (AutomationProviderState, Vec<UnifiedAutomation>) {
    let dbs = codex_automation_dbs(home);
    if dbs.is_empty() {
        let dir = codex_sqlite_dir(home);
        let detail = if dir.is_dir() {
            "No automations database yet".to_string()
        } else {
            format!("{} not found", dir.display())
        };
        return (
            AutomationProviderState {
                provider: AutomationProvider::Codex,
                available: dir.is_dir(),
                detail,
                count: 0,
            },
            Vec::new(),
        );
    }
    let mut automations = Vec::new();
    let mut errors = Vec::new();
    for path in &dbs {
        match read_codex_db(path) {
            Ok(mut rows) => automations.append(&mut rows),
            Err(error) => errors.push(format!("{}: {error}", path.display())),
        }
    }
    // The same automation can surface from more than one database file after
    // a channel migration; the first (alphabetically stable) copy wins.
    automations.sort_by(|a, b| a.id.cmp(&b.id));
    automations.dedup_by(|a, b| a.id == b.id);
    let detail = if errors.is_empty() {
        dbs.iter().map(|path| path.display().to_string()).collect::<Vec<_>>().join(", ")
    } else {
        errors.join("; ")
    };
    (
        AutomationProviderState {
            provider: AutomationProvider::Codex,
            available: errors.len() < dbs.len(),
            detail,
            count: automations.len(),
        },
        automations,
    )
}

fn read_codex_db(path: &Path) -> Result<Vec<UnifiedAutomation>, BridgeError> {
    let db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let mut statement = db.prepare(
        "SELECT id, name, prompt, status, next_run_at, last_run_at, cwds, rrule,
                model, reasoning_effort, created_at
         FROM automations ORDER BY created_at DESC",
    )?;
    let mut automations: Vec<UnifiedAutomation> = statement
        .query_map([], |row| {
            let status_raw: String = row.get(3)?;
            let cwds_raw: String = row.get::<_, Option<String>>(6)?.unwrap_or_else(|| "[]".to_string());
            let rrule: String = row.get(7)?;
            Ok(UnifiedAutomation {
                id: row.get(0)?,
                provider: AutomationProvider::Codex,
                name: row.get(1)?,
                prompt: row.get(2)?,
                schedule: AutomationSchedule {
                    kind: "rrule".to_string(),
                    human: rrule_human(&rrule),
                    expression: rrule,
                },
                status: match status_raw.to_ascii_uppercase().as_str() {
                    "ACTIVE" => AutomationStatus::Active,
                    "PAUSED" => AutomationStatus::Paused,
                    _ => AutomationStatus::Unknown,
                },
                recurring: true,
                created_at: row.get(10)?,
                next_run_at: row.get(4)?,
                last_run_at: row.get(5)?,
                cwds: serde_json::from_str(&cwds_raw).unwrap_or_default(),
                model: row.get(8)?,
                effort: row.get(9)?,
                can_pause: true,
                runs: Vec::new(),
            })
        })?
        .collect::<Result<_, _>>()?;
    for automation in &mut automations {
        automation.runs = codex_runs(&db, &automation.id).unwrap_or_default();
    }
    Ok(automations)
}

fn codex_runs(db: &Connection, automation_id: &str) -> Result<Vec<AutomationRun>, BridgeError> {
    let mut statement = db.prepare(
        "SELECT thread_id, status, COALESCE(inbox_title, thread_title), inbox_summary, created_at
         FROM automation_runs WHERE automation_id = ?1 ORDER BY created_at DESC LIMIT ?2",
    )?;
    let runs = statement
        .query_map(rusqlite::params![automation_id, MAX_RUNS_PER_AUTOMATION as i64], |row| {
            Ok(AutomationRun {
                id: row.get(0)?,
                status: row.get(1)?,
                title: row.get(2)?,
                summary: row.get(3)?,
                created_at: row.get(4)?,
            })
        })?
        .collect::<Result<_, _>>()?;
    Ok(runs)
}

// ---- catalog ----------------------------------------------------------------

pub fn catalog(home: &Path) -> AutomationCatalog {
    let (claude_state, mut claude) = claude_automations(home);
    let (codex_state, mut codex) = codex_automations(home);
    let mut automations = Vec::with_capacity(claude.len() + codex.len());
    automations.append(&mut claude);
    automations.append(&mut codex);
    // Most recently created first, providers interleaved.
    automations.sort_by(|a, b| b.created_at.unwrap_or(0).cmp(&a.created_at.unwrap_or(0)));
    AutomationCatalog {
        automations,
        providers: vec![
            claude_state,
            codex_state,
            AutomationProviderState {
                provider: AutomationProvider::OpenCode,
                available: false,
                detail: format!("{} has no automations feature", AutomationProvider::OpenCode.display_name()),
                count: 0,
            },
        ],
    }
}

// ---- actions ----------------------------------------------------------------

pub fn execute(
    home: &Path,
    provider: AutomationProvider,
    id: &str,
    action: AutomationAction,
) -> Result<AutomationActionResult, BridgeError> {
    let message = match (provider, action) {
        (AutomationProvider::Claude, AutomationAction::Delete) => {
            edit_claude_tasks(home, |tasks| {
                let before = tasks.len();
                tasks.retain(|task| task.get("id").and_then(|value| value.as_str()) != Some(id));
                if tasks.len() == before {
                    return Err(BridgeError::Invalid(format!(
                        "No Claude Code scheduled task with id {id}"
                    )));
                }
                Ok(())
            })?;
            "Deleted from Claude Code's schedule file".to_string()
        }
        (AutomationProvider::Claude, _) => {
            return Err(BridgeError::Invalid(
                "Claude Code's schedule format has no paused state; delete the task or edit it in Claude Code"
                    .to_string(),
            ));
        }
        (AutomationProvider::Codex, action) => codex_execute(home, id, action)?,
        (AutomationProvider::OpenCode, _) => {
            return Err(BridgeError::Invalid(
                "OpenCode has no automations feature".to_string(),
            ));
        }
    };
    Ok(AutomationActionResult {
        provider,
        id: id.to_string(),
        action,
        success: true,
        message,
    })
}

fn codex_execute(home: &Path, id: &str, action: AutomationAction) -> Result<String, BridgeError> {
    let now_ms = chrono::Utc::now().timestamp_millis();
    for path in codex_automation_dbs(home) {
        let db = Connection::open(&path)?;
        let changed = match action {
            AutomationAction::Pause => db.execute(
                "UPDATE automations SET status='PAUSED', updated_at=?2 WHERE id=?1",
                rusqlite::params![id, now_ms],
            )?,
            AutomationAction::Resume => db.execute(
                "UPDATE automations SET status='ACTIVE', updated_at=?2 WHERE id=?1",
                rusqlite::params![id, now_ms],
            )?,
            AutomationAction::Delete => db.execute(
                "DELETE FROM automations WHERE id=?1",
                rusqlite::params![id],
            )?,
        };
        if changed > 0 {
            return Ok(match action {
                AutomationAction::Pause => "Paused in Codex".to_string(),
                AutomationAction::Resume => "Resumed in Codex".to_string(),
                AutomationAction::Delete => "Deleted from Codex".to_string(),
            });
        }
    }
    Err(BridgeError::Invalid(format!("No Codex automation with id {id}")))
}

// ---- schedule phrasing --------------------------------------------------------

const WEEKDAY_NAMES: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];

/// Best-effort phrasing for the 5-field cron shapes Claude Code produces.
/// Anything irregular falls back to the raw expression.
fn cron_human(expression: &str) -> String {
    let fields: Vec<&str> = expression.split_whitespace().collect();
    let [minute, hour, day_of_month, month, day_of_week] = fields.as_slice() else {
        return expression.to_string();
    };
    let time = |minute: &str, hour: &str| -> Option<String> {
        Some(format!("{:02}:{:02}", hour.parse::<u8>().ok()?, minute.parse::<u8>().ok()?))
    };
    if let Some(interval) = minute.strip_prefix("*/") {
        if [hour, day_of_month, month, day_of_week].iter().all(|field| **field == "*") {
            return format!("Every {interval} minutes");
        }
    }
    if (*day_of_month, *month) == ("*", "*") {
        if let Some(time) = time(minute, hour) {
            return match *day_of_week {
                "*" => format!("Daily at {time}"),
                "1-5" => format!("Weekdays at {time}"),
                "0,6" | "6,0" => format!("Weekends at {time}"),
                day => day
                    .parse::<usize>()
                    .ok()
                    .and_then(|index| WEEKDAY_NAMES.get(index))
                    .map(|name| format!("{name}s at {time}"))
                    .unwrap_or_else(|| expression.to_string()),
            };
        }
    }
    if *day_of_week == "*" {
        if let (Some(time), Ok(day), Ok(month)) = (time(minute, hour), day_of_month.parse::<u8>(), month.parse::<u8>()) {
            return format!("Once on {month:02}-{day:02} at {time}");
        }
    }
    expression.to_string()
}

/// Best-effort phrasing for the RRULE subset Codex writes
/// (FREQ/INTERVAL/BYHOUR/BYMINUTE/BYDAY).
fn rrule_human(rrule: &str) -> String {
    let mut freq = None;
    let mut interval: u32 = 1;
    let mut by_hour = None;
    let mut by_minute: u32 = 0;
    let mut by_day = None;
    for part in rrule.split(';') {
        match part.split_once('=') {
            Some(("FREQ", value)) => freq = Some(value.to_string()),
            Some(("INTERVAL", value)) => interval = value.parse().unwrap_or(1),
            Some(("BYHOUR", value)) => by_hour = value.parse::<u32>().ok(),
            Some(("BYMINUTE", value)) => by_minute = value.parse().unwrap_or(0),
            Some(("BYDAY", value)) => by_day = Some(value.to_string()),
            _ => {}
        }
    }
    let time = by_hour.map(|hour| format!("{hour:02}:{by_minute:02}"));
    match (freq.as_deref(), interval, time, by_day) {
        (Some("DAILY"), 1, Some(time), None) => format!("Daily at {time}"),
        (Some("WEEKLY"), 1, Some(time), Some(days)) => format!("Weekly ({days}) at {time}"),
        (Some("HOURLY"), 24, Some(time), _) => format!("Daily at {time}"),
        (Some("HOURLY"), 24, None, _) => "Daily".to_string(),
        (Some("HOURLY"), 1, _, _) => "Hourly".to_string(),
        (Some("HOURLY"), interval, _, _) => format!("Every {interval} hours"),
        (Some("DAILY"), interval, Some(time), _) if interval > 1 => {
            format!("Every {interval} days at {time}")
        }
        _ => rrule.to_string(),
    }
}

fn derive_name(prompt: &str) -> String {
    let first_line = prompt.lines().next().unwrap_or(prompt).trim();
    let cut = first_line
        .char_indices()
        .filter(|(_, character)| matches!(character, '.' | ';' | ':'))
        .map(|(index, _)| index)
        .find(|index| *index >= 12)
        .unwrap_or(first_line.len());
    let head = &first_line[..cut];
    if head.chars().count() <= 64 {
        head.to_string()
    } else {
        let truncated: String = head.chars().take(63).collect();
        format!("{}…", truncated.trim_end())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_home() -> tempfile::TempDir {
        tempfile::tempdir().expect("tempdir")
    }

    fn write_claude_tasks(home: &Path, tasks: serde_json::Value) {
        let dir = home.join(".claude");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("scheduled_tasks.json"),
            serde_json::to_vec(&serde_json::json!({ "tasks": tasks })).unwrap(),
        )
        .unwrap();
    }

    fn seed_codex_db(home: &Path) -> PathBuf {
        let dir = home.join(".codex").join("sqlite");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("codex.db");
        let db = Connection::open(&path).unwrap();
        db.execute_batch(
            "CREATE TABLE automations (
                id TEXT PRIMARY KEY, name TEXT NOT NULL, prompt TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'ACTIVE', next_run_at INTEGER, last_run_at INTEGER,
                cwds TEXT NOT NULL DEFAULT '[]', rrule TEXT NOT NULL, model TEXT,
                reasoning_effort TEXT, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL,
                target_type TEXT, project_id TEXT);
             CREATE TABLE automation_runs (
                thread_id TEXT PRIMARY KEY, automation_id TEXT NOT NULL, status TEXT NOT NULL,
                read_at INTEGER, thread_title TEXT, source_cwd TEXT, inbox_title TEXT,
                inbox_summary TEXT, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
             INSERT INTO automations VALUES
                ('auto-1','Nightly audit','Audit deps','ACTIVE',1900000000000,NULL,
                 '[\"/tmp/repo\"]','FREQ=DAILY;BYHOUR=3;BYMINUTE=15','gpt-5.3-codex','high',
                 1700000000000,1700000000000,'local',NULL);
             INSERT INTO automation_runs VALUES
                ('thread-1','auto-1','COMPLETED',NULL,'Audit run',NULL,'Deps clean',
                 'No CVEs found',1700000100000,1700000100000);",
        )
        .unwrap();
        path
    }

    #[test]
    fn catalog_merges_both_providers_and_reports_opencode_unsupported() {
        let home = fixture_home();
        write_claude_tasks(
            home.path(),
            serde_json::json!([
                {"id": "task-1", "cron": "7 9 * * 1-5", "prompt": "Summarize CI failures", "createdAt": 1800000000000i64, "recurring": true},
                {"id": "task-broken", "cron": 5, "prompt": "malformed"},
            ]),
        );
        seed_codex_db(home.path());
        let catalog = catalog(home.path());
        assert_eq!(catalog.automations.len(), 2, "malformed Claude entry is skipped, not fatal");
        assert_eq!(catalog.automations[0].id, "task-1", "newest created first");
        assert_eq!(catalog.automations[0].schedule.human, "Weekdays at 09:07");
        let codex = &catalog.automations[1];
        assert_eq!(codex.schedule.human, "Daily at 03:15");
        assert_eq!(codex.cwds, vec!["/tmp/repo".to_string()]);
        assert_eq!(codex.runs.len(), 1);
        assert!(codex.can_pause);
        let opencode = catalog.providers.iter().find(|state| state.provider == AutomationProvider::OpenCode).unwrap();
        assert!(!opencode.available);
    }

    #[test]
    fn absent_stores_degrade_to_empty_availability_not_errors() {
        let home = fixture_home();
        let catalog = catalog(home.path());
        assert!(catalog.automations.is_empty());
        let claude = catalog.providers.iter().find(|state| state.provider == AutomationProvider::Claude).unwrap();
        assert!(claude.available, "a missing file just means no tasks yet");
        let codex = catalog.providers.iter().find(|state| state.provider == AutomationProvider::Codex).unwrap();
        assert!(!codex.available);
    }

    #[test]
    fn claude_delete_removes_exactly_the_named_task_and_preserves_unknown_fields() {
        let home = fixture_home();
        write_claude_tasks(
            home.path(),
            serde_json::json!([
                {"id": "task-1", "cron": "7 9 * * *", "prompt": "a", "createdAt": 1, "customField": "keep-me"},
                {"id": "task-2", "cron": "0 12 * * *", "prompt": "b", "createdAt": 2},
            ]),
        );
        execute(home.path(), AutomationProvider::Claude, "task-2", AutomationAction::Delete).unwrap();
        let raw: serde_json::Value =
            serde_json::from_slice(&fs::read(claude_tasks_path(home.path())).unwrap()).unwrap();
        let tasks = raw.get("tasks").unwrap().as_array().unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].get("customField").unwrap(), "keep-me");
        assert!(!claude_lock_path(home.path()).exists(), "lock released");
    }

    #[test]
    fn claude_pause_is_refused_because_the_format_has_no_paused_state() {
        let home = fixture_home();
        write_claude_tasks(home.path(), serde_json::json!([{"id": "task-1", "cron": "7 9 * * *", "prompt": "a", "createdAt": 1}]));
        let error = execute(home.path(), AutomationProvider::Claude, "task-1", AutomationAction::Pause).unwrap_err();
        assert!(matches!(error, BridgeError::Invalid(_)));
    }

    #[test]
    fn claude_delete_waits_out_a_held_lock_then_gives_up_clearly() {
        let home = fixture_home();
        write_claude_tasks(home.path(), serde_json::json!([{"id": "task-1", "cron": "7 9 * * *", "prompt": "a", "createdAt": 1}]));
        fs::write(claude_lock_path(home.path()), b"").unwrap();
        let error = execute(home.path(), AutomationProvider::Claude, "task-1", AutomationAction::Delete).unwrap_err();
        assert!(error.to_string().contains("try again"), "{error}");
    }

    #[test]
    fn codex_pause_resume_and_delete_round_trip() {
        let home = fixture_home();
        let path = seed_codex_db(home.path());
        execute(home.path(), AutomationProvider::Codex, "auto-1", AutomationAction::Pause).unwrap();
        let status: String = Connection::open(&path)
            .unwrap()
            .query_row("SELECT status FROM automations WHERE id='auto-1'", [], |row| row.get(0))
            .unwrap();
        assert_eq!(status, "PAUSED");
        execute(home.path(), AutomationProvider::Codex, "auto-1", AutomationAction::Resume).unwrap();
        execute(home.path(), AutomationProvider::Codex, "auto-1", AutomationAction::Delete).unwrap();
        let count: i64 = Connection::open(&path)
            .unwrap()
            .query_row("SELECT count(*) FROM automations", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);
        let missing = execute(home.path(), AutomationProvider::Codex, "auto-1", AutomationAction::Delete).unwrap_err();
        assert!(matches!(missing, BridgeError::Invalid(_)));
    }

    #[test]
    fn schedule_phrasing_covers_the_common_shapes_and_falls_back_raw() {
        assert_eq!(cron_human("*/5 * * * *"), "Every 5 minutes");
        assert_eq!(cron_human("0 12 * * *"), "Daily at 12:00");
        assert_eq!(cron_human("30 14 28 2 *"), "Once on 02-28 at 14:30");
        assert_eq!(cron_human("3 9 * * 1"), "Mondays at 09:03");
        assert_eq!(cron_human("not a cron"), "not a cron");
        assert_eq!(rrule_human("FREQ=HOURLY;INTERVAL=24;BYMINUTE=0"), "Daily");
        assert_eq!(rrule_human("FREQ=HOURLY;INTERVAL=1"), "Hourly");
        assert_eq!(rrule_human("FREQ=WEEKLY;BYDAY=MO,WE;BYHOUR=9;BYMINUTE=30"), "Weekly (MO,WE) at 09:30");
        assert_eq!(rrule_human("FREQ=YEARLY"), "FREQ=YEARLY");
    }

    #[test]
    fn names_derive_from_the_prompt_head() {
        assert_eq!(derive_name("Summarize overnight CI failures. Then file issues."), "Summarize overnight CI failures");
        assert!(derive_name(&"very long ".repeat(30)).chars().count() <= 64);
    }
}
