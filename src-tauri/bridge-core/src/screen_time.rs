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
//! Entries written before this existed are counted by a backfill. A full scan
//! of `session_entries` reads every payload page (the timestamp is stored
//! after the payload) and took 33 s on a 7.9 GB history, so it never runs on
//! boot and never reads under the core mutex: it starts the first time the
//! Time tab asks, reads through its own read-only connection in rowid
//! batches, and only takes the core connection for each short write.

use crate::{BridgeCore, BridgeError};
use bridge_protocol::messages as wire;
use chrono::{DateTime, NaiveDate, Utc};
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

/// Stores the minutes the person was active. Minutes after `now_minute` or
/// older than a week are dropped; returns how many were new.
pub fn record_presence(db: &Connection, minutes: &[i64], now_minute: i64) -> Result<i64, BridgeError> {
    if minutes.len() > MAX_PRESENCE_MINUTES {
        return Err(BridgeError::Invalid(format!(
            "record_presence takes at most {MAX_PRESENCE_MINUTES} minutes per call, got {}",
            minutes.len()
        )));
    }
    let oldest = now_minute - PRESENCE_MAX_AGE_MINUTES;
    let mut statement = db.prepare_cached("INSERT OR IGNORE INTO human_activity_minutes(minute) VALUES (?1)")?;
    let mut recorded = 0;
    for minute in minutes.iter().copied().filter(|minute| (oldest..=now_minute).contains(minute)) {
        recorded += i64::try_from(statement.execute([minute])?).unwrap_or(0);
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

    fn local_day(&self, minute: i64) -> Option<NaiveDate> {
        DateTime::<Utc>::from_timestamp(minute.checked_mul(60)?, 0).map(|at| at.with_timezone(&self.zone).date_naive())
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

/// The Time tab's figures: one entry per day in the range and the all-time
/// totals.
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

    let (human_minutes, first_human): (i64, Option<i64>) =
        db.query_row("SELECT COUNT(*), MIN(minute) FROM human_activity_minutes", [], |row| Ok((row.get(0)?, row.get(1)?)))?;
    let (low, high) = range.coarse_minutes();
    let mut statement = db.prepare("SELECT minute FROM human_activity_minutes WHERE minute >= ?1 AND minute < ?2")?;
    for minute in statement.query_map(params![low, high], |row| row.get::<_, i64>(0))? {
        if let Some(cell) = range.local_day(minute?).and_then(|day| days.get_mut(&day)) {
            cell.0 += 60;
        }
    }

    let mut agent_minutes = 0_i64;
    let mut first_agent: Option<i64> = None;
    for_each_agent_minute(db, |minute| {
        agent_minutes += 1;
        first_agent = Some(first_agent.map_or(minute, |first| first.min(minute)));
        if (low..high).contains(&minute) {
            if let Some(cell) = range.local_day(minute).and_then(|day| days.get_mut(&day)) {
                cell.1 += 60;
            }
        }
    })?;

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
    })
}

/// Calls `visit` once per counted agent minute, gaps of up to
/// `GAP_FILL_MINUTES` inside one chat filled in.
fn for_each_agent_minute(db: &Connection, mut visit: impl FnMut(i64)) -> Result<(), BridgeError> {
    let mut statement = db.prepare(&format!(
        "SELECT a.session_id, a.minute FROM agent_activity_minutes a
         JOIN sessions s ON s.id = a.session_id
         WHERE {COUNTED_KINDS_SQL}
         ORDER BY a.session_id, a.minute"
    ))?;
    let mut rows = statement.query([])?;
    let mut previous: Option<(String, i64)> = None;
    while let Some(row) = rows.next()? {
        let session: String = row.get(0)?;
        let minute: i64 = row.get(1)?;
        if let Some((last_session, last_minute)) = &previous {
            if *last_session == session && minute - last_minute - 1 <= GAP_FILL_MINUTES {
                for silent in (last_minute + 1)..minute {
                    visit(silent);
                }
            }
        }
        visit(minute);
        previous = Some((session, minute));
    }
    Ok(())
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
            "CREATE TABLE sessions(id TEXT PRIMARY KEY, kind TEXT);
             CREATE TABLE session_entries(id TEXT PRIMARY KEY, session_id TEXT NOT NULL, kind TEXT NOT NULL, created_at TEXT NOT NULL);",
        )
        .unwrap();
    }

    fn install(db: &mut Connection) {
        let transaction = db.transaction().unwrap();
        install_store(&transaction).unwrap();
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
        record_presence(&db, &[now - 1, now], now).unwrap();
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
        assert_eq!(record_presence(&db, &[now - 2, now - 1, now, now + 1, now - 7 * 1_440 - 1], now).unwrap(), 3);
        assert_eq!(record_presence(&db, &[now - 1, now], now).unwrap(), 0);
        assert_eq!(record_presence(&db, &[now - 7 * 1_440], now).unwrap(), 1);
        let too_many: Vec<i64> = (0..1_441).map(|offset| now - offset).collect();
        assert!(matches!(record_presence(&db, &too_many, now), Err(BridgeError::Invalid(_))));
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
}
