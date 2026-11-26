//! Screen Time: how much of the day Bridge took, and how much work the agents
//! did in it. Contract: `testing/feat-usage-screen-time.md`.
//!
//! Both kinds of time are stored as whole minutes, so recording one twice
//! changes nothing and two windows never double count.
//!
//! - **Your time** comes from the webview: it reports, through
//!   `usage/record_presence`, each minute the window was focused, visible and
//!   in use. Nothing can recover it after the fact, so it starts at zero.
//! - **Agent time** comes from the session forest: a trigger notes the minute
//!   of every entry a chat writes, except the person's own messages. Within a
//!   chat, a gap of up to `GAP_FILL_MINUTES` silent minutes still counts, so a
//!   long quiet tool run is not lost. Chats add up: two chats working through
//!   the same minute are two minutes of agent work.
//!
//! The range is also split by project, pull request, local hour and harness.
//! Contract: `testing/feat-screen-time-breakdown.md`. Your time per project
//! comes from the chat the webview reports open during each minute
//! (`human_activity_chats`, migration 100); agent time per project from the
//! chat's workspace; pull requests from `session_pull_requests`.
//!
//! Entries written before this existed are counted by a backfill. A full scan
//! of `session_entries` reads every payload page (the timestamp is stored
//! after the payload) and took 33 s on a 7.9 GB history, so it never runs on
//! boot and never reads under the core mutex: it starts the first time the
//! Time tab asks, reads through its own read-only connection in rowid
//! batches, and only takes the core connection for each short write.

use crate::{BridgeCore, BridgeError};
use bridge_protocol::messages as wire;
use chrono::{DateTime, NaiveDate, Timelike, Utc};
use chrono_tz::Tz;
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

/// Silent minutes inside one chat that still count as working.
pub const GAP_FILL_MINUTES: i64 = 2;
/// The most minutes one `usage/record_presence` call may carry: one day.
pub const MAX_PRESENCE_MINUTES: usize = 1_440;
/// How far back a reported minute is still accepted.
const PRESENCE_MAX_AGE_MINUTES: i64 = 7 * 1_440;
/// The longest range `usage/screen_time` answers.
pub const MAX_RANGE_DAYS: i64 = 92;
/// The longest chat id `usage/record_presence` accepts.
const MAX_CHAT_ID_CHARS: usize = 128;
/// Pull requests listed under `recent`.
pub const RECENT_PULL_REQUESTS: usize = 6;
/// What a chat with no project is called.
pub const NO_PROJECT: &str = "No project";
/// Session entries read per backfill batch.
const BACKFILL_BATCH: i64 = 20_000;
/// Pause between backfill batches, so a running chat's writes interleave.
const BACKFILL_PAUSE: Duration = Duration::from_millis(50);

/// Session kinds that are the person's own work: chats, orchestrators and
/// their workers, and harness subagents. Briefings, suggestions, extraction,
/// connector runs, chat search and imported history never count.
const COUNTED_KINDS_SQL: &str =
    "(s.kind IS NULL OR s.kind IN ('direct', 'orchestrator', 'harness_subagent'))";

/// Migration 88: both minute tables, the backfill cursor, and the trigger.
pub fn install_store(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS human_activity_minutes (
            minute INTEGER PRIMARY KEY
        );
        CREATE TABLE IF NOT EXISTS agent_activity_minutes (
            session_id TEXT NOT NULL,
            minute INTEGER NOT NULL,
            PRIMARY KEY(session_id, minute)
        ) WITHOUT ROWID;
        CREATE TABLE IF NOT EXISTS screen_time_backfill (
            id INTEGER PRIMARY KEY CHECK(id = 1),
            cursor INTEGER NOT NULL,
            end_rowid INTEGER NOT NULL
        );
        INSERT OR IGNORE INTO screen_time_backfill(id, cursor, end_rowid)
            SELECT 1, 0, COALESCE(MAX(rowid), 0) FROM session_entries;
        CREATE TRIGGER IF NOT EXISTS session_entries_agent_minutes
        AFTER INSERT ON session_entries
        WHEN NEW.kind <> 'user.message' AND strftime('%s', NEW.created_at) IS NOT NULL
        BEGIN
            INSERT OR IGNORE INTO agent_activity_minutes(session_id, minute)
            VALUES (NEW.session_id, CAST(strftime('%s', NEW.created_at) AS INTEGER) / 60);
        END;",
    )?;
    Ok(())
}

/// Migration 100: which chat was open during a minute of your time. One chat
/// per minute; the first one stored keeps it.
pub fn install_chat_store(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS human_activity_chats (
            minute INTEGER PRIMARY KEY,
            session_id TEXT NOT NULL
        );",
    )?;
    Ok(())
}

/// Stores the minutes the person was active, and the chat open in some of
/// them. Minutes after `now_minute` or older than a week are dropped; a chat
/// minute is kept only when the same minute is accepted from `minutes`.
/// Returns how many minutes were new.
pub fn record_presence(db: &Connection, minutes: &[i64], chats: &[wire::PresenceChat], now_minute: i64) -> Result<i64, BridgeError> {
    if minutes.len() > MAX_PRESENCE_MINUTES {
        return Err(BridgeError::Invalid(format!(
            "record_presence takes at most {MAX_PRESENCE_MINUTES} minutes per call, got {}",
            minutes.len()
        )));
    }
    let chat_minutes: usize = chats.iter().map(|chat| chat.minutes.len()).sum();
    if chat_minutes > MAX_PRESENCE_MINUTES {
        return Err(BridgeError::Invalid(format!(
            "record_presence takes at most {MAX_PRESENCE_MINUTES} chat minutes per call, got {chat_minutes}"
        )));
    }
    if let Some(chat) = chats.iter().find(|chat| chat.session_id.trim().is_empty() || chat.session_id.chars().count() > MAX_CHAT_ID_CHARS) {
        return Err(BridgeError::Invalid(format!(
            "a chat id must be 1 to {MAX_CHAT_ID_CHARS} characters, got {} characters",
            chat.session_id.chars().count()
        )));
    }
    let oldest = now_minute - PRESENCE_MAX_AGE_MINUTES;
    let accepted: BTreeSet<i64> = minutes.iter().copied().filter(|minute| (oldest..=now_minute).contains(minute)).collect();
    let mut statement = db.prepare_cached("INSERT OR IGNORE INTO human_activity_minutes(minute) VALUES (?1)")?;
    let mut recorded = 0;
    for minute in &accepted {
        recorded += i64::try_from(statement.execute([minute])?).unwrap_or(0);
    }
    let mut chat_statement = db.prepare_cached("INSERT OR IGNORE INTO human_activity_chats(minute, session_id) VALUES (?1, ?2)")?;
    for chat in chats {
        for minute in chat.minutes.iter().filter(|minute| accepted.contains(minute)) {
            chat_statement.execute(params![minute, chat.session_id])?;
        }
    }
    Ok(recorded)
}

struct Range {
    zone: Tz,
    since: NaiveDate,
    until: NaiveDate,
}

impl Range {
    fn parse(params: &wire::ScreenTimeParams) -> Result<Self, BridgeError> {
        let zone: Tz = match params.time_zone.as_deref().map(str::trim).filter(|zone| !zone.is_empty()) {
            None => chrono_tz::UTC,
            Some(name) => name
                .parse()
                .map_err(|_| BridgeError::Invalid(format!("unknown time zone {name:?}")))?,
        };
        let day = |value: &str, field: &str| {
            NaiveDate::parse_from_str(value.trim(), "%Y-%m-%d")
                .map_err(|_| BridgeError::Invalid(format!("{field} must be YYYY-MM-DD, got {value:?}")))
        };
        let since = day(&params.since_day, "sinceDay")?;
        let until = day(&params.until_day, "untilDay")?;
        if until < since {
            return Err(BridgeError::Invalid("untilDay is before sinceDay".into()));
        }
        if (until - since).num_days() + 1 > MAX_RANGE_DAYS {
            return Err(BridgeError::Invalid(format!("screen time covers at most {MAX_RANGE_DAYS} days")));
        }
        Ok(Self { zone, since, until })
    }

    fn local(&self, minute: i64) -> Option<DateTime<Tz>> {
        DateTime::<Utc>::from_timestamp(minute.checked_mul(60)?, 0).map(|at| at.with_timezone(&self.zone))
    }

    fn local_day(&self, minute: i64) -> Option<NaiveDate> {
        self.local(minute).map(|at| at.date_naive())
    }

    /// The local day and hour of `minute` when its day is in the range.
    fn place(&self, minute: i64) -> Option<(NaiveDate, usize)> {
        let at = self.local(minute)?;
        let day = at.date_naive();
        (self.since..=self.until).contains(&day).then(|| (day, at.hour() as usize))
    }

    /// A UTC envelope around the range, a day wider on each side than any
    /// zone offset; the exact test is `local_day`.
    fn coarse_minutes(&self) -> (i64, i64) {
        let start = self.since.pred_opt().and_then(|day| day.pred_opt()).unwrap_or(self.since);
        let end = self.until.succ_opt().and_then(|day| day.succ_opt()).unwrap_or(self.until);
        let minute = |day: NaiveDate| day.and_hms_opt(0, 0, 0).map_or(0, |at| at.and_utc().timestamp() / 60);
        (minute(start), minute(end))
    }
}

/// One project's running figures for the range.
#[derive(Default)]
struct ProjectTally {
    name: String,
    human_seconds: i64,
    agent_seconds: i64,
    chats: i64,
    pull_requests: i64,
}

/// Figures per project, keyed by project id (`None` for no project).
#[derive(Default)]
struct Projects(BTreeMap<Option<String>, ProjectTally>);

impl Projects {
    fn entry(&mut self, id: Option<String>, name: Option<String>) -> &mut ProjectTally {
        let tally = self.0.entry(id.clone()).or_default();
        if tally.name.is_empty() {
            tally.name = match (id, name) {
                (Some(_), Some(name)) if !name.trim().is_empty() => name,
                _ => NO_PROJECT.to_owned(),
            };
        }
        tally
    }

    fn into_wire(self) -> Vec<wire::ScreenTimeProject> {
        let mut projects: Vec<wire::ScreenTimeProject> = self
            .0
            .into_iter()
            .filter(|(_, tally)| tally.human_seconds + tally.agent_seconds > 0 || tally.pull_requests > 0)
            .map(|(project_id, tally)| wire::ScreenTimeProject {
                project_id,
                name: tally.name,
                human_seconds: tally.human_seconds,
                agent_seconds: tally.agent_seconds,
                chats: tally.chats,
                pull_requests: tally.pull_requests,
            })
            .collect();
        projects.sort_by(|a, b| {
            (b.human_seconds + b.agent_seconds)
                .cmp(&(a.human_seconds + a.agent_seconds))
                .then_with(|| a.project_id.is_none().cmp(&b.project_id.is_none()))
                .then_with(|| a.name.cmp(&b.name))
                .then_with(|| a.project_id.cmp(&b.project_id))
        });
        projects
    }
}

/// The chat an agent minute belongs to, as far as the breakdown cares.
struct AgentChat {
    id: String,
    harness: String,
    top_level: bool,
    project_id: Option<String>,
    project_name: Option<String>,
}

/// The Time tab's figures: one entry per day in the range, the all-time
/// totals, and the range split by project, pull request, hour and harness.
pub fn screen_time(db: &Connection, params: &wire::ScreenTimeParams) -> Result<wire::UsageScreenTimeResult, BridgeError> {
    let range = Range::parse(params)?;
    let mut days: BTreeMap<NaiveDate, (i64, i64)> = BTreeMap::new();
    let mut day = range.since;
    while day <= range.until {
        days.insert(day, (0, 0));
        day = match day.succ_opt() {
            Some(next) => next,
            None => break,
        };
    }
    let mut hours = [(0_i64, 0_i64); 24];
    let mut projects = Projects::default();
    let mut harnesses: BTreeMap<String, (i64, i64)> = BTreeMap::new();

    let (human_minutes, first_human): (i64, Option<i64>) =
        db.query_row("SELECT COUNT(*), MIN(minute) FROM human_activity_minutes", [], |row| Ok((row.get(0)?, row.get(1)?)))?;
    let (low, high) = range.coarse_minutes();
    let mut statement = db.prepare("SELECT minute FROM human_activity_minutes WHERE minute >= ?1 AND minute < ?2")?;
    for minute in statement.query_map(params![low, high], |row| row.get::<_, i64>(0))? {
        if let Some((day, hour)) = range.place(minute?) {
            if let Some(cell) = days.get_mut(&day) {
                cell.0 += 60;
            }
            hours[hour].0 += 60;
        }
    }

    // Your time per project: minutes recorded with a chat open. Any chat
    // counts here, whatever its kind: it is where you were looking.
    let mut statement = db.prepare(
        "SELECT c.minute, p.id, p.name FROM human_activity_chats c
         JOIN human_activity_minutes m ON m.minute = c.minute
         LEFT JOIN sessions s ON s.id = c.session_id
         LEFT JOIN workspaces w ON w.id = s.workspace_id
         LEFT JOIN projects p ON p.id = w.project_id
         WHERE c.minute >= ?1 AND c.minute < ?2",
    )?;
    let mut rows = statement.query(params![low, high])?;
    while let Some(row) = rows.next()? {
        if range.place(row.get(0)?).is_some() {
            projects.entry(row.get(1)?, row.get(2)?).human_seconds += 60;
        }
    }

    let mut agent_minutes = 0_i64;
    let mut first_agent: Option<i64> = None;
    let mut counted_chat: Option<String> = None;
    for_each_agent_minute(db, |chat, minute| {
        agent_minutes += 1;
        first_agent = Some(first_agent.map_or(minute, |first| first.min(minute)));
        if !(low..high).contains(&minute) {
            return;
        }
        let Some((day, hour)) = range.place(minute) else { return };
        if let Some(cell) = days.get_mut(&day) {
            cell.1 += 60;
        }
        hours[hour].1 += 60;
        let first_in_range = counted_chat.as_deref() != Some(chat.id.as_str());
        if first_in_range {
            counted_chat = Some(chat.id.clone());
        }
        let new_chat = i64::from(first_in_range && chat.top_level);
        let project = projects.entry(chat.project_id.clone(), chat.project_name.clone());
        project.agent_seconds += 60;
        project.chats += new_chat;
        let harness = harnesses.entry(chat.harness.clone()).or_default();
        harness.0 += 60;
        harness.1 += new_chat;
    })?;

    let pull_requests = pull_requests(db, &range, &mut projects)?;

    let mut harnesses: Vec<wire::ScreenTimeHarness> = harnesses
        .into_iter()
        .filter(|(_, (seconds, _))| *seconds > 0)
        .map(|(harness, (agent_seconds, chats))| wire::ScreenTimeHarness { harness, agent_seconds, chats })
        .collect();
    harnesses.sort_by(|a, b| b.agent_seconds.cmp(&a.agent_seconds).then_with(|| a.harness.cmp(&b.harness)));

    let since_day = |minute: Option<i64>| minute.and_then(|minute| range.local_day(minute)).map(|day| day.to_string());
    Ok(wire::UsageScreenTimeResult {
        since_day: range.since.to_string(),
        until_day: range.until.to_string(),
        time_zone: range.zone.name().to_owned(),
        days: days
            .into_iter()
            .map(|(day, (human_seconds, agent_seconds))| wire::ScreenTimeDay { day: day.to_string(), human_seconds, agent_seconds })
            .collect(),
        all_time: wire::ScreenTimeTotals {
            human_seconds: human_minutes * 60,
            agent_seconds: agent_minutes * 60,
            human_since_day: since_day(first_human),
            agent_since_day: since_day(first_agent),
        },
        agent_history: if backfill_complete(db)? {
            wire::ScreenTimeAgentHistory::Complete
        } else {
            wire::ScreenTimeAgentHistory::Counting
        },
        projects: projects.into_wire(),
        pull_requests,
        hours: hours
            .iter()
            .enumerate()
            .map(|(hour, (human_seconds, agent_seconds))| wire::ScreenTimeHour {
                hour: u8::try_from(hour).unwrap_or(u8::MAX),
                human_seconds: *human_seconds,
                agent_seconds: *agent_seconds,
            })
            .collect(),
        harnesses,
    })
}

/// Calls `visit` once per counted agent minute with the chat it belongs to,
/// gaps of up to `GAP_FILL_MINUTES` inside one chat filled in. Minutes come
/// grouped by chat, oldest first.
fn for_each_agent_minute(db: &Connection, mut visit: impl FnMut(&AgentChat, i64)) -> Result<(), BridgeError> {
    let mut statement = db.prepare(&format!(
        "SELECT a.session_id, a.minute, s.harness, s.parent_session_id IS NULL, p.id, p.name
         FROM agent_activity_minutes a
         JOIN sessions s ON s.id = a.session_id
         LEFT JOIN workspaces w ON w.id = s.workspace_id
         LEFT JOIN projects p ON p.id = w.project_id
         WHERE {COUNTED_KINDS_SQL}
         ORDER BY a.session_id, a.minute"
    ))?;
    let mut rows = statement.query([])?;
    let mut chat: Option<AgentChat> = None;
    let mut last_minute = i64::MIN;
    while let Some(row) = rows.next()? {
        let minute: i64 = row.get(1)?;
        let same_chat = match (&chat, row.get_ref(0)?.as_str()) {
            (Some(chat), Ok(id)) => chat.id == id,
            _ => false,
        };
        if same_chat {
            if minute - last_minute - 1 <= GAP_FILL_MINUTES {
                if let Some(chat) = &chat {
                    for silent in (last_minute + 1)..minute {
                        visit(chat, silent);
                    }
                }
            }
        } else {
            chat = Some(AgentChat {
                id: row.get(0)?,
                harness: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                top_level: row.get::<_, Option<bool>>(3)?.unwrap_or(true),
                project_id: row.get(4)?,
                project_name: row.get(5)?,
            });
        }
        if let Some(chat) = &chat {
            visit(chat, minute);
        }
        last_minute = minute;
    }
    Ok(())
}

/// One pull request, merged from every chat row that links it.
struct PullRequestTally {
    repo: String,
    number: i64,
    opened_at: i64,
    project_id: Option<String>,
    project_name: Option<String>,
    fetched_at: Option<i64>,
    title: String,
    url: String,
    state: wire::ScreenTimePullRequestState,
}

fn pull_request_state(state: Option<&str>) -> wire::ScreenTimePullRequestState {
    match state {
        Some("open") => wire::ScreenTimePullRequestState::Open,
        Some("merged") => wire::ScreenTimePullRequestState::Merged,
        Some("closed") => wire::ScreenTimePullRequestState::Closed,
        _ => wire::ScreenTimePullRequestState::Unknown,
    }
}

/// Pull requests agents opened (`gh pr create` in a chat) on a local day in
/// the range, one per repository and number. The opened day and project come
/// from the earliest link; title, URL and state from the latest fetch.
fn pull_requests(db: &Connection, range: &Range, projects: &mut Projects) -> Result<wire::ScreenTimePullRequests, BridgeError> {
    let mut statement = db.prepare(
        "SELECT r.repo_host, r.repo_owner, r.repo_name, r.number,
                CAST(strftime('%s', r.created_at) AS INTEGER),
                CAST(strftime('%s', r.fetched_at) AS INTEGER),
                COALESCE(CASE WHEN json_valid(r.snapshot) THEN json_extract(r.snapshot, '$.title') END, r.title),
                COALESCE(CASE WHEN json_valid(r.snapshot) THEN json_extract(r.snapshot, '$.url') END, r.url),
                CASE WHEN json_valid(r.snapshot) THEN json_extract(r.snapshot, '$.state') END,
                p.id, p.name
         FROM session_pull_requests r
         LEFT JOIN sessions s ON s.id = r.session_id
         LEFT JOIN workspaces w ON w.id = s.workspace_id
         LEFT JOIN projects p ON p.id = w.project_id
         WHERE r.attribution = ?1",
    )?;
    let mut tallies: BTreeMap<(String, String, String, i64), PullRequestTally> = BTreeMap::new();
    let mut rows = statement.query([crate::session_prs::ATTRIBUTION_TOOL_COMPLETION])?;
    while let Some(row) = rows.next()? {
        let (host, owner, name, number): (String, String, String, i64) = (row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?);
        let Some(opened_at) = row.get::<_, Option<i64>>(4)? else { continue };
        let fetched_at: Option<i64> = row.get(5)?;
        let link = PullRequestTally {
            repo: format!("{owner}/{name}"),
            number,
            opened_at,
            project_id: row.get(9)?,
            project_name: row.get(10)?,
            fetched_at,
            title: row.get::<_, Option<String>>(6)?.unwrap_or_default(),
            url: row.get::<_, Option<String>>(7)?.unwrap_or_default(),
            state: pull_request_state(row.get::<_, Option<String>>(8)?.as_deref()),
        };
        match tallies.entry((host, owner, name, number)) {
            std::collections::btree_map::Entry::Vacant(slot) => {
                slot.insert(link);
            }
            std::collections::btree_map::Entry::Occupied(mut slot) => {
                let tally = slot.get_mut();
                if link.opened_at < tally.opened_at {
                    tally.opened_at = link.opened_at;
                    tally.project_id = link.project_id.clone();
                    tally.project_name = link.project_name.clone();
                }
                if link.fetched_at > tally.fetched_at {
                    tally.fetched_at = link.fetched_at;
                    tally.title = link.title;
                    tally.url = link.url;
                    tally.state = link.state;
                }
            }
        }
    }

    let mut summary = wire::ScreenTimePullRequests { opened: 0, merged: 0, open: 0, closed: 0, repos: Vec::new(), recent: Vec::new() };
    let mut repos: BTreeMap<String, (i64, i64)> = BTreeMap::new();
    let mut opened: Vec<(i64, wire::ScreenTimePullRequest)> = Vec::new();
    for tally in tallies.into_values() {
        let Some((day, _)) = range.place(tally.opened_at.div_euclid(60)) else { continue };
        summary.opened += 1;
        let merged = tally.state == wire::ScreenTimePullRequestState::Merged;
        match tally.state {
            wire::ScreenTimePullRequestState::Merged => summary.merged += 1,
            wire::ScreenTimePullRequestState::Open => summary.open += 1,
            wire::ScreenTimePullRequestState::Closed => summary.closed += 1,
            wire::ScreenTimePullRequestState::Unknown => {}
        }
        let repo = repos.entry(tally.repo.clone()).or_default();
        repo.0 += 1;
        repo.1 += i64::from(merged);
        projects.entry(tally.project_id, tally.project_name).pull_requests += 1;
        opened.push((
            tally.opened_at,
            wire::ScreenTimePullRequest {
                repo: tally.repo,
                number: tally.number,
                title: tally.title,
                url: tally.url,
                state: tally.state,
                opened_day: day.to_string(),
            },
        ));
    }
    summary.repos = repos.into_iter().map(|(repo, (opened, merged))| wire::ScreenTimeRepo { repo, opened, merged }).collect();
    summary.repos.sort_by(|a, b| b.opened.cmp(&a.opened).then_with(|| a.repo.cmp(&b.repo)));
    opened.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.repo.cmp(&b.1.repo)).then_with(|| b.1.number.cmp(&a.1.number)));
    summary.recent = opened.into_iter().take(RECENT_PULL_REQUESTS).map(|(_, pull_request)| pull_request).collect();
    Ok(summary)
}

fn backfill_complete(db: &Connection) -> Result<bool, BridgeError> {
    let state: Option<(i64, i64)> = db
        .query_row("SELECT cursor, end_rowid FROM screen_time_backfill WHERE id = 1", [], |row| Ok((row.get(0)?, row.get(1)?)))
        .optional()?;
    Ok(state.is_none_or(|(cursor, end)| cursor >= end))
}

fn read_state(db: &Connection) -> Result<(i64, i64), BridgeError> {
    Ok(db.query_row("SELECT cursor, end_rowid FROM screen_time_backfill WHERE id = 1", [], |row| Ok((row.get(0)?, row.get(1)?)))?)
}

/// Reads up to `limit` entries after `cursor` (never past `end`) and returns
/// their distinct `(session, minute)` pairs with the cursor to resume from.
pub fn read_batch(reader: &Connection, cursor: i64, end: i64, limit: i64) -> Result<(BTreeSet<(String, i64)>, i64), BridgeError> {
    let mut statement = reader.prepare_cached(
        "SELECT rowid, session_id, CAST(strftime('%s', created_at) AS INTEGER) / 60
         FROM session_entries
         WHERE rowid > ?1 AND rowid <= ?2
           AND kind <> 'user.message' AND strftime('%s', created_at) IS NOT NULL
         ORDER BY rowid LIMIT ?3",
    )?;
    let mut pairs = BTreeSet::new();
    let mut last = cursor;
    let mut read = 0_i64;
    for row in statement.query_map(params![cursor, end, limit], |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?, row.get::<_, i64>(2)?))
    })? {
        let (rowid, session, minute) = row?;
        pairs.insert((session, minute));
        last = rowid;
        read += 1;
    }
    Ok((pairs, if read < limit { end } else { last }))
}

/// Stores one batch and moves the cursor in one transaction, only if the
/// cursor is still where the batch started, so a second runner cannot rewind it.
pub fn store_batch(db: &mut Connection, from: i64, pairs: &BTreeSet<(String, i64)>, next: i64) -> Result<(), BridgeError> {
    let transaction = db.transaction()?;
    if read_state(&transaction)?.0 != from {
        return Ok(());
    }
    {
        let mut insert = transaction.prepare_cached("INSERT OR IGNORE INTO agent_activity_minutes(session_id, minute) VALUES (?1, ?2)")?;
        for (session, minute) in pairs {
            insert.execute(params![session, minute])?;
        }
    }
    transaction.execute("UPDATE screen_time_backfill SET cursor = ?1 WHERE id = 1", [next])?;
    transaction.commit()?;
    Ok(())
}

/// Runs the backfill to completion: reads through `reader` with no lock held,
/// and takes `writer` only to read the cursor and to store each batch.
pub fn run_backfill(reader: &Connection, writer: &Mutex<Connection>, limit: i64, pause: Duration) -> Result<(), BridgeError> {
    let lock = || writer.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    loop {
        let (cursor, end) = read_state(&lock())?;
        if cursor >= end {
            return Ok(());
        }
        let (pairs, next) = read_batch(reader, cursor, end, limit)?;
        store_batch(&mut lock(), cursor, &pairs, next)?;
        if next >= end {
            return Ok(());
        }
        std::thread::sleep(pause);
    }
}

fn running() -> &'static Mutex<HashSet<PathBuf>> {
    static RUNNING: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();
    RUNNING.get_or_init(|| Mutex::new(HashSet::new()))
}

/// Starts the backfill on its own thread unless one is already running for
/// this database. Returns at once.
pub fn ensure_backfill(core: &Arc<BridgeCore>) {
    let path = core.database_path.clone();
    if !running().lock().unwrap_or_else(std::sync::PoisonError::into_inner).insert(path.clone()) {
        return;
    }
    let core = core.clone();
    std::thread::spawn(move || {
        let outcome = Connection::open_with_flags(
            &path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(BridgeError::from)
        .and_then(|reader| run_backfill(&reader, &core.db, BACKFILL_BATCH, BACKFILL_PAUSE));
        if let Err(error) = outcome {
            eprintln!("[screen-time] agent history backfill stopped: {error}");
        }
        running().lock().unwrap_or_else(std::sync::PoisonError::into_inner).remove(&path);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    const BASE: &str = "2026-11-20T10:00:00Z";

    fn schema(db: &Connection) {
        db.execute_batch(
            "CREATE TABLE projects(id TEXT PRIMARY KEY, name TEXT NOT NULL);
             CREATE TABLE workspaces(id TEXT PRIMARY KEY, project_id TEXT);
             CREATE TABLE sessions(id TEXT PRIMARY KEY, kind TEXT, workspace_id TEXT, harness TEXT, parent_session_id TEXT);
             CREATE TABLE session_entries(id TEXT PRIMARY KEY, session_id TEXT NOT NULL, kind TEXT NOT NULL, created_at TEXT NOT NULL);",
        )
        .unwrap();
    }

    fn install(db: &mut Connection) {
        let transaction = db.transaction().unwrap();
        install_store(&transaction).unwrap();
        install_chat_store(&transaction).unwrap();
        crate::session_prs::install_store(&transaction).unwrap();
        transaction.commit().unwrap();
    }

    fn fresh() -> Connection {
        let mut db = Connection::open_in_memory().unwrap();
        schema(&db);
        install(&mut db);
        db
    }

    fn session(db: &Connection, id: &str, kind: Option<&str>) {
        db.execute("INSERT INTO sessions(id, kind) VALUES (?1, ?2)", params![id, kind]).unwrap();
    }

    /// A project with one workspace of the same id.
    fn project(db: &Connection, id: &str, name: &str) {
        db.execute("INSERT INTO projects(id, name) VALUES (?1, ?2)", params![id, name]).unwrap();
        db.execute("INSERT INTO workspaces(id, project_id) VALUES (?1, ?1)", params![id]).unwrap();
    }

    fn chat(db: &Connection, id: &str, kind: &str, harness: &str, workspace: Option<&str>, parent: Option<&str>) {
        db.execute(
            "INSERT INTO sessions(id, kind, workspace_id, harness, parent_session_id) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id, kind, workspace, harness, parent],
        )
        .unwrap();
    }

    /// One `session_pull_requests` row on `github.com/<repo>`.
    #[allow(clippy::too_many_arguments)]
    fn pull_request(db: &Connection, session: &str, repo: &str, number: i64, attribution: &str, state: Option<&str>, created_at: &str, fetched_at: Option<&str>) {
        let (owner, name) = repo.split_once('/').unwrap();
        let snapshot = state.map(|state| format!(r#"{{"title":"PR {number} {state}","url":"https://github.com/{repo}/pull/{number}","state":"{state}"}}"#));
        db.execute(
            "INSERT INTO session_pull_requests(session_id, workspace_id, repo_host, repo_owner, repo_name, number, url, title, head_branch, head_sha, attribution, snapshot, fetched_at, created_at)
             VALUES (?1, '', 'github.com', ?2, ?3, ?4, ?5, ?6, 'b', 's', ?7, ?8, ?9, ?10)",
            params![session, owner, name, number, format!("https://github.com/{repo}/pull/{number}"), format!("PR {number}"), attribution, snapshot, fetched_at, created_at],
        )
        .unwrap();
    }

    /// `offset` minutes after `BASE`, alternating the two timestamp shapes
    /// the forest writes (`Z` with millis, `+00:00` with micros).
    fn at(offset: i64, seconds: u32) -> String {
        let base = DateTime::parse_from_rfc3339(BASE).unwrap().with_timezone(&Utc);
        let instant = base + chrono::Duration::minutes(offset) + chrono::Duration::seconds(i64::from(seconds));
        if offset % 2 == 0 {
            instant.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
        } else {
            instant.format("%Y-%m-%dT%H:%M:%S%.6f+00:00").to_string()
        }
    }

    fn entry(db: &Connection, session: &str, kind: &str, created_at: &str) {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let id = format!("e{}", NEXT.fetch_add(1, Ordering::Relaxed));
        db.execute(
            "INSERT INTO session_entries(id, session_id, kind, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![id, session, kind, created_at],
        )
        .unwrap();
    }

    fn base_minute() -> i64 {
        DateTime::parse_from_rfc3339(BASE).unwrap().timestamp() / 60
    }

    fn ask(db: &Connection, since: &str, until: &str, zone: Option<&str>) -> wire::UsageScreenTimeResult {
        screen_time(
            db,
            &wire::ScreenTimeParams { since_day: since.into(), until_day: until.into(), time_zone: zone.map(str::to_owned) },
        )
        .unwrap()
    }

    fn agent_minutes(db: &Connection) -> i64 {
        ask(db, "2026-11-20", "2026-11-20", None).all_time.agent_seconds / 60
    }

    #[test]
    fn the_trigger_notes_each_entry_minute_once_and_skips_the_persons_messages() {
        let db = fresh();
        session(&db, "chat", Some("direct"));
        entry(&db, "chat", "user.message", &at(0, 5));
        entry(&db, "chat", "assistant.message", &at(10, 1));
        entry(&db, "chat", "command.output_delta", &at(10, 30));
        entry(&db, "chat", "tool.completed", &at(11, 59));
        let rows: Vec<i64> = db
            .prepare("SELECT minute FROM agent_activity_minutes ORDER BY minute")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(rows, vec![base_minute() + 10, base_minute() + 11]);
    }

    #[test]
    fn two_silent_minutes_inside_a_chat_count_and_three_do_not() {
        let db = fresh();
        session(&db, "chat", None);
        entry(&db, "chat", "turn.started", &at(0, 0));
        // 1 and 2 are silent: filled.
        entry(&db, "chat", "tool.completed", &at(3, 0));
        // 4, 5 and 6 are silent: idle.
        entry(&db, "chat", "assistant.message", &at(7, 0));
        assert_eq!(agent_minutes(&db), 4 + 1);
    }

    #[test]
    fn chats_working_through_the_same_minute_add_up() {
        let db = fresh();
        session(&db, "a", Some("direct"));
        session(&db, "worker", Some("orchestrator"));
        session(&db, "sub", Some("harness_subagent"));
        for chat in ["a", "worker", "sub"] {
            entry(&db, chat, "assistant.message", &at(0, 0));
            entry(&db, chat, "assistant.message", &at(1, 0));
        }
        assert_eq!(agent_minutes(&db), 6);
    }

    #[test]
    fn background_and_imported_sessions_never_count_as_agent_time() {
        let db = fresh();
        for (id, kind) in [("b", "briefing"), ("s", "suggestion"), ("x", "extraction"), ("c", "connector"), ("q", "chat_search"), ("i", "imported")] {
            session(&db, id, Some(kind));
            entry(&db, id, "assistant.message", &at(0, 0));
        }
        session(&db, "chat", Some("direct"));
        entry(&db, "chat", "assistant.message", &at(0, 0));
        assert_eq!(agent_minutes(&db), 1);
    }

    #[test]
    fn minutes_land_on_the_local_day_their_start_falls_in() {
        let db = fresh();
        session(&db, "chat", Some("direct"));
        // 14:59Z is 23:59 in Tokyo on the 20th; 15:00Z is 00:00 on the 21st.
        entry(&db, "chat", "assistant.message", "2026-11-20T14:59:30Z");
        entry(&db, "chat", "assistant.message", "2026-11-20T15:00:10Z");
        let now = DateTime::parse_from_rfc3339("2026-11-20T15:00:00Z").unwrap().timestamp() / 60;
        record_presence(&db, &[now - 1, now], &[], now).unwrap();
        let result = ask(&db, "2026-11-19", "2026-11-22", Some("Asia/Tokyo"));
        let days: Vec<(&str, i64, i64)> =
            result.days.iter().map(|day| (day.day.as_str(), day.human_seconds, day.agent_seconds)).collect();
        assert_eq!(
            days,
            vec![("2026-11-19", 0, 0), ("2026-11-20", 60, 60), ("2026-11-21", 60, 60), ("2026-11-22", 0, 0)]
        );
        assert_eq!(result.time_zone, "Asia/Tokyo");
        assert_eq!(result.all_time.human_since_day.as_deref(), Some("2026-11-20"));
        assert_eq!(result.all_time.agent_since_day.as_deref(), Some("2026-11-20"));
        // UTC puts both on the 20th.
        let utc = ask(&db, "2026-11-20", "2026-11-21", None);
        assert_eq!(utc.days[0].human_seconds, 120);
        assert_eq!(utc.days[1].human_seconds, 0);
    }

    #[test]
    fn all_time_covers_history_outside_the_range_and_is_empty_when_nothing_is_recorded() {
        let db = fresh();
        let empty = ask(&db, "2026-11-20", "2026-11-26", None);
        assert_eq!(empty.days.len(), 7);
        assert!(empty.days.iter().all(|day| day.human_seconds == 0 && day.agent_seconds == 0));
        assert_eq!(empty.all_time, wire::ScreenTimeTotals { human_seconds: 0, agent_seconds: 0, human_since_day: None, agent_since_day: None });
        assert_eq!(empty.agent_history, wire::ScreenTimeAgentHistory::Complete);

        session(&db, "chat", Some("direct"));
        entry(&db, "chat", "assistant.message", "2026-10-01T09:00:00Z");
        let later = ask(&db, "2026-11-20", "2026-11-26", None);
        assert_eq!(later.all_time.agent_seconds, 60);
        assert_eq!(later.all_time.agent_since_day.as_deref(), Some("2026-10-01"));
        assert!(later.days.iter().all(|day| day.agent_seconds == 0));
    }

    #[test]
    fn presence_drops_future_and_stale_minutes_and_stores_each_minute_once() {
        let db = fresh();
        let now = base_minute();
        assert_eq!(record_presence(&db, &[now - 2, now - 1, now, now + 1, now - 7 * 1_440 - 1], &[], now).unwrap(), 3);
        assert_eq!(record_presence(&db, &[now - 1, now], &[], now).unwrap(), 0);
        assert_eq!(record_presence(&db, &[now - 7 * 1_440], &[], now).unwrap(), 1);
        let too_many: Vec<i64> = (0..1_441).map(|offset| now - offset).collect();
        assert!(matches!(record_presence(&db, &too_many, &[], now), Err(BridgeError::Invalid(_))));
        let count: i64 = db.query_row("SELECT COUNT(*) FROM human_activity_minutes", [], |row| row.get(0)).unwrap();
        assert_eq!(count, 4);
    }

    #[test]
    fn bad_ranges_and_zones_are_refused() {
        let db = fresh();
        let refuse = |since: &str, until: &str, zone: Option<&str>| {
            screen_time(&db, &wire::ScreenTimeParams { since_day: since.into(), until_day: until.into(), time_zone: zone.map(str::to_owned) })
                .is_err_and(|error| matches!(error, BridgeError::Invalid(_)))
        };
        assert!(refuse("2026-11-21", "2026-11-20", None));
        assert!(refuse("2026-08-01", "2026-11-20", None));
        assert!(refuse("2026-11-20", "2026-11-20", Some("Mars/Olympus")));
        assert!(refuse("20-11-2026", "2026-11-20", None));
        assert!(!refuse("2026-08-21", "2026-11-20", Some("  ")));
    }

    /// Entries from before migration 88 have no trigger rows; the backfill
    /// must write exactly what the trigger would have, and resume after a stop.
    #[test]
    fn the_backfill_counts_old_entries_exactly_like_the_trigger_and_resumes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bridge.db");
        let mut old = Connection::open(&path).unwrap();
        schema(&old);
        let history = |db: &Connection| {
            session(db, "chat", Some("direct"));
            session(db, "brief", Some("briefing"));
            for offset in [0, 1, 1, 3, 9, 30] {
                entry(db, "chat", "assistant.message", &at(offset, 7));
            }
            entry(db, "chat", "user.message", &at(50, 0));
            entry(db, "brief", "assistant.message", &at(2, 0));
            entry(db, "chat", "assistant.message", "not a time");
        };
        history(&old);
        install(&mut old);
        assert_eq!(ask(&old, "2026-11-20", "2026-11-20", None).agent_history, wire::ScreenTimeAgentHistory::Counting);
        assert_eq!(agent_minutes(&old), 0);

        // The same history written after the trigger exists.
        let live = fresh();
        history(&live);
        let pairs = |db: &Connection| -> Vec<(String, i64)> {
            db.prepare("SELECT session_id, minute FROM agent_activity_minutes ORDER BY 1, 2")
                .unwrap()
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                .unwrap()
                .map(Result::unwrap)
                .collect()
        };

        let reader = Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let (cursor, end) = read_state(&old).unwrap();
        let (first, next) = read_batch(&reader, cursor, end, 3).unwrap();
        assert!(next < end, "a full batch stops short of the end");
        store_batch(&mut old, cursor, &first, next).unwrap();
        // A stale runner holding the old cursor cannot rewind or double write.
        store_batch(&mut old, cursor, &first, next).unwrap();
        assert_eq!(read_state(&old).unwrap().0, next);
        assert_eq!(ask(&old, "2026-11-20", "2026-11-20", None).agent_history, wire::ScreenTimeAgentHistory::Counting);

        let writer = Mutex::new(old);
        run_backfill(&reader, &writer, 3, Duration::ZERO).unwrap();
        let old = writer.into_inner().unwrap();
        assert_eq!(pairs(&old), pairs(&live));
        assert_eq!(agent_minutes(&old), agent_minutes(&live));
        assert_eq!(ask(&old, "2026-11-20", "2026-11-20", None).agent_history, wire::ScreenTimeAgentHistory::Complete);

        // New entries after the migration come from the trigger, not the backfill.
        entry(&old, "chat", "assistant.message", &at(31, 0));
        assert_eq!(agent_minutes(&old), agent_minutes(&live) + 1);
    }
    fn place_chat(session_id: &str, minutes: &[i64]) -> wire::PresenceChat {
        wire::PresenceChat { session_id: session_id.into(), minutes: minutes.to_vec() }
    }

    fn day_agent_total(result: &wire::UsageScreenTimeResult) -> i64 {
        result.days.iter().map(|day| day.agent_seconds).sum()
    }

    #[test]
    fn agent_time_splits_by_project_and_harness_and_adds_up_to_the_range() {
        let db = fresh();
        project(&db, "p-harness", "harness");
        project(&db, "p-kairo", "kairo");
        db.execute("INSERT INTO workspaces(id, project_id) VALUES ('loose', NULL)", []).unwrap();
        chat(&db, "a", "direct", "claude", Some("p-harness"), None);
        chat(&db, "worker", "orchestrator", "codex", Some("p-harness"), Some("a"));
        chat(&db, "k", "direct", "codex", Some("p-kairo"), None);
        chat(&db, "loose", "direct", "opencode", Some("loose"), None);
        chat(&db, "nowhere", "direct", "claude", None, None);
        chat(&db, "brief", "briefing", "claude", Some("p-kairo"), None);
        // a: 0..=3 with 1 and 2 filled (4 minutes); worker: 2 minutes.
        entry(&db, "a", "assistant.message", &at(0, 0));
        entry(&db, "a", "tool.completed", &at(3, 0));
        entry(&db, "worker", "assistant.message", &at(5, 0));
        entry(&db, "worker", "assistant.message", &at(6, 0));
        // k: two minutes three silent minutes apart, no fill.
        entry(&db, "k", "assistant.message", &at(0, 0));
        entry(&db, "k", "assistant.message", &at(4, 0));
        entry(&db, "loose", "assistant.message", &at(0, 0));
        entry(&db, "nowhere", "assistant.message", &at(0, 0));
        entry(&db, "brief", "assistant.message", &at(0, 0));
        // Outside the range: counts all-time, not per project.
        entry(&db, "k", "assistant.message", "2026-10-01T09:00:00Z");

        let result = ask(&db, "2026-11-20", "2026-11-20", None);
        let projects: Vec<(Option<&str>, &str, i64, i64)> = result
            .projects
            .iter()
            .map(|project| (project.project_id.as_deref(), project.name.as_str(), project.agent_seconds / 60, project.chats))
            .collect();
        assert_eq!(
            projects,
            vec![(Some("p-harness"), "harness", 6, 1), (Some("p-kairo"), "kairo", 2, 1), (None, NO_PROJECT, 2, 2)]
        );
        assert_eq!(result.projects.iter().map(|project| project.agent_seconds).sum::<i64>(), day_agent_total(&result));
        assert_eq!(result.all_time.agent_seconds, day_agent_total(&result) + 60);

        let harnesses: Vec<(&str, i64, i64)> =
            result.harnesses.iter().map(|harness| (harness.harness.as_str(), harness.agent_seconds / 60, harness.chats)).collect();
        assert_eq!(harnesses, vec![("claude", 5, 2), ("codex", 4, 1), ("opencode", 1, 1)]);
    }

    #[test]
    fn your_time_per_project_counts_only_minutes_with_a_chat_open() {
        let db = fresh();
        project(&db, "p", "harness");
        chat(&db, "a", "direct", "claude", Some("p"), None);
        chat(&db, "b", "direct", "claude", None, None);
        let now = base_minute() + 30;
        let minutes = [now - 4, now - 3, now - 2, now - 1];
        let chats = [place_chat("a", &[now - 4, now - 3, now - 9]), place_chat("b", &[now - 2])];
        assert_eq!(record_presence(&db, &minutes, &chats, now).unwrap(), 4);
        // A second window cannot move a minute to another chat.
        record_presence(&db, &[now - 4], &[place_chat("b", &[now - 4])], now).unwrap();
        let stored: i64 = db.query_row("SELECT COUNT(*) FROM human_activity_chats", [], |row| row.get(0)).unwrap();
        assert_eq!(stored, 3, "now - 9 was not among the accepted minutes");

        let result = ask(&db, "2026-11-20", "2026-11-20", None);
        let human: Vec<(&str, i64)> = result.projects.iter().map(|project| (project.name.as_str(), project.human_seconds / 60)).collect();
        assert_eq!(human, vec![("harness", 2), (NO_PROJECT, 1)]);
        assert_eq!(result.days[0].human_seconds / 60, 4, "a minute with no chat open still counts as yours");
        assert!(result.projects.iter().all(|project| project.chats == 0), "your time alone never makes a chat count");
    }

    #[test]
    fn presence_chats_are_validated_before_anything_is_stored() {
        let db = fresh();
        let now = base_minute();
        let refuse = |chats: &[wire::PresenceChat]| matches!(record_presence(&db, &[now], chats, now), Err(BridgeError::Invalid(_)));
        assert!(refuse(&[place_chat(" ", &[now])]));
        assert!(refuse(&[place_chat(&"x".repeat(129), &[now])]));
        let many: Vec<i64> = (0..1_441).map(|offset| now - offset).collect();
        assert!(refuse(&[place_chat("a", &many[..700]), place_chat("b", &many[700..])]));
        let stored: i64 = db.query_row("SELECT COUNT(*) FROM human_activity_minutes", [], |row| row.get(0)).unwrap();
        assert_eq!(stored, 0);
        assert!(!refuse(&[place_chat(&"x".repeat(128), &[now])]));
    }

    #[test]
    fn pull_requests_count_once_skip_manual_links_and_take_the_latest_state() {
        let db = fresh();
        project(&db, "p", "harness");
        project(&db, "q", "kairo");
        chat(&db, "a", "direct", "claude", Some("p"), None);
        chat(&db, "b", "direct", "codex", Some("q"), None);
        // #1 is linked from two chats: opened by a (earliest), latest fetch says merged.
        pull_request(&db, "a", "me/bridge", 1, "toolCompletion", Some("open"), "2026-11-20T09:00:00Z", Some("2026-11-20T09:05:00Z"));
        pull_request(&db, "b", "me/bridge", 1, "toolCompletion", Some("merged"), "2026-11-20T11:00:00.123456+00:00", Some("2026-11-20T12:00:00Z"));
        pull_request(&db, "a", "me/bridge", 2, "toolCompletion", Some("closed"), "2026-11-20T10:00:00Z", None);
        pull_request(&db, "b", "me/kairo", 7, "toolCompletion", None, "2026-11-20T13:00:00Z", None);
        pull_request(&db, "b", "me/kairo", 8, "toolCompletion", Some("open"), "2026-11-20T14:00:00Z", Some("2026-11-20T14:00:00Z"));
        // Attached by hand, outside the range, or with a broken snapshot.
        pull_request(&db, "a", "me/bridge", 3, "manual", Some("merged"), "2026-11-20T10:00:00Z", None);
        pull_request(&db, "a", "me/bridge", 4, "toolCompletion", Some("merged"), "2026-11-19T23:59:00Z", None);
        db.execute("UPDATE session_pull_requests SET snapshot = 'not json' WHERE number = 8", []).unwrap();

        let result = ask(&db, "2026-11-20", "2026-11-20", None);
        let summary = &result.pull_requests;
        assert_eq!((summary.opened, summary.merged, summary.open, summary.closed), (4, 1, 0, 1));
        let repos: Vec<(&str, i64, i64)> = summary.repos.iter().map(|repo| (repo.repo.as_str(), repo.opened, repo.merged)).collect();
        assert_eq!(repos, vec![("me/bridge", 2, 1), ("me/kairo", 2, 0)]);
        let recent: Vec<(i64, wire::ScreenTimePullRequestState, &str)> =
            summary.recent.iter().map(|pr| (pr.number, pr.state, pr.title.as_str())).collect();
        assert_eq!(
            recent,
            vec![
                (8, wire::ScreenTimePullRequestState::Unknown, "PR 8"),
                (7, wire::ScreenTimePullRequestState::Unknown, "PR 7"),
                (2, wire::ScreenTimePullRequestState::Closed, "PR 2 closed"),
                (1, wire::ScreenTimePullRequestState::Merged, "PR 1 merged"),
            ]
        );
        assert_eq!(summary.recent[3].opened_day, "2026-11-20");
        assert_eq!(summary.recent[3].url, "https://github.com/me/bridge/pull/1");
        let per_project: Vec<(&str, i64)> = result.projects.iter().map(|project| (project.name.as_str(), project.pull_requests)).collect();
        assert_eq!(per_project, vec![("harness", 2), ("kairo", 2)], "#1 belongs to the chat that opened it");

        // In Tokyo, #4 (23:59Z on the 19th) lands on the 20th.
        let tokyo = ask(&db, "2026-11-20", "2026-11-20", Some("Asia/Tokyo"));
        assert!(tokyo.pull_requests.recent.iter().any(|pr| pr.number == 4));
    }

    #[test]
    fn recent_pull_requests_stop_at_six_newest_first() {
        let db = fresh();
        chat(&db, "a", "direct", "claude", None, None);
        for number in 1..=9 {
            pull_request(&db, "a", "me/bridge", number, "toolCompletion", Some("open"), &format!("2026-11-20T0{number}:00:00Z"), None);
        }
        let result = ask(&db, "2026-11-20", "2026-11-20", None);
        assert_eq!(result.pull_requests.opened, 9);
        let numbers: Vec<i64> = result.pull_requests.recent.iter().map(|pr| pr.number).collect();
        assert_eq!(numbers, vec![9, 8, 7, 6, 5, 4]);
        assert_eq!(result.projects.len(), 1);
        assert_eq!((result.projects[0].name.as_str(), result.projects[0].pull_requests), (NO_PROJECT, 9));
    }

    #[test]
    fn hours_are_local_and_always_twenty_four() {
        let db = fresh();
        chat(&db, "a", "direct", "claude", None, None);
        // 10:00Z is 19:00 in Tokyo.
        entry(&db, "a", "assistant.message", &at(0, 0));
        record_presence(&db, &[base_minute(), base_minute() + 61], &[], base_minute() + 61).unwrap();
        let tokyo = ask(&db, "2026-11-20", "2026-11-20", Some("Asia/Tokyo"));
        assert_eq!(tokyo.hours.len(), 24);
        let busy: Vec<(u8, i64, i64)> = tokyo
            .hours
            .iter()
            .filter(|hour| hour.human_seconds + hour.agent_seconds > 0)
            .map(|hour| (hour.hour, hour.human_seconds, hour.agent_seconds))
            .collect();
        assert_eq!(busy, vec![(19, 60, 60), (20, 60, 0)]);
        let empty = ask(&db, "2026-11-01", "2026-11-02", None);
        assert_eq!(empty.hours.len(), 24);
        assert!(empty.projects.is_empty() && empty.harnesses.is_empty() && empty.pull_requests.recent.is_empty());
    }
}
