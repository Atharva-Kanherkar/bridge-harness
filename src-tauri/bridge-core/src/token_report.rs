//! The token report: the main-window tokens of each tool class in each
//! top-level chat.
//!
//! The report reads the database and never writes to it. It opens the file
//! read-only, sets `query_only`, and runs all its queries in one read
//! transaction. A write from another connection after the first read does
//! not change the result. It never calls [`crate::store::open`], because that
//! function migrates the file.
//!
//! Each history row gives the tokens that a model reads from it: the stored
//! `token_estimate`, else [`entry_tokens::estimate`]. The rows of one item
//! make a group for each kind, and a group counts one time with its largest
//! estimate.
//!
//! The output has ids, harness names, kinds, numbers and timestamps. It never
//! has a label, a title, chat text, tool input or tool output.

use crate::entry_tokens::{self, ESTIMATED_KINDS};
use crate::BridgeError;
use chrono::{DateTime, Days, NaiveDate, NaiveTime, SecondsFormat, Utc};
use rusqlite::types::Value as SqlValue;
use rusqlite::{params, params_from_iter, Connection, OpenFlags, OptionalExtension, Row};
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

/// The class of the tokens of one group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TokenClass {
    Read,
    Search,
    Shell,
    Edit,
    Web,
    Mcp,
    Agent,
    OtherTool,
    Conversation,
    Bridge,
}

impl TokenClass {
    /// All classes, in the order of the output.
    pub const ALL: [TokenClass; 10] = [
        TokenClass::Read,
        TokenClass::Search,
        TokenClass::Shell,
        TokenClass::Edit,
        TokenClass::Web,
        TokenClass::Mcp,
        TokenClass::Agent,
        TokenClass::OtherTool,
        TokenClass::Conversation,
        TokenClass::Bridge,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            TokenClass::Read => "read",
            TokenClass::Search => "search",
            TokenClass::Shell => "shell",
            TokenClass::Edit => "edit",
            TokenClass::Web => "web",
            TokenClass::Mcp => "mcp",
            TokenClass::Agent => "agent",
            TokenClass::OtherTool => "other_tool",
            TokenClass::Conversation => "conversation",
            TokenClass::Bridge => "bridge",
        }
    }
}

/// The chats that the report compares.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Cohort {
    /// Top-level orchestrator chats of a workspace.
    Orchestrator,
    /// Top-level direct chats.
    Direct,
}

impl Cohort {
    pub fn as_str(self) -> &'static str {
        match self {
            Cohort::Orchestrator => "orchestrator",
            Cohort::Direct => "direct",
        }
    }

    /// The cohort of a `sessions.kind` value or of a `--cohort` value.
    pub fn parse(value: &str) -> Option<Cohort> {
        match value {
            "orchestrator" => Some(Cohort::Orchestrator),
            "direct" => Some(Cohort::Direct),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReportOptions {
    pub cohort: Cohort,
    /// A chat is recent when its last own message is at or after this day,
    /// at 00:00 UTC.
    pub since: NaiveDate,
    /// The count of rows that the report keeps.
    pub top: usize,
    /// One session only. The cohort comes from its kind, and the report
    /// does not apply recency.
    pub session: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub schema_version: i64,
    pub cohort: Cohort,
    pub since: String,
    /// The count of sessions before the report keeps the first `top`.
    pub eligible: usize,
    pub fallbacks: Vec<&'static str>,
    pub unreadable_timestamps: usize,
    pub sessions: Vec<SessionTokens>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionTokens {
    pub rank: usize,
    pub session_id: String,
    pub harness: String,
    pub chat_kind: String,
    pub total: i64,
    pub classes: BTreeMap<TokenClass, i64>,
    pub reading_share: f64,
    pub shell_share: f64,
    pub unestimated: u64,
    pub fork_prefix_entries: i64,
    pub fork_prefix_unknown: bool,
    pub last_activity_at: Option<String>,
    pub last_context_used: Option<i64>,
    pub max_context_used: Option<i64>,
}

/// The header text when the database has no fork columns.
pub const FALLBACK_NO_FORKS: &str =
    "no fork_parent_session_id or fork_parent_entry_id: no session is a fork";
/// The header text when the database has no provider epoch column.
pub const FALLBACK_NO_EPOCH: &str = "no context_usage_after_id: the value is 0";
/// The header text when the ledger has no window column.
pub const FALLBACK_NO_WINDOW: &str =
    "no usage_ledger.context_used_tokens: lastContextUsed and maxContextUsed are null";

/// Without one of these columns the report cannot count, so it stops.
const REQUIRED_COLUMNS: [(&str, &[&str]); 4] = [
    ("sessions", &["id", "workspace_id", "harness", "kind", "parent_session_id", "depth"]),
    ("worker_runtime", &["session_id"]),
    (
        "session_entries",
        &[
            "id",
            "session_id",
            "parent_entry_id",
            "sequence",
            "kind",
            "payload",
            "provider_event_id",
            "token_estimate",
            "created_at",
        ],
    ),
    ("usage_ledger", &["id", "session_id"]),
];

const CONVERSATION_KINDS: [&str; 4] =
    ["user.message", "assistant.message", "reasoning", "reasoning.completed"];

const BRIDGE_KINDS: [&str; 8] = [
    "session.context",
    "delegation.result",
    "context.compacted",
    "checkpoint",
    "compaction",
    "branch.summary",
    "handoff.brief",
    "worker.result",
];

/// The kinds of a tool row. The report reads the payload of these rows to
/// find the tool.
const TOOL_KINDS: [&str; 6] = [
    "tool.started",
    "command.started",
    "file_change.started",
    "tool.completed",
    "command.completed",
    "file_change.completed",
];

/// The kinds that show activity in a chat.
const ACTIVITY_KINDS: [&str; 2] = ["user.message", "assistant.message"];

/// Characters that make a command text a shell program. Quotes do not
/// change this, so `rg 'a|b'` is `shell`.
const SHELL_OPERATORS: [char; 10] = ['|', '&', ';', '<', '>', '(', ')', '$', '`', '\n'];

/// The length of the parent chain of an entry in its session. The row count
/// of the session stops a cycle.
const CHAIN_SQL: &str = "WITH RECURSIVE chain(id, parent, n) AS (
         SELECT id, parent_entry_id, 1 FROM session_entries WHERE id = ?1 AND session_id = ?2
         UNION ALL
         SELECT e.id, e.parent_entry_id, c.n + 1 FROM session_entries e JOIN chain c ON e.id = c.parent
         WHERE e.session_id = ?2 AND c.n < ?3)
     SELECT MAX(n) FROM chain";

/// Opens the database for the report: read-only, with no mutex, and with
/// `query_only` on. The file does not change.
pub fn open_read_only(path: &Path) -> Result<Connection, BridgeError> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|error| {
        BridgeError::Invalid(format!("cannot open {} read-only: {error}", path.display()))
    })?;
    connection.execute_batch("PRAGMA query_only=ON; PRAGMA busy_timeout=5000;")?;
    Ok(connection)
}

/// Counts the main-window tokens of each chat in the cohort.
pub fn report(db: &Connection, options: &ReportOptions) -> Result<Report, BridgeError> {
    build(db, options, &mut || {})
}

/// Reads a `YYYY-MM-DD` date.
pub fn parse_since(value: &str) -> Result<NaiveDate, BridgeError> {
    let shaped = value.len() == 10
        && value.bytes().enumerate().all(|(index, byte)| match index {
            4 | 7 => byte == b'-',
            _ => byte.is_ascii_digit(),
        });
    shaped
        .then(|| NaiveDate::parse_from_str(value, "%Y-%m-%d").ok())
        .flatten()
        .ok_or_else(|| {
            BridgeError::Invalid(format!("{value} is not a real date in the form YYYY-MM-DD"))
        })
}

/// The default start of the window: 30 days before `today`.
pub fn default_since(today: NaiveDate) -> NaiveDate {
    today.checked_sub_days(Days::new(30)).unwrap_or(NaiveDate::MIN)
}

/// The current date in UTC.
pub fn utc_today() -> NaiveDate {
    Utc::now().date_naive()
}

/// The class of a group of `kind`. `start` is a start row of the same item,
/// and `completion` is the row of the group. A row that names no tool gives
/// `other_tool`. The report never reads `title`.
pub fn classify_tool(kind: &str, start: Option<&Value>, completion: Option<&Value>) -> TokenClass {
    group_class(
        kind,
        start.and_then(|payload| named_class(payload, false)),
        completion.and_then(|payload| named_class(payload, kind == "command.completed")),
    )
}

/// The class of a shell command, a string or an argv array. The rule is
/// strict: a command that it does not know is `shell`.
pub fn classify_shell(command: &Value) -> TokenClass {
    match command {
        Value::String(text) => classify_text(text, true),
        Value::Array(items) => {
            let Some(words) = items.iter().map(Value::as_str).collect::<Option<Vec<&str>>>()
            else {
                return TokenClass::Shell;
            };
            match wrapped_script(&words) {
                Some(script) => classify_text(script, false),
                None => classify_words(&words),
            }
        }
        _ => TokenClass::Shell,
    }
}

/// The report as markdown: a header line, one line for each fallback, and
/// one table.
pub fn render_markdown(report: &Report) -> String {
    let mut out = format!(
        "Token report: schema {}, cohort {}, since {}, eligible {}, shown {}, unreadableTimestamps {}\n",
        report.schema_version,
        report.cohort.as_str(),
        report.since,
        report.eligible,
        report.sessions.len(),
        report.unreadable_timestamps,
    );
    for fallback in &report.fallbacks {
        out.push_str(&format!("Fallback: {fallback}\n"));
    }
    let mut columns = vec!["rank", "session", "harness", "total"];
    columns.extend(TokenClass::ALL.iter().map(|class| class.as_str()));
    columns.extend([
        "readingShare",
        "shellShare",
        "unestimated",
        "lastContextUsed",
        "maxContextUsed",
        "lastActivityAt",
        "notes",
    ]);
    out.push('\n');
    out.push_str(&table_line(columns.iter().map(|column| (*column).to_owned())));
    out.push_str(&table_line(columns.iter().map(|_| "---".to_owned())));
    for session in &report.sessions {
        out.push_str(&table_line(session_cells(session)));
    }
    out
}

/// The report as pretty JSON.
pub fn render_json(report: &Report) -> Result<String, BridgeError> {
    serde_json::to_string_pretty(report)
        .map_err(|error| BridgeError::Invalid(format!("cannot write the report as JSON: {error}")))
}

fn table_line(cells: impl Iterator<Item = String>) -> String {
    let mut line = String::from("|");
    for cell in cells {
        line.push(' ');
        line.push_str(&cell);
        line.push_str(" |");
    }
    line.push('\n');
    line
}

fn session_cells(session: &SessionTokens) -> impl Iterator<Item = String> {
    let mut cells = vec![
        session.rank.to_string(),
        session.session_id.clone(),
        session.harness.clone(),
        session.total.to_string(),
    ];
    cells.extend(
        TokenClass::ALL
            .iter()
            .map(|class| session.classes.get(class).copied().unwrap_or(0).to_string()),
    );
    let number = |value: Option<i64>| value.map_or_else(|| "-".to_owned(), |value| value.to_string());
    cells.extend([
        percent(session.reading_share),
        percent(session.shell_share),
        session.unestimated.to_string(),
        number(session.last_context_used),
        number(session.max_context_used),
        session.last_activity_at.clone().unwrap_or_else(|| "-".to_owned()),
        if session.fork_prefix_unknown { "fork-prefix-unknown" } else { "-" }.to_owned(),
    ]);
    cells.into_iter()
}

fn percent(share: f64) -> String {
    format!("{:.1}%", share * 100.0)
}

fn build(
    db: &Connection,
    options: &ReportOptions,
    after_first_read: &mut dyn FnMut(),
) -> Result<Report, BridgeError> {
    // BEGIN DEFERRED: the first read fixes the snapshot of the whole report.
    let transaction = db.unchecked_transaction()?;
    let schema = read_schema(&transaction)?;
    after_first_read();
    let session = options.session.as_deref();
    let cohort = match session {
        Some(id) => session_cohort(&transaction, id)?,
        None => options.cohort,
    };
    let candidates = candidates(&transaction, &schema, cohort, session)?;
    if let Some(id) = session {
        if candidates.is_empty() {
            return Err(not_in_cohort(id));
        }
    }
    let since = options.since.and_time(NaiveTime::MIN).and_utc();
    let mut unreadable_timestamps = 0;
    let mut rows = Vec::new();
    for candidate in candidates {
        let (prefix, prefix_unknown) = fork_prefix(&transaction, &candidate)?;
        let activity = match last_activity(&transaction, &candidate.id, prefix)? {
            Activity::At(at) => Some(at),
            Activity::Unreadable => {
                if session.is_none() {
                    unreadable_timestamps += 1;
                }
                None
            }
            Activity::Absent => None,
        };
        if session.is_none() && !activity.is_some_and(|at| at >= since) {
            continue;
        }
        let counted = count_session(&transaction, &candidate.id, prefix)?;
        let epoch = candidate.epoch;
        rows.push((session_tokens(candidate, counted, prefix, prefix_unknown, activity), epoch));
    }
    let eligible = rows.len();
    rows.sort_by(|(left, _), (right, _)| {
        right.total.cmp(&left.total).then_with(|| left.session_id.cmp(&right.session_id))
    });
    rows.truncate(options.top);
    let mut sessions = Vec::with_capacity(rows.len());
    for (index, (mut row, epoch)) in rows.into_iter().enumerate() {
        row.rank = index + 1;
        if schema.window {
            (row.last_context_used, row.max_context_used) =
                window(&transaction, &row.session_id, epoch)?;
        }
        sessions.push(row);
    }
    transaction.rollback()?;
    Ok(Report {
        schema_version: schema.version,
        cohort,
        since: options.since.format("%Y-%m-%d").to_string(),
        eligible,
        fallbacks: schema.fallbacks(),
        unreadable_timestamps,
        sessions,
    })
}

struct Schema {
    version: i64,
    forks: bool,
    epoch: bool,
    window: bool,
}

impl Schema {
    fn fallbacks(&self) -> Vec<&'static str> {
        [
            (!self.forks).then_some(FALLBACK_NO_FORKS),
            (!self.epoch).then_some(FALLBACK_NO_EPOCH),
            (!self.window).then_some(FALLBACK_NO_WINDOW),
        ]
        .into_iter()
        .flatten()
        .collect()
    }
}

fn read_schema(db: &Connection) -> Result<Schema, BridgeError> {
    // A newer schema stops the report before it reads a table.
    let version = crate::store::supported_schema_version(db)?;
    for (table, columns) in REQUIRED_COLUMNS {
        for column in columns {
            if !has_column(db, table, column)? {
                return Err(BridgeError::Invalid(format!(
                    "The database has no column {table}.{column}. Open it one time with a \
                     current Bridge build, then take a new backup."
                )));
            }
        }
    }
    Ok(Schema {
        version,
        forks: has_column(db, "sessions", "fork_parent_session_id")?
            && has_column(db, "sessions", "fork_parent_entry_id")?,
        epoch: has_column(db, "sessions", "context_usage_after_id")?,
        window: has_column(db, "usage_ledger", "context_used_tokens")?,
    })
}

fn has_column(db: &Connection, table: &str, column: &str) -> Result<bool, BridgeError> {
    Ok(db.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info(?1) WHERE name = ?2)",
        params![table, column],
        |row| row.get(0),
    )?)
}

fn session_cohort(db: &Connection, id: &str) -> Result<Cohort, BridgeError> {
    let kind: Option<String> = db
        .query_row("SELECT kind FROM sessions WHERE id = ?1", [id], |row| row.get(0))
        .optional()?;
    match kind {
        None => Err(BridgeError::Invalid(format!("No session has the id {id}."))),
        Some(kind) => Cohort::parse(&kind).ok_or_else(|| not_in_cohort(id)),
    }
}

fn not_in_cohort(id: &str) -> BridgeError {
    BridgeError::Invalid(format!(
        "The session {id} is not a top-level orchestrator chat or direct chat. The report \
         counts only these chats."
    ))
}

struct Candidate {
    id: String,
    harness: String,
    kind: String,
    fork_session: Option<String>,
    fork_entry: Option<String>,
    epoch: i64,
}

fn candidates(
    db: &Connection,
    schema: &Schema,
    cohort: Cohort,
    session: Option<&str>,
) -> Result<Vec<Candidate>, BridgeError> {
    // Each fragment is a fixed literal. No value enters the text of the query.
    let forks = if schema.forks {
        "s.fork_parent_session_id, s.fork_parent_entry_id"
    } else {
        "NULL, NULL"
    };
    let epoch = if schema.epoch { "s.context_usage_after_id" } else { "0" };
    let workspace = match cohort {
        Cohort::Orchestrator => "AND s.workspace_id IS NOT NULL",
        Cohort::Direct => "",
    };
    let sql = format!(
        "SELECT s.id, s.harness, s.kind, {forks}, {epoch} FROM sessions s
         WHERE s.kind = ?1 {workspace}
           AND s.parent_session_id IS NULL AND COALESCE(s.depth, 0) = 0
           AND NOT EXISTS (SELECT 1 FROM worker_runtime w WHERE w.session_id = s.id)
           AND (?2 IS NULL OR s.id = ?2)
         ORDER BY s.id"
    );
    let mut statement = db.prepare(&sql)?;
    let rows = statement.query_map(params![cohort.as_str(), session], |row| {
        Ok(Candidate {
            id: row.get(0)?,
            harness: row.get(1)?,
            kind: row.get(2)?,
            fork_session: text_at(row, 3)?,
            fork_entry: text_at(row, 4)?,
            epoch: integer_at(row, 5)?.unwrap_or(0),
        })
    })?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

/// The count of entries that the fork copied from its source, and true when
/// the report cannot find that count.
fn fork_prefix(db: &Connection, candidate: &Candidate) -> Result<(i64, bool), BridgeError> {
    let (source, entry) = match (&candidate.fork_session, &candidate.fork_entry) {
        (None, None) => return Ok((0, false)),
        (Some(source), Some(entry)) => (source, entry),
        _ => return Ok((0, true)),
    };
    let rows: i64 = db.query_row(
        "SELECT COUNT(*) FROM session_entries WHERE session_id = ?1",
        [source],
        |row| row.get(0),
    )?;
    let length: Option<i64> =
        db.query_row(CHAIN_SQL, params![entry, source, rows], |row| row.get(0))?;
    Ok(match length {
        Some(length) => (length, false),
        None => (0, true),
    })
}

enum Activity {
    Absent,
    At(DateTime<Utc>),
    Unreadable,
}

/// The time of the last own message, by sequence. The copied prefix of a
/// fork is not activity.
fn last_activity(db: &Connection, session: &str, prefix: i64) -> Result<Activity, BridgeError> {
    let value: Option<SqlValue> = db
        .query_row(
            "SELECT created_at FROM session_entries
             WHERE session_id = ?1 AND sequence > ?2 AND kind IN (?3, ?4)
             ORDER BY sequence DESC LIMIT 1",
            params![session, prefix, ACTIVITY_KINDS[0], ACTIVITY_KINDS[1]],
            |row| row.get(0),
        )
        .optional()?;
    Ok(match value {
        None => Activity::Absent,
        Some(SqlValue::Text(text)) => match DateTime::parse_from_rfc3339(&text) {
            Ok(at) => Activity::At(at.with_timezone(&Utc)),
            Err(_) => Activity::Unreadable,
        },
        Some(_) => Activity::Unreadable,
    })
}

/// The identity of one provider item in one session. The variant is the
/// source, so keys of two sources never merge.
#[derive(Clone, PartialEq, Eq, Hash)]
enum ItemKey {
    Call(String),
    Event(String),
    Entry(String),
}

struct Group {
    tokens: Option<i64>,
    /// True when a row of the group is not a preparing start.
    full: bool,
    named: Option<TokenClass>,
}

struct Counted {
    classes: BTreeMap<TokenClass, i64>,
    unestimated: u64,
}

fn count_session(db: &Connection, session: &str, prefix: i64) -> Result<Counted, BridgeError> {
    let tool_kinds = placeholders(3, TOOL_KINDS.len());
    let estimated_kinds = placeholders(3 + TOOL_KINDS.len(), ESTIMATED_KINDS.len());
    let sql = format!(
        "SELECT id, kind, token_estimate, provider_event_id,
                CASE WHEN json_valid(payload) THEN
                  CASE WHEN json_type(payload, '$.itemId') = 'text' THEN json_extract(payload, '$.itemId') END
                END,
                CASE WHEN token_estimate IS NULL OR kind IN ({tool_kinds}) THEN payload END
         FROM session_entries
         WHERE session_id = ?1 AND sequence > ?2 AND kind IN ({estimated_kinds})
         ORDER BY sequence"
    );
    let mut values = vec![SqlValue::Text(session.to_owned()), SqlValue::Integer(prefix)];
    values.extend(
        TOOL_KINDS
            .iter()
            .chain(ESTIMATED_KINDS.iter())
            .map(|kind| SqlValue::Text((*kind).to_owned())),
    );
    let mut statement = db.prepare(&sql)?;
    let mut rows = statement.query(params_from_iter(values))?;
    let mut groups: HashMap<(String, ItemKey), Group> = HashMap::new();
    let mut started: HashMap<ItemKey, TokenClass> = HashMap::new();
    while let Some(row) = rows.next()? {
        let id: String = row.get(0)?;
        let kind: String = row.get(1)?;
        let stored = integer_at(row, 2)?;
        let event = text_at(row, 3)?.filter(|value| !value.is_empty());
        let item = text_at(row, 4)?.filter(|value| !value.is_empty());
        let payload = text_at(row, 5)?
            .and_then(|text| serde_json::from_str::<Value>(&text).ok())
            .unwrap_or(Value::Null);
        let key = match (item, event) {
            (Some(item), _) => ItemKey::Call(item),
            (None, Some(event)) => ItemKey::Event(event),
            (None, None) => ItemKey::Entry(id),
        };
        let tokens = stored.or_else(|| entry_tokens::estimate(&kind, &payload));
        let start = kind.ends_with(".started");
        let preparing = start && entry_tokens::is_preparing_start(&payload);
        let named = TOOL_KINDS
            .contains(&kind.as_str())
            .then(|| named_class(&payload, kind == "command.completed"))
            .flatten();
        if let (true, Some(class)) = (start, named) {
            started.insert(key.clone(), class);
        }
        let group = groups.entry((kind, key)).or_insert(Group {
            tokens: None,
            full: false,
            named: None,
        });
        group.tokens = group.tokens.max(tokens);
        group.full |= !preparing;
        group.named = named.or(group.named);
    }
    let mut classes: BTreeMap<TokenClass, i64> =
        TokenClass::ALL.iter().map(|class| (*class, 0)).collect();
    let mut unestimated = 0;
    for ((kind, key), group) in groups {
        let class = group_class(&kind, started.get(&key).copied(), group.named);
        match group.tokens {
            Some(tokens) => {
                let slot = classes.entry(class).or_insert(0);
                *slot = slot.saturating_add(tokens);
            }
            None if group.full => unestimated += 1,
            None => {}
        }
    }
    Ok(Counted { classes, unestimated })
}

fn session_tokens(
    candidate: Candidate,
    counted: Counted,
    prefix: i64,
    prefix_unknown: bool,
    activity: Option<DateTime<Utc>>,
) -> SessionTokens {
    let total = counted.classes.values().fold(0i64, |sum, tokens| sum.saturating_add(*tokens));
    let share = |tokens: i64| if total == 0 { 0.0 } else { tokens as f64 / total as f64 };
    let class = |class: TokenClass| counted.classes.get(&class).copied().unwrap_or(0);
    SessionTokens {
        rank: 0,
        session_id: candidate.id,
        harness: candidate.harness,
        chat_kind: candidate.kind,
        total,
        reading_share: share(class(TokenClass::Read).saturating_add(class(TokenClass::Search))),
        shell_share: share(class(TokenClass::Shell)),
        classes: counted.classes,
        unestimated: counted.unestimated,
        fork_prefix_entries: prefix,
        fork_prefix_unknown: prefix_unknown,
        last_activity_at: activity.map(|at| at.to_rfc3339_opts(SecondsFormat::Secs, true)),
        last_context_used: None,
        max_context_used: None,
    }
}

/// The last and the largest window of the current provider epoch, by ledger
/// id.
fn window(
    db: &Connection,
    session: &str,
    epoch: i64,
) -> Result<(Option<i64>, Option<i64>), BridgeError> {
    let mut statement = db.prepare(
        "SELECT context_used_tokens FROM usage_ledger
         WHERE session_id = ?1 AND id > ?2 AND context_used_tokens IS NOT NULL
         ORDER BY id",
    )?;
    let mut rows = statement.query(params![session, epoch])?;
    let (mut last, mut max) = (None, None);
    while let Some(row) = rows.next()? {
        let Some(value) = integer_at(row, 0)? else {
            continue;
        };
        last = Some(value);
        max = Some(max.map_or(value, |largest: i64| largest.max(value)));
    }
    Ok((last, max))
}

fn placeholders(first: usize, count: usize) -> String {
    (first..first + count).map(|index| format!("?{index}")).collect::<Vec<_>>().join(", ")
}

fn text_at(row: &Row<'_>, index: usize) -> rusqlite::Result<Option<String>> {
    Ok(match row.get::<_, SqlValue>(index)? {
        SqlValue::Text(text) => Some(text),
        SqlValue::Integer(number) => Some(number.to_string()),
        _ => None,
    })
}

fn integer_at(row: &Row<'_>, index: usize) -> rusqlite::Result<Option<i64>> {
    Ok(match row.get::<_, SqlValue>(index)? {
        SqlValue::Integer(number) => Some(number),
        _ => None,
    })
}

/// The class from the kind, else from a start of the item, else from the
/// fields of the row, else `other_tool`.
fn group_class(kind: &str, start: Option<TokenClass>, own: Option<TokenClass>) -> TokenClass {
    kind_class(kind).or(start).or(own).unwrap_or(TokenClass::OtherTool)
}

fn kind_class(kind: &str) -> Option<TokenClass> {
    if CONVERSATION_KINDS.contains(&kind) {
        Some(TokenClass::Conversation)
    } else if BRIDGE_KINDS.contains(&kind) {
        Some(TokenClass::Bridge)
    } else if kind.starts_with("file_change.") {
        Some(TokenClass::Edit)
    } else {
        None
    }
}

/// The class that the fields of a tool row name, in this order: Codex
/// `data.type`, OpenCode `data.tool`, Claude and import `data.name`, and
/// `data.command` on a Bash completion. `title` is free text, so the report
/// never reads it.
fn named_class(payload: &Value, command_rule: bool) -> Option<TokenClass> {
    let data = payload.get("data")?;
    let codex = data.get("type").and_then(Value::as_str);
    if let Some(class) = codex.and_then(|item_type| codex_class(item_type, data)) {
        return Some(class);
    }
    if let Some(tool) = data.get("tool").and_then(Value::as_str) {
        return Some(opencode_class(tool, data));
    }
    if let Some(name) = data.get("name").and_then(Value::as_str) {
        return Some(claude_class(name, data));
    }
    if command_rule {
        return data.get("command").filter(|command| !command.is_null()).map(classify_shell);
    }
    None
}

fn codex_class(item_type: &str, data: &Value) -> Option<TokenClass> {
    Some(match item_type {
        "commandExecution" => classify_shell(data.get("command").unwrap_or(&Value::Null)),
        "mcpToolCall" => TokenClass::Mcp,
        "webSearch" => TokenClass::Web,
        "collabAgentToolCall" => TokenClass::Agent,
        "fileChange" => TokenClass::Edit,
        "dynamicToolCall" => TokenClass::OtherTool,
        _ => return None,
    })
}

fn opencode_class(tool: &str, data: &Value) -> TokenClass {
    match tool {
        "read" => TokenClass::Read,
        "grep" | "glob" | "list" => TokenClass::Search,
        "bash" | "shell" => {
            classify_shell(data.pointer("/state/input/command").unwrap_or(&Value::Null))
        }
        "edit" | "write" | "patch" | "multiedit" | "apply_patch" => TokenClass::Edit,
        "webfetch" | "websearch" => TokenClass::Web,
        "task" => TokenClass::Agent,
        other if other.starts_with("bridge_") => TokenClass::Mcp,
        _ => TokenClass::OtherTool,
    }
}

fn claude_class(name: &str, data: &Value) -> TokenClass {
    match name {
        "Read" | "NotebookRead" => TokenClass::Read,
        "Grep" | "Glob" | "LS" => TokenClass::Search,
        "Bash" => classify_shell(data.pointer("/input/command").unwrap_or(&Value::Null)),
        "Edit" | "Write" | "MultiEdit" | "NotebookEdit" => TokenClass::Edit,
        "WebFetch" | "WebSearch" => TokenClass::Web,
        "Task" | "Agent" => TokenClass::Agent,
        other if other.starts_with("mcp__") => TokenClass::Mcp,
        _ => TokenClass::OtherTool,
    }
}

/// `unwrap` is false for the script of a wrapper: the rule unwraps one time.
fn classify_text(text: &str, unwrap: bool) -> TokenClass {
    if text.contains(&SHELL_OPERATORS[..]) {
        return TokenClass::Shell;
    }
    let Ok(owned) = shell_words::split(text) else {
        return TokenClass::Shell;
    };
    let words: Vec<&str> = owned.iter().map(String::as_str).collect();
    if unwrap {
        if let Some(script) = wrapped_script(&words) {
            return classify_text(script, false);
        }
    }
    classify_words(&words)
}

/// The script of `sh`, `bash` or `zsh` with `-c`, `-lc` or `-cl`.
fn wrapped_script<'a>(words: &[&'a str]) -> Option<&'a str> {
    let [shell, flag, script] = words else {
        return None;
    };
    let known = matches!(basename(shell), "sh" | "bash" | "zsh")
        && matches!(*flag, "-c" | "-lc" | "-cl");
    known.then_some(*script)
}

fn classify_words(words: &[&str]) -> TokenClass {
    let Some((first, args)) = words.split_first() else {
        return TokenClass::Shell;
    };
    if is_assignment(first) {
        return TokenClass::Shell;
    }
    match basename(first) {
        "cat" | "head" | "tail" | "nl" | "wc" => TokenClass::Read,
        "sed" if is_sed_print(args) => TokenClass::Read,
        "git" => git_class(args),
        "rg" if !args.iter().any(|arg| arg.starts_with("--pre")) => TokenClass::Search,
        "grep" | "ls" => TokenClass::Search,
        "fd" if !args.iter().any(|arg| is_fd_exec(arg)) => TokenClass::Search,
        "find" if !args.iter().any(|arg| is_find_action(arg)) => TokenClass::Search,
        _ => TokenClass::Shell,
    }
}

fn basename(word: &str) -> &str {
    word.rsplit('/').next().unwrap_or(word)
}

/// A first word of the form `NAME=value`.
fn is_assignment(word: &str) -> bool {
    let Some((name, _)) = word.split_once('=') else {
        return false;
    };
    let mut chars = name.chars();
    chars.next().is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
        && chars.all(|next| next.is_ascii_alphanumeric() || next == '_')
}

/// `sed -n '<N>p'` or `sed -n '<N>,<M>p'`, plus one path or more.
fn is_sed_print(args: &[&str]) -> bool {
    let [flag, script, paths @ ..] = args else {
        return false;
    };
    let lines = script.strip_suffix('p').is_some_and(|range| match range.split_once(',') {
        Some((first, last)) => is_number(first) && is_number(last),
        None => is_number(range),
    });
    *flag == "-n" && lines && !paths.is_empty() && paths.iter().all(|path| !path.starts_with('-'))
}

fn is_number(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit())
}

fn git_class(args: &[&str]) -> TokenClass {
    let Some((command, rest)) = args.split_first() else {
        return TokenClass::Shell;
    };
    match *command {
        "show" | "diff" | "log" | "blame" if !rest.iter().any(|arg| arg.starts_with("--output")) => {
            TokenClass::Read
        }
        "grep" | "ls-files" => TokenClass::Search,
        _ => TokenClass::Shell,
    }
}

/// `-x`, `-X`, `--exec`, `--exec-batch`, or a group of short flags with `x`
/// or `X`.
fn is_fd_exec(arg: &str) -> bool {
    if let Some(long) = arg.strip_prefix("--") {
        return long.starts_with("exec");
    }
    arg.starts_with('-') && arg.contains(['x', 'X'])
}

fn is_find_action(arg: &str) -> bool {
    arg.starts_with("-exec")
        || arg.starts_with("-ok")
        || arg.starts_with("-fprint")
        || arg == "-delete"
        || arg == "-fls"
}

#[cfg(test)]
mod tests;
