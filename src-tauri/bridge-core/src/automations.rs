//! Unified automations: the scheduled jobs each harness already keeps on
//! disk, read from their native stores and shown in one catalog. Bridge is
//! not a scheduler here — Claude Code and Codex own execution; Bridge only
//! lists what they will run and applies the few mutations their formats
//! support (create/edit/delete for Claude; pause/resume/delete for Codex).
//!
//! Sources:
//! * Claude Code — `~/.claude/scheduled_tasks.json`, guarded by a sibling
//!   `scheduled_tasks.lock`. Its own reader skips malformed entries rather
//!   than failing the file, and this module mirrors that tolerance.
//! * Codex — `~/.codex/sqlite/*.db`, whichever databases carry an
//!   `automations` table (the app has shipped both `codex.db` and
//!   `codex-dev.db`). Read-only connections for the catalog; a short-lived
//!   writable connection for actions.
//! * Cursor and OpenCode have no automations feature; the catalog reports
//!   them absent so the UI can say so instead of guessing.

use crate::BridgeError;
use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs,
    io::Write,
    path::{Path, PathBuf},
    time::Duration,
};
use uuid::Uuid;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;

/// How many run-history rows ride along per Codex automation.
const MAX_RUNS_PER_AUTOMATION: usize = 20;
/// How long an action waits for Claude Code's schedule lock before giving up.
const CLAUDE_LOCK_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum AutomationProvider {
    Claude,
    Codex,
    Cursor,
    OpenCode,
}

impl AutomationProvider {
    fn display_name(self) -> &'static str {
        match self {
            Self::Claude => "Claude Code",
            Self::Codex => "Codex",
            Self::Cursor => "Cursor",
            Self::OpenCode => "OpenCode",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum AutomationCapability {
    Create,
    Edit,
    RunNow,
    Pause,
    Resume,
    Delete,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum AutomationAction {
    Pause,
    Resume,
    RunNow,
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
    pub automation_id: String,
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
    pub capabilities: Vec<AutomationCapability>,
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

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutomationSaveResult {
    pub provider: AutomationProvider,
    pub id: String,
    pub created: bool,
    pub message: String,
}

pub fn provider_capabilities(provider: AutomationProvider) -> Vec<AutomationCapability> {
    match provider {
        AutomationProvider::Claude => vec![
            AutomationCapability::Create,
            AutomationCapability::Edit,
            AutomationCapability::Delete,
        ],
        AutomationProvider::Codex => vec![
            AutomationCapability::Pause,
            AutomationCapability::Resume,
            AutomationCapability::Delete,
        ],
        // Neither ships an automations store, so neither exposes a control.
        AutomationProvider::Cursor | AutomationProvider::OpenCode => Vec::new(),
    }
}

/// A provider Bridge can only report on, never mutate.
fn unsupported_provider_state(provider: AutomationProvider) -> AutomationProviderState {
    AutomationProviderState {
        provider,
        available: false,
        detail: format!("{} has no native automations feature", provider.display_name()),
        count: 0,
        capabilities: provider_capabilities(provider),
    }
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
    #[serde(flatten)]
    extra: serde_json::Map<String, serde_json::Value>,
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
                    capabilities: provider_capabilities(AutomationProvider::Claude),
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
                    capabilities: provider_capabilities(AutomationProvider::Claude),
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
                    capabilities: provider_capabilities(AutomationProvider::Claude),
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
            capabilities: provider_capabilities(AutomationProvider::Claude),
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
    if let Some(directory) = lock_path.parent() {
        fs::create_dir_all(directory)?;
    }
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
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => ClaudeTaskFile {
                tasks: Vec::new(),
                extra: serde_json::Map::new(),
            },
            Err(error) => return Err(BridgeError::Io(error)),
        };
        mutate(&mut file.tasks)?;
        let tmp = path.with_extension("json.bridge-tmp");
        let _ = fs::remove_file(&tmp);
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        let mut tmp_file = options.open(&tmp)?;
        if let Ok(metadata) = fs::metadata(&path) {
            fs::set_permissions(&tmp, metadata.permissions())?;
        }
        tmp_file.write_all(&serde_json::to_vec_pretty(&file).expect("schedule file serializes"))?;
        tmp_file.sync_all()?;
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

/// Every database under `~/.codex/sqlite` that carries the Codex automation
/// and run-history schemas. The app has shipped differently named files across
/// channels, so membership is decided by schema, not filename.
fn has_codex_automation_schema(db: &Connection) -> bool {
    const AUTOMATION_COLUMNS: [&str; 12] = [
        "id", "name", "prompt", "status", "next_run_at", "last_run_at", "cwds", "rrule", "model",
        "reasoning_effort", "created_at", "updated_at",
    ];
    const RUN_COLUMNS: [&str; 7] = [
        "thread_id", "automation_id", "status", "thread_title", "inbox_title", "inbox_summary",
        "created_at",
    ];
    table_has_columns(db, "automations", &AUTOMATION_COLUMNS)
        && table_has_columns(db, "automation_runs", &RUN_COLUMNS)
}

fn table_has_columns(db: &Connection, table: &str, required: &[&str]) -> bool {
    let Ok(mut statement) = db.prepare(&format!("PRAGMA table_info({table})")) else {
        return false;
    };
    let Ok(rows) = statement.query_map([], |row| row.get::<_, String>(1)) else {
        return false;
    };
    let columns: HashSet<String> = rows.flatten().collect();
    required.iter().all(|column| columns.contains(*column))
}

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
                .map(|db| has_codex_automation_schema(&db))
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
                available: false,
                detail,
                count: 0,
                capabilities: provider_capabilities(AutomationProvider::Codex),
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
            capabilities: provider_capabilities(AutomationProvider::Codex),
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
                automation_id: automation_id.to_string(),
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
            unsupported_provider_state(AutomationProvider::Cursor),
            unsupported_provider_state(AutomationProvider::OpenCode),
        ],
    }
}

// ---- actions ----------------------------------------------------------------

const MONTH_NAMES: [&str; 12] = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];
const DAY_NAMES: [&str; 7] = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"];

/// One cron field's numeric domain, plus the three-letter aliases it accepts.
struct CronField {
    label: &'static str,
    min: u8,
    max: u8,
    names: &'static [&'static str],
}

const CRON_FIELDS: [CronField; 5] = [
    CronField { label: "minute", min: 0, max: 59, names: &[] },
    CronField { label: "hour", min: 0, max: 23, names: &[] },
    CronField { label: "day of month", min: 1, max: 31, names: &[] },
    CronField { label: "month", min: 1, max: 12, names: &MONTH_NAMES },
    // 0 and 7 both mean Sunday, the way every cron implementation reads it.
    CronField { label: "day of week", min: 0, max: 7, names: &DAY_NAMES },
];

impl CronField {
    fn value(&self, token: &str) -> Option<u8> {
        let lowered = token.to_ascii_lowercase();
        let number = match self.names.iter().position(|name| *name == lowered) {
            // Named months are one-based; named weekdays are zero-based.
            Some(index) => u8::try_from(index).ok()? + self.min,
            None => token.parse::<u8>().ok()?,
        };
        (self.min..=self.max).contains(&number).then_some(number)
    }

    /// A field is a comma-separated list of `*`, `n`, `a-b`, or any of those
    /// followed by `/step`. Anything else is not a schedule Claude Code can run.
    fn accepts(&self, field: &str) -> bool {
        !field.is_empty()
            && field.split(',').all(|item| {
                let (spec, step) = match item.split_once('/') {
                    Some((spec, step)) => (spec, Some(step)),
                    None => (item, None),
                };
                if step.is_some_and(|step| !matches!(step.parse::<u8>(), Ok(1..=u8::MAX))) {
                    return false;
                }
                match spec.split_once('-') {
                    Some((low, high)) => matches!((self.value(low), self.value(high)), (Some(low), Some(high)) if low <= high),
                    None => spec == "*" || self.value(spec).is_some(),
                }
            })
    }
}

/// Claude Code parses these expressions itself, and a file it cannot parse is
/// a file it may reject wholesale — so Bridge refuses to write a cron it does
/// not understand rather than reporting success for a task that never fires.
fn validate_cron(expression: &str) -> Result<(), BridgeError> {
    let fields: Vec<&str> = expression.split_whitespace().collect();
    let Ok(fields) = <[&str; 5]>::try_from(fields.as_slice()) else {
        return Err(BridgeError::Invalid(format!(
            "Claude Code schedules require a five-field cron expression (minute hour day-of-month month day-of-week); got {} field{}",
            expression.split_whitespace().count(),
            if expression.split_whitespace().count() == 1 { "" } else { "s" }
        )));
    };
    for (field, spec) in CRON_FIELDS.iter().zip(fields) {
        if !field.accepts(spec) {
            return Err(BridgeError::Invalid(format!(
                "`{spec}` is not a valid cron {} field",
                field.label
            )));
        }
    }
    Ok(())
}

fn validate_claude_draft(prompt: &str, schedule_expression: &str) -> Result<(), BridgeError> {
    if prompt.trim().is_empty() {
        return Err(BridgeError::Invalid("Automation prompt cannot be empty".to_string()));
    }
    validate_cron(schedule_expression)
}

/// Create or edit an automation only through a native provider store that
/// exposes a safe write shape. This never calculates a next occurrence or
/// synthesizes Codex/Cursor records.
pub fn save(
    home: &Path,
    provider: AutomationProvider,
    id: Option<&str>,
    prompt: &str,
    schedule_expression: &str,
    recurring: bool,
) -> Result<AutomationSaveResult, BridgeError> {
    let operation = if id.is_some() { AutomationCapability::Edit } else { AutomationCapability::Create };
    if !provider_capabilities(provider).contains(&operation) {
        return Err(BridgeError::Invalid(format!(
            "{} does not expose native automation {} support",
            provider.display_name(),
            if id.is_some() { "editing" } else { "creation" }
        )));
    }
    // Capabilities say *whether* a provider can be written; this match says
    // *where* that write lands. Both have to agree before a byte moves, so
    // widening a capability list can never redirect one store into another.
    match provider {
        AutomationProvider::Claude => save_claude(home, id, prompt, schedule_expression, recurring),
        AutomationProvider::Codex | AutomationProvider::Cursor | AutomationProvider::OpenCode => {
            Err(BridgeError::Invalid(format!(
                "Bridge has no native write path for {} automations",
                provider.display_name()
            )))
        }
    }
}

fn save_claude(
    home: &Path,
    id: Option<&str>,
    prompt: &str,
    schedule_expression: &str,
    recurring: bool,
) -> Result<AutomationSaveResult, BridgeError> {
    validate_claude_draft(prompt, schedule_expression)?;

    let created = id.is_none();
    let automation_id = id.map(str::to_string).unwrap_or_else(|| Uuid::new_v4().to_string());
    let prompt = prompt.trim().to_string();
    let schedule_expression = schedule_expression.trim().to_string();
    edit_claude_tasks(home, |tasks| {
        if created {
            tasks.push(serde_json::json!({
                "id": automation_id,
                "cron": schedule_expression,
                "prompt": prompt,
                "createdAt": chrono::Utc::now().timestamp_millis(),
                "recurring": recurring,
            }));
            return Ok(());
        }

        let Some(task) = tasks.iter_mut().find(|task| {
            task.get("id").and_then(serde_json::Value::as_str) == Some(automation_id.as_str())
        }) else {
            return Err(BridgeError::Invalid(format!("No Claude Code scheduled task with id {automation_id}")));
        };
        let Some(object) = task.as_object_mut() else {
            return Err(BridgeError::Invalid(format!("Claude Code scheduled task {automation_id} is not editable")));
        };
        object.insert("cron".to_string(), schedule_expression.into());
        object.insert("prompt".to_string(), prompt.into());
        object.insert("recurring".to_string(), recurring.into());
        Ok(())
    })?;

    Ok(AutomationSaveResult {
        provider: AutomationProvider::Claude,
        id: automation_id,
        created,
        message: if created {
            "Created in Claude Code's schedule file".to_string()
        } else {
            "Updated in Claude Code's schedule file".to_string()
        },
    })
}

pub fn execute(
    home: &Path,
    provider: AutomationProvider,
    id: &str,
    action: AutomationAction,
) -> Result<AutomationActionResult, BridgeError> {
    let capability = match action {
        AutomationAction::Pause => AutomationCapability::Pause,
        AutomationAction::Resume => AutomationCapability::Resume,
        AutomationAction::RunNow => AutomationCapability::RunNow,
        AutomationAction::Delete => AutomationCapability::Delete,
    };
    if !provider_capabilities(provider).contains(&capability) {
        return Err(BridgeError::Invalid(format!(
            "{} does not expose native automation {} support",
            provider.display_name(),
            match action {
                AutomationAction::Pause => "pause",
                AutomationAction::Resume => "resume",
                AutomationAction::RunNow => "run-now",
                AutomationAction::Delete => "delete",
            }
        )));
    }
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
        (AutomationProvider::Codex, action) => codex_execute(home, id, action)?,
        // Same contract as `save`: the capability list decides whether an
        // action is offered, this match decides which store performs it. A
        // capability with no store behind it is an error, never a panic.
        (provider, action) => {
            return Err(BridgeError::Invalid(format!(
                "Bridge has no native {action:?} path for {} automations",
                provider.display_name()
            )));
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
    // Resolved once, up front: every action Codex's schema can perform maps to
    // one statement and one message here, and anything else leaves with an
    // error rather than reaching a panic further in.
    let (statement, stamps_updated_at, success) = match action {
        AutomationAction::Pause => (
            "UPDATE automations SET status='PAUSED', updated_at=?2 WHERE id=?1",
            true,
            "Paused in Codex",
        ),
        AutomationAction::Resume => (
            "UPDATE automations SET status='ACTIVE', updated_at=?2 WHERE id=?1",
            true,
            "Resumed in Codex",
        ),
        AutomationAction::Delete => ("DELETE FROM automations WHERE id=?1", false, "Deleted from Codex"),
        AutomationAction::RunNow => {
            return Err(BridgeError::Invalid(
                "Codex has no native run-now API; Bridge will not fake one by starting a session"
                    .to_string(),
            ));
        }
    };
    let now_ms = chrono::Utc::now().timestamp_millis();
    for path in codex_automation_dbs(home) {
        let db = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
        if !has_codex_automation_schema(&db) {
            continue;
        }
        let changed = if stamps_updated_at {
            db.execute(statement, rusqlite::params![id, now_ms])?
        } else {
            db.execute(statement, rusqlite::params![id])?
        };
        if changed > 0 {
            return Ok(success.to_string());
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
    fn catalog_merges_native_stores_and_reports_provider_capabilities() {
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
        assert_eq!(codex.runs[0].automation_id, "auto-1");
        let claude_state = catalog.providers.iter().find(|state| state.provider == AutomationProvider::Claude).unwrap();
        assert_eq!(claude_state.capabilities, vec![AutomationCapability::Create, AutomationCapability::Edit, AutomationCapability::Delete]);
        let codex_state = catalog.providers.iter().find(|state| state.provider == AutomationProvider::Codex).unwrap();
        assert_eq!(codex_state.capabilities, vec![AutomationCapability::Pause, AutomationCapability::Resume, AutomationCapability::Delete]);
        // Bridge ships four harnesses; the two without an automations store
        // must still be named, or the strip silently under-reports the machine.
        for provider in [AutomationProvider::Cursor, AutomationProvider::OpenCode] {
            let state = catalog.providers.iter().find(|state| state.provider == provider).unwrap();
            assert!(!state.available, "{provider:?}");
            assert!(state.capabilities.is_empty(), "{provider:?}");
            assert!(state.detail.contains("no native automations feature"), "{provider:?}: {}", state.detail);
        }
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
    fn unrelated_sqlite_database_is_not_treated_as_codex_automations() {
        let home = fixture_home();
        let dir = home.path().join(".codex").join("sqlite");
        fs::create_dir_all(&dir).unwrap();
        let db = Connection::open(dir.join("unrelated.db")).unwrap();
        db.execute_batch(
            "CREATE TABLE automations (
                id TEXT, name TEXT, prompt TEXT, status TEXT, next_run_at INTEGER,
                last_run_at INTEGER, cwds TEXT, rrule TEXT, model TEXT,
                reasoning_effort TEXT, created_at INTEGER, updated_at INTEGER
            );",
        )
        .unwrap();
        drop(db);

        let catalog = catalog(home.path());
        let codex = catalog
            .providers
            .iter()
            .find(|state| state.provider == AutomationProvider::Codex)
            .unwrap();
        assert!(!codex.available);
        assert!(catalog.automations.is_empty());
    }

    #[test]
    fn claude_delete_removes_exactly_the_named_task_and_preserves_unknown_fields() {
        let home = fixture_home();
        let dir = home.path().join(".claude");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("scheduled_tasks.json"),
            serde_json::to_vec(&serde_json::json!({
                "tasks": [
                    {"id": "task-1", "cron": "7 9 * * *", "prompt": "a", "createdAt": 1, "customField": "keep-me"},
                    {"id": "task-2", "cron": "0 12 * * *", "prompt": "b", "createdAt": 2}
                ],
                "futureMetadata": {"keep": true}
            }))
            .unwrap(),
        )
        .unwrap();
        execute(home.path(), AutomationProvider::Claude, "task-2", AutomationAction::Delete).unwrap();
        let raw: serde_json::Value =
            serde_json::from_slice(&fs::read(claude_tasks_path(home.path())).unwrap()).unwrap();
        let tasks = raw.get("tasks").unwrap().as_array().unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].get("customField").unwrap(), "keep-me");
        assert_eq!(raw["futureMetadata"], serde_json::json!({"keep": true}));
        assert!(!claude_lock_path(home.path()).exists(), "lock released");
    }

    #[test]
    fn claude_create_writes_native_shape_and_preserves_file_metadata() {
        let home = fixture_home();
        let dir = home.path().join(".claude");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("scheduled_tasks.json"), serde_json::to_vec(&serde_json::json!({
            "tasks": [], "futureMetadata": {"keep": true}
        })).unwrap()).unwrap();

        let result = save(home.path(), AutomationProvider::Claude, None, "  Summarize CI failures  ", "  7 9 * * 1-5  ", true).unwrap();
        assert!(result.created);
        assert!(!result.id.is_empty());
        let raw: serde_json::Value = serde_json::from_slice(&fs::read(claude_tasks_path(home.path())).unwrap()).unwrap();
        assert_eq!(raw["futureMetadata"], serde_json::json!({"keep": true}));
        let task = &raw["tasks"][0];
        assert_eq!(task["id"], result.id);
        assert_eq!(task["cron"], "7 9 * * 1-5");
        assert_eq!(task["prompt"], "Summarize CI failures");
        assert_eq!(task["recurring"], true);
        assert!(task["createdAt"].as_i64().is_some());
        assert!(!claude_lock_path(home.path()).exists(), "lock released");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(fs::metadata(claude_tasks_path(home.path())).unwrap().permissions().mode() & 0o777, 0o644);
        }
    }

    #[test]
    fn claude_create_initializes_a_missing_native_store_privately() {
        let home = fixture_home();
        save(home.path(), AutomationProvider::Claude, None, "Review open pull requests", "0 9 * * 1-5", true).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(fs::metadata(claude_tasks_path(home.path())).unwrap().permissions().mode() & 0o777, 0o600);
        }
    }

    #[test]
    fn claude_edit_updates_supported_fields_and_preserves_unknown_fields() {
        let home = fixture_home();
        write_claude_tasks(home.path(), serde_json::json!([{
            "id": "task-1", "cron": "7 9 * * *", "prompt": "old", "createdAt": 123,
            "lastFiredAt": 456, "recurring": true, "futureField": {"keep": true}
        }]));
        let result = save(home.path(), AutomationProvider::Claude, Some("task-1"), "new prompt", "0 12 * * *", false).unwrap();
        assert!(!result.created);
        let raw: serde_json::Value = serde_json::from_slice(&fs::read(claude_tasks_path(home.path())).unwrap()).unwrap();
        let task = &raw["tasks"][0];
        assert_eq!(task["id"], "task-1");
        assert_eq!(task["createdAt"], 123);
        assert_eq!(task["lastFiredAt"], 456);
        assert_eq!(task["futureField"], serde_json::json!({"keep": true}));
        assert_eq!(task["cron"], "0 12 * * *");
        assert_eq!(task["prompt"], "new prompt");
        assert_eq!(task["recurring"], false);
    }

    #[test]
    fn invalid_or_unsupported_saves_do_not_mutate_native_stores() {
        let home = fixture_home();
        write_claude_tasks(home.path(), serde_json::json!([{
            "id": "task-1", "cron": "7 9 * * *", "prompt": "old", "createdAt": 123
        }]));
        let before = fs::read(claude_tasks_path(home.path())).unwrap();
        let invalid = save(home.path(), AutomationProvider::Claude, Some("task-1"), "", "bad cron", true).unwrap_err();
        assert!(invalid.to_string().contains("prompt"));
        assert_eq!(fs::read(claude_tasks_path(home.path())).unwrap(), before);
        let missing = save(home.path(), AutomationProvider::Claude, Some("missing"), "valid", "0 9 * * *", true).unwrap_err();
        assert!(missing.to_string().contains("missing"));
        assert_eq!(fs::read(claude_tasks_path(home.path())).unwrap(), before);
        let codex = save(home.path(), AutomationProvider::Codex, None, "valid", "0 9 * * *", true).unwrap_err();
        assert!(codex.to_string().contains("does not expose native automation creation"));
    }

    #[test]
    fn claude_pause_is_refused_because_the_format_has_no_paused_state() {
        let home = fixture_home();
        write_claude_tasks(home.path(), serde_json::json!([{"id": "task-1", "cron": "7 9 * * *", "prompt": "a", "createdAt": 1}]));
        let error = execute(home.path(), AutomationProvider::Claude, "task-1", AutomationAction::Pause).unwrap_err();
        assert!(matches!(error, BridgeError::Invalid(_)));
    }

    #[test]
    fn cron_validation_refuses_expressions_claude_code_cannot_run() {
        for good in [
            "0 9 * * 1-5",
            "*/15 * * * *",
            "7 9 1,15 * *",
            "0 0 1 JAN *",
            "30 8 * * MON-FRI",
            "0 12 * * 7",
        ] {
            assert!(validate_cron(good).is_ok(), "{good} should be accepted");
        }
        for (bad, needle) in [
            // Five words is not five cron fields.
            ("every day at nine am", "minute"),
            ("60 9 * * *", "minute"),
            ("0 24 * * *", "hour"),
            ("0 9 0 * *", "day of month"),
            ("0 9 * 13 *", "month"),
            ("0 9 * * 8", "day of week"),
            ("0 9 * * MONDAY", "day of week"),
            ("*/0 * * * *", "minute"),
            ("9-5 9 * * *", "minute"),
            ("0 9 * *", "five-field"),
            ("0 9 * * * *", "five-field"),
            ("", "five-field"),
        ] {
            let error = validate_cron(bad).unwrap_err().to_string();
            assert!(error.contains(needle), "{bad:?} -> {error}");
        }
    }

    #[test]
    fn a_garbage_cron_never_reaches_claude_codes_schedule_file() {
        let home = fixture_home();
        let error = save(home.path(), AutomationProvider::Claude, None, "do a thing", "every day at nine am", true)
            .unwrap_err();
        assert!(error.to_string().contains("minute"), "{error}");
        assert!(!claude_tasks_path(home.path()).exists(), "no file written for a cron Claude cannot parse");
    }

    #[test]
    fn only_claude_has_a_write_path_even_if_a_capability_list_widens() {
        // The capability gate is bypassed here on purpose: this asserts the
        // second, independent guard that decides *which* store a save touches.
        let home = fixture_home();
        for provider in [AutomationProvider::Codex, AutomationProvider::Cursor, AutomationProvider::OpenCode] {
            let error = save(home.path(), provider, None, "valid", "0 9 * * *", true).unwrap_err();
            assert!(error.to_string().contains(provider.display_name()), "{provider:?}: {error}");
            assert!(!claude_tasks_path(home.path()).exists(), "{provider:?} must not write Claude's store");
        }
    }

    #[test]
    fn unsupported_run_now_is_explicit_for_every_provider() {
        let home = fixture_home();
        for provider in [
            AutomationProvider::Claude,
            AutomationProvider::Codex,
            AutomationProvider::Cursor,
            AutomationProvider::OpenCode,
        ] {
            let error = execute(home.path(), provider, "anything", AutomationAction::RunNow).unwrap_err();
            assert!(error.to_string().contains("run-now"), "{provider:?}: {error}");
        }
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
