//! Carrying a chat past a provider usage limit, on the person's say-so.
//!
//! A limit stops a chat mid-task and, until now, it stayed stopped until
//! someone noticed: they switched model by hand or came back after the reset.
//! The setting here (`config/save_limit_recovery_settings`) lets them choose
//! one of two things instead:
//!
//! - **Resume**: wait for the reset the provider stated, then send the chat a
//!   continue turn on the same harness, model and effort.
//! - **Backup**: switch the chat to a backup model now and continue there. A
//!   backup that also runs out falls back to resuming at its own reset.
//!
//! The pieces are split so each can be tested on its own:
//!
//! - [`plan`] and [`guard`] are pure. They decide what to schedule and whether
//!   a due row may still act, from plain facts and an injected clock.
//! - [`observe`] is the hook `live_turn` calls from the error frame, with the
//!   database lock it already holds. It only writes a row.
//! - [`tick`] runs due rows through an [`Actions`] implementation. The live
//!   one ([`LiveActions`]) switches the model with `api::update_chat_model`
//!   and sends the turn through the same input path a person's message takes.
//!
//! The loop compares wall-clock instants on every tick rather than sleeping
//! until a deadline, so a Mac that slept through the reset acts on wake, and
//! a row survives a restart because it lives in SQLite. While any row is
//! pending, [`PowerAssertion`] holds `caffeinate -i` so the machine does not
//! idle-sleep through the reset; a closed lid still sleeps, which is the
//! person's call.

use crate::{provider_limit::LimitKind, BridgeError};
use bridge_protocol::messages::{
    self as wire, Effort, LimitRecoveryMode, LimitRecoverySettings, LimitRecoveryState,
    LimitRecoveryTarget, LimitRecoveryView,
};
use chrono::{DateTime, Duration, Utc};
use rusqlite::{params, Connection, OptionalExtension, Transaction};

const SETTINGS_KIND: &str = "limit_recovery_settings";
const SETTINGS_ID: &str = "global";

/// How long after the stated reset Bridge waits before resuming. Providers
/// round their banners to the minute and their clocks are not ours; resuming
/// at the exact instant lands a few seconds early about half the time and
/// burns an attempt on a limit that is still in force.
pub const RESET_GRACE: Duration = Duration::seconds(90);

/// Limits in a row, while recovering, before Bridge stops trying. Counted
/// only across rows Bridge itself acted on within [`STREAK_WINDOW`]: a limit
/// next week after a successful resume starts a fresh count.
pub const MAX_ATTEMPTS: u32 = 6;
pub const STREAK_WINDOW: Duration = Duration::hours(12);

/// With no stated reset, Bridge probes: 15 minutes, then doubling, capped.
/// A probe that hits the limit again costs one failed turn and no tokens.
pub const FIRST_PROBE: Duration = Duration::minutes(15);
pub const MAX_PROBE: Duration = Duration::hours(2);

/// A row Bridge could not act on because it was closed is still acted on at
/// the next start when it is at most this late. Later than that, the person
/// has very likely moved on, and a chat waking up a day later on its own is a
/// surprise rather than a help.
pub const CATCH_UP: Duration = Duration::hours(12);

/// What the chat is told when Bridge continues it. Plain, so it reads right
/// in the transcript, and explicit about not redoing work: the provider may
/// or may not have recorded the interrupted turn, and either way the model
/// should pick up from the last finished step.
pub const CONTINUE_PROMPT: &str = "Your previous turn stopped because the provider's usage limit was reached. The limit has cleared, so continue the task from where it stopped. Do not redo work that is already finished.";

/// The same, after Bridge switched the chat to the backup model.
pub const CONTINUE_ON_BACKUP_PROMPT: &str = "Your previous turn stopped because the provider's usage limit was reached, so this chat was switched to a backup model. Continue the task from where it stopped. Do not redo work that is already finished.";

// ---------------------------------------------------------------- settings

pub fn load_settings(db: &Connection) -> Result<LimitRecoverySettings, BridgeError> {
    let payload: Option<String> = db
        .query_row(
            "SELECT payload FROM configuration_entries WHERE kind=?1 AND id=?2",
            params![SETTINGS_KIND, SETTINGS_ID],
            |row| row.get(0),
        )
        .optional()?;
    // A record this build cannot read is "off", never a guess: acting on a
    // misread setting would send turns the person did not ask for.
    Ok(payload
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default())
}

/// Validate and store. `known_harness` answers whether a harness id is one
/// this Bridge can run, so a backup can never name something unlaunchable.
pub fn save_settings(
    db: &Connection,
    settings: &LimitRecoverySettings,
    known_harness: impl Fn(&str) -> bool,
) -> Result<LimitRecoverySettings, BridgeError> {
    let normalized = normalize_settings(settings, known_harness)?;
    db.execute(
        "INSERT INTO configuration_entries(kind,id,payload,created_at,updated_at)
         VALUES(?1,?2,?3,?4,?4) ON CONFLICT(kind,id) DO UPDATE SET payload=excluded.payload,updated_at=excluded.updated_at",
        params![
            SETTINGS_KIND,
            SETTINGS_ID,
            serde_json::to_string(&normalized).map_err(|error| BridgeError::Invalid(error.to_string()))?,
            Utc::now().to_rfc3339()
        ],
    )?;
    load_settings(db)
}

fn normalize_settings(
    settings: &LimitRecoverySettings,
    known_harness: impl Fn(&str) -> bool,
) -> Result<LimitRecoverySettings, BridgeError> {
    let backup = match &settings.backup {
        None => None,
        Some(target) => {
            let harness = target.harness.trim().to_ascii_lowercase();
            if harness.is_empty() {
                None
            } else {
                if !known_harness(&harness) {
                    return Err(BridgeError::Invalid(format!("Unknown backup harness `{}`", target.harness)));
                }
                Some(LimitRecoveryTarget {
                    harness,
                    model: target.model.as_deref().map(str::trim).filter(|model| !model.is_empty()).map(str::to_owned),
                    effort: target.effort,
                })
            }
        }
    };
    if settings.mode == LimitRecoveryMode::Backup && backup.is_none() {
        return Err(BridgeError::Invalid("Choose a backup model before turning on Switch to a backup model".into()));
    }
    Ok(LimitRecoverySettings { mode: settings.mode, backup })
}

// ---------------------------------------------------------------- store

pub fn install_store(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS limit_recoveries (
            session_id TEXT PRIMARY KEY,
            mode TEXT NOT NULL,
            state TEXT NOT NULL,
            signal TEXT NOT NULL,
            reset_at TEXT,
            due_at TEXT NOT NULL,
            from_harness TEXT NOT NULL,
            from_model TEXT,
            from_effort TEXT,
            target_harness TEXT NOT NULL,
            target_model TEXT,
            target_effort TEXT,
            switches_model INTEGER NOT NULL,
            attempts INTEGER NOT NULL,
            outcome TEXT,
            observed_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS limit_recoveries_due ON limit_recoveries(state, due_at);",
    )?;
    Ok(())
}

/// What a chat runs on. Effort is the stored string (`low`..`xhigh`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    pub harness: String,
    pub model: Option<String>,
    pub effort: Option<String>,
}

impl Selection {
    /// Whether two selections run the same model. Effort is not part of it:
    /// a chat on the backup model at another effort is on the backup.
    fn same_model(&self, other: &Selection) -> bool {
        self.harness == other.harness && self.model == other.model
    }

    fn from_target(target: &LimitRecoveryTarget) -> Selection {
        Selection {
            harness: target.harness.clone(),
            model: target.model.clone(),
            effort: target.effort.map(effort_name).map(str::to_owned),
        }
    }

    fn to_target(&self) -> LimitRecoveryTarget {
        LimitRecoveryTarget {
            harness: self.harness.clone(),
            model: self.model.clone(),
            effort: self.effort.as_deref().and_then(effort_from_name),
        }
    }
}

fn effort_name(effort: Effort) -> &'static str {
    match effort {
        Effort::Low => "low",
        Effort::Medium => "medium",
        Effort::High => "high",
        Effort::Xhigh => "xhigh",
    }
}

fn effort_from_name(name: &str) -> Option<Effort> {
    Some(match name {
        "low" => Effort::Low,
        "medium" => Effort::Medium,
        "high" => Effort::High,
        "xhigh" => Effort::Xhigh,
        _ => return None,
    })
}

/// One chat's recovery row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recovery {
    pub session_id: String,
    pub mode: LimitRecoveryMode,
    pub state: LimitRecoveryState,
    pub signal: String,
    pub reset_at: Option<DateTime<Utc>>,
    pub due_at: DateTime<Utc>,
    pub from: Selection,
    pub target: Selection,
    pub switches_model: bool,
    pub attempts: u32,
    pub outcome: Option<String>,
    pub observed_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

fn mode_name(mode: LimitRecoveryMode) -> &'static str {
    match mode {
        LimitRecoveryMode::Off => "off",
        LimitRecoveryMode::Resume => "resume",
        LimitRecoveryMode::Backup => "backup",
    }
}

fn mode_from_name(name: &str) -> LimitRecoveryMode {
    match name {
        "resume" => LimitRecoveryMode::Resume,
        "backup" => LimitRecoveryMode::Backup,
        _ => LimitRecoveryMode::Off,
    }
}

fn state_name(state: LimitRecoveryState) -> &'static str {
    match state {
        LimitRecoveryState::Pending => "pending",
        LimitRecoveryState::Acting => "acting",
        LimitRecoveryState::Done => "done",
        LimitRecoveryState::Cancelled => "cancelled",
        LimitRecoveryState::GaveUp => "gave_up",
        LimitRecoveryState::Expired => "expired",
        LimitRecoveryState::Failed => "failed",
    }
}

fn state_from_name(name: &str) -> LimitRecoveryState {
    match name {
        "pending" => LimitRecoveryState::Pending,
        "acting" => LimitRecoveryState::Acting,
        "done" => LimitRecoveryState::Done,
        "cancelled" => LimitRecoveryState::Cancelled,
        "gave_up" => LimitRecoveryState::GaveUp,
        "expired" => LimitRecoveryState::Expired,
        _ => LimitRecoveryState::Failed,
    }
}

fn parse_time(text: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(text)
        .map(|time| time.with_timezone(&Utc))
        // A row this build cannot read is due now rather than never, so it
        // reaches the guard and is settled visibly instead of sitting forever.
        .unwrap_or(DateTime::<Utc>::MIN_UTC)
}

const COLUMNS: &str = "session_id,mode,state,signal,reset_at,due_at,from_harness,from_model,from_effort,target_harness,target_model,target_effort,switches_model,attempts,outcome,observed_at,updated_at";

fn read_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Recovery> {
    Ok(Recovery {
        session_id: row.get(0)?,
        mode: mode_from_name(&row.get::<_, String>(1)?),
        state: state_from_name(&row.get::<_, String>(2)?),
        signal: row.get(3)?,
        reset_at: row.get::<_, Option<String>>(4)?.map(|text| parse_time(&text)),
        due_at: parse_time(&row.get::<_, String>(5)?),
        from: Selection { harness: row.get(6)?, model: row.get(7)?, effort: row.get(8)? },
        target: Selection { harness: row.get(9)?, model: row.get(10)?, effort: row.get(11)? },
        switches_model: row.get(12)?,
        attempts: row.get(13)?,
        outcome: row.get(14)?,
        observed_at: parse_time(&row.get::<_, String>(15)?),
        updated_at: parse_time(&row.get::<_, String>(16)?),
    })
}

pub fn get(db: &Connection, session_id: &str) -> Result<Option<Recovery>, BridgeError> {
    Ok(db
        .query_row(
            &format!("SELECT {COLUMNS} FROM limit_recoveries WHERE session_id=?1"),
            [session_id],
            read_row,
        )
        .optional()?)
}

fn upsert(db: &Connection, row: &Recovery) -> Result<(), BridgeError> {
    db.execute(
        &format!(
            "INSERT INTO limit_recoveries({COLUMNS}) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17)
             ON CONFLICT(session_id) DO UPDATE SET mode=excluded.mode,state=excluded.state,signal=excluded.signal,
               reset_at=excluded.reset_at,due_at=excluded.due_at,from_harness=excluded.from_harness,
               from_model=excluded.from_model,from_effort=excluded.from_effort,target_harness=excluded.target_harness,
               target_model=excluded.target_model,target_effort=excluded.target_effort,
               switches_model=excluded.switches_model,attempts=excluded.attempts,outcome=excluded.outcome,
               observed_at=excluded.observed_at,updated_at=excluded.updated_at"
        ),
        params![
            row.session_id,
            mode_name(row.mode),
            state_name(row.state),
            row.signal,
            row.reset_at.map(|time| time.to_rfc3339()),
            row.due_at.to_rfc3339(),
            row.from.harness,
            row.from.model,
            row.from.effort,
            row.target.harness,
            row.target.model,
            row.target.effort,
            row.switches_model,
            row.attempts,
            row.outcome,
            row.observed_at.to_rfc3339(),
            row.updated_at.to_rfc3339(),
        ],
    )?;
    Ok(())
}

/// Move a row from one state to another, only if it is still in `from`.
/// Returns whether this caller won the transition: the tick, a cancel and a
/// "resume now" can race, and exactly one of them may act.
fn transition(
    db: &Connection,
    session_id: &str,
    from: &[LimitRecoveryState],
    to: LimitRecoveryState,
    outcome: Option<&str>,
    now: DateTime<Utc>,
) -> Result<bool, BridgeError> {
    let from: Vec<&str> = from.iter().map(|state| state_name(*state)).collect();
    let placeholders = (0..from.len()).map(|index| format!("?{}", index + 5)).collect::<Vec<_>>().join(",");
    let sql = format!(
        "UPDATE limit_recoveries SET state=?2,outcome=COALESCE(?3,outcome),updated_at=?4
         WHERE session_id=?1 AND state IN ({placeholders})"
    );
    let mut values: Vec<Box<dyn rusqlite::ToSql>> = vec![
        Box::new(session_id.to_owned()),
        Box::new(state_name(to)),
        Box::new(outcome.map(str::to_owned)),
        Box::new(now.to_rfc3339()),
    ];
    for state in from {
        values.push(Box::new(state));
    }
    let changed = db.execute(&sql, rusqlite::params_from_iter(values.iter().map(|value| value.as_ref())))?;
    Ok(changed == 1)
}

pub fn view(row: &Recovery) -> LimitRecoveryView {
    LimitRecoveryView {
        session_id: row.session_id.clone(),
        mode: row.mode,
        state: row.state,
        signal: row.signal.clone(),
        reset_at: row.reset_at.map(|time| time.to_rfc3339()),
        due_at: row.due_at.to_rfc3339(),
        target: row.target.to_target(),
        switches_model: row.switches_model,
        attempts: row.attempts,
        outcome: row.outcome.clone(),
        updated_at: row.updated_at.to_rfc3339(),
    }
}

pub fn status(db: &Connection, session_id: &str) -> Result<wire::LimitRecoveryStatus, BridgeError> {
    Ok(wire::LimitRecoveryStatus { recovery: get(db, session_id)?.as_ref().map(view) })
}

// ---------------------------------------------------------------- planning

/// What one limit frame should lead to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// Nothing to do, and why.
    Ignore(&'static str),
    /// Schedule a recovery.
    Schedule {
        due_at: DateTime<Utc>,
        target: Selection,
        switches_model: bool,
        attempts: u32,
    },
    /// The limit kept coming back; stop.
    GiveUp { attempts: u32 },
}

/// Decide what a limit means for a chat, from plain facts.
///
/// `prior` is the chat's existing row, if any. Its attempts carry over only
/// when Bridge acted on it recently, so a person's own resend, a cancelled
/// row or a week-old success never counts toward giving up.
pub fn plan(
    settings: &LimitRecoverySettings,
    kind: LimitKind,
    reset_at: Option<DateTime<Utc>>,
    current: &Selection,
    prior: Option<&Recovery>,
    now: DateTime<Utc>,
) -> Decision {
    if settings.mode == LimitRecoveryMode::Off {
        return Decision::Ignore("off");
    }
    if kind == LimitKind::Capacity {
        return Decision::Ignore("capacity");
    }
    let streak = prior
        .filter(|row| row.state == LimitRecoveryState::Done && now - row.updated_at <= STREAK_WINDOW)
        .map_or(0, |row| row.attempts);
    let attempts = streak.saturating_add(1);
    if attempts > MAX_ATTEMPTS {
        return Decision::GiveUp { attempts: streak };
    }
    if settings.mode == LimitRecoveryMode::Backup {
        if let Some(backup) = settings.backup.as_ref().map(Selection::from_target) {
            if !backup.same_model(current) {
                return Decision::Schedule { due_at: now, target: backup, switches_model: true, attempts };
            }
        }
    }
    let due_at = match reset_at {
        Some(reset) if reset > now => reset + RESET_GRACE,
        _ => now + probe_delay(attempts),
    };
    Decision::Schedule { due_at, target: current.clone(), switches_model: false, attempts }
}

/// The wait before probing a limit that named no reset: 15 minutes for the
/// first attempt, doubling, never past two hours.
pub fn probe_delay(attempts: u32) -> Duration {
    let doublings = attempts.saturating_sub(1).min(16);
    let delay = FIRST_PROBE * 2i32.pow(doublings);
    delay.min(MAX_PROBE)
}

/// The chat as it stands when a due row comes up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatFacts {
    pub exists: bool,
    pub archived: bool,
    /// The chat's status column: `working` and `waiting` mean a turn is
    /// already in flight, which a person started.
    pub status: String,
    pub selection: Selection,
}

/// Whether a due row may still act. `Err` is the reason it may not, recorded
/// as the row's outcome.
pub fn guard(row: &Recovery, chat: &ChatFacts, now: DateTime<Utc>) -> Result<(), &'static str> {
    if !chat.exists {
        return Err("the chat was deleted");
    }
    if chat.archived {
        return Err("the chat was archived");
    }
    if now - row.due_at > CATCH_UP {
        return Err("Bridge was closed past the reset");
    }
    if matches!(chat.status.as_str(), "working" | "waiting") {
        return Err("the chat is already running");
    }
    if row.switches_model {
        // The person switched to something else themselves: theirs wins. On
        // the backup already (a second window, a retry) is fine.
        if !chat.selection.same_model(&row.from) && !chat.selection.same_model(&row.target) {
            return Err("the chat's model was changed");
        }
    } else if !chat.selection.same_model(&row.target) {
        return Err("the chat's model was changed");
    }
    Ok(())
}

// ---------------------------------------------------------------- hook

/// Record what a limit frame on a root chat should lead to. Called by
/// `live_turn` with the database lock it already holds, so it only reads and
/// writes rows. Returns the decision for the caller's ledger and tests.
pub fn observe(
    db: &Connection,
    session_id: &str,
    kind: LimitKind,
    signal: &str,
    reset_at: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> Result<Decision, BridgeError> {
    let settings = load_settings(db)?;
    if settings.mode == LimitRecoveryMode::Off {
        return Ok(Decision::Ignore("off"));
    }
    let Some((current, depth)) = db
        .query_row(
            "SELECT harness,model,effort,depth FROM sessions WHERE id=?1",
            [session_id],
            |row| {
                Ok((
                    Selection { harness: row.get(0)?, model: row.get(1)?, effort: row.get(2)? },
                    // Older rows leave depth NULL; only a worker sets it.
                    row.get::<_, Option<i64>>(3)?.unwrap_or(0),
                ))
            },
        )
        .optional()?
    else {
        return Ok(Decision::Ignore("unknown chat"));
    };
    // Workers have their own failover in the pool, and a workflow run is
    // read-only to anything but its runner.
    if depth != 0 {
        return Ok(Decision::Ignore("worker"));
    }
    if crate::workflow_run::is_workflow_session(db, session_id) {
        return Ok(Decision::Ignore("not a chat"));
    }
    let prior = get(db, session_id)?;
    // The same limit reported twice (an error frame and its compaction echo)
    // must not schedule twice or count two attempts.
    if prior.as_ref().is_some_and(|row| {
        matches!(row.state, LimitRecoveryState::Pending | LimitRecoveryState::Acting)
    }) {
        return Ok(Decision::Ignore("already scheduled"));
    }
    let decision = plan(&settings, kind, reset_at, &current, prior.as_ref(), now);
    match &decision {
        Decision::Ignore(_) => {}
        Decision::Schedule { due_at, target, switches_model, attempts } => {
            upsert(
                db,
                &Recovery {
                    session_id: session_id.to_owned(),
                    mode: settings.mode,
                    state: LimitRecoveryState::Pending,
                    signal: signal.to_owned(),
                    reset_at,
                    due_at: *due_at,
                    from: current.clone(),
                    target: target.clone(),
                    switches_model: *switches_model,
                    attempts: *attempts,
                    outcome: None,
                    observed_at: now,
                    updated_at: now,
                },
            )?;
            let _ = crate::store::event(
                db,
                "limit_recovery",
                "limit_recovery.scheduled",
                session_id,
                &format!("{} at {}", if *switches_model { "switch" } else { "resume" }, due_at.to_rfc3339()),
            );
        }
        Decision::GiveUp { attempts } => {
            let mut row = prior.clone().expect("a give-up always has a prior row");
            row.state = LimitRecoveryState::GaveUp;
            row.outcome = Some(format!("the limit came back {attempts} times in a row"));
            row.updated_at = now;
            row.signal = signal.to_owned();
            row.reset_at = reset_at;
            upsert(db, &row)?;
            let _ = crate::store::event(db, "limit_recovery", "limit_recovery.gave_up", session_id, signal);
        }
    }
    Ok(decision)
}

/// A person sent the chat a message: any pending recovery is theirs to
/// replace. Only `pending` rows are touched, so the tick's own send (which
/// holds the row in `acting`) does not cancel itself.
///
/// A message after a recovery Bridge finished also ends its streak: the next
/// limit is on the person's turn, not a repeat of Bridge's.
pub fn cancel_for_user_input(db: &Connection, session_id: &str, now: DateTime<Utc>) -> Result<bool, BridgeError> {
    db.execute(
        "UPDATE limit_recoveries SET attempts=0 WHERE session_id=?1 AND state='done'",
        [session_id],
    )?;
    transition(
        db,
        session_id,
        &[LimitRecoveryState::Pending],
        LimitRecoveryState::Cancelled,
        Some("you sent a message"),
        now,
    )
}

pub fn cancel(db: &Connection, session_id: &str, now: DateTime<Utc>) -> Result<bool, BridgeError> {
    transition(
        db,
        session_id,
        &[LimitRecoveryState::Pending],
        LimitRecoveryState::Cancelled,
        Some("you cancelled it"),
        now,
    )
}

/// Make a pending row due now. The tick picks it up on its next pass; the
/// caller may also run [`tick`] itself to act immediately.
pub fn make_due_now(db: &Connection, session_id: &str, now: DateTime<Utc>) -> Result<bool, BridgeError> {
    let changed = db.execute(
        "UPDATE limit_recoveries SET due_at=?2,updated_at=?2 WHERE session_id=?1 AND state='pending'",
        params![session_id, now.to_rfc3339()],
    )?;
    Ok(changed == 1)
}

/// At start: a row left `acting` by a crash mid-act goes back to `pending`,
/// so it is re-guarded and acted on rather than stranded.
pub fn recover_interrupted(db: &Connection, now: DateTime<Utc>) -> Result<usize, BridgeError> {
    Ok(db.execute(
        "UPDATE limit_recoveries SET state='pending',updated_at=?1 WHERE state='acting'",
        [now.to_rfc3339()],
    )?)
}

pub fn has_pending(db: &Connection) -> bool {
    db.query_row("SELECT EXISTS(SELECT 1 FROM limit_recoveries WHERE state='pending')", [], |row| row.get(0))
        .unwrap_or(false)
}

// ---------------------------------------------------------------- acting

/// The two things acting on a row needs from the rest of Bridge.
pub trait Actions {
    fn switch_model(&self, session_id: &str, target: &Selection) -> Result<(), BridgeError>;
    fn send_continue(&self, session_id: &str, text: &str) -> Result<(), BridgeError>;
}

/// What a pass over due rows did, for the loop and for tests.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TickReport {
    pub acted: Vec<String>,
    pub cancelled: Vec<(String, &'static str)>,
    pub failed: Vec<(String, String)>,
}

impl TickReport {
    pub fn changed(&self) -> bool {
        !(self.acted.is_empty() && self.cancelled.is_empty() && self.failed.is_empty())
    }
}

fn chat_facts(db: &Connection, session_id: &str) -> Result<ChatFacts, BridgeError> {
    let row = db
        .query_row(
            "SELECT harness,model,effort,status,archived_at FROM sessions WHERE id=?1",
            [session_id],
            |row| {
                Ok(ChatFacts {
                    exists: true,
                    archived: row.get::<_, Option<String>>(4)?.is_some(),
                    status: row.get(3)?,
                    selection: Selection { harness: row.get(0)?, model: row.get(1)?, effort: row.get(2)? },
                })
            },
        )
        .optional()?;
    Ok(row.unwrap_or(ChatFacts {
        exists: false,
        archived: false,
        status: String::new(),
        selection: Selection { harness: String::new(), model: None, effort: None },
    }))
}

/// Act on every due row. Holds the database lock only to read and to move
/// rows between states, never across an action: switching a model and
/// starting a turn both take the lock themselves.
pub fn tick(
    db: &std::sync::Mutex<Connection>,
    actions: &dyn Actions,
    now: DateTime<Utc>,
) -> Result<TickReport, BridgeError> {
    let due: Vec<Recovery> = {
        let db = db.lock().unwrap();
        let mut statement = db.prepare(&format!(
            "SELECT {COLUMNS} FROM limit_recoveries WHERE state='pending' AND due_at<=?1 ORDER BY due_at"
        ))?;
        let rows = statement.query_map([now.to_rfc3339()], read_row)?.collect::<Result<Vec<_>, _>>()?;
        rows
    };
    let mut report = TickReport::default();
    for row in due {
        let verdict = {
            let db = db.lock().unwrap();
            // Claim first: a cancel or a second tick racing this one loses.
            if !transition(&db, &row.session_id, &[LimitRecoveryState::Pending], LimitRecoveryState::Acting, None, now)? {
                continue;
            }
            let facts = chat_facts(&db, &row.session_id)?;
            match guard(&row, &facts, now) {
                Ok(()) => Ok(!facts.selection.same_model(&row.target)),
                Err(reason) => {
                    let state = if reason == "Bridge was closed past the reset" {
                        LimitRecoveryState::Expired
                    } else {
                        LimitRecoveryState::Cancelled
                    };
                    transition(&db, &row.session_id, &[LimitRecoveryState::Acting], state, Some(reason), now)?;
                    let _ = crate::store::event(&db, "limit_recovery", "limit_recovery.cancelled", &row.session_id, reason);
                    Err(reason)
                }
            }
        };
        let needs_switch = match verdict {
            Ok(needs_switch) => needs_switch,
            Err(reason) => {
                report.cancelled.push((row.session_id.clone(), reason));
                continue;
            }
        };
        let result = (|| {
            if needs_switch {
                actions.switch_model(&row.session_id, &row.target)?;
            }
            let prompt = if row.switches_model { CONTINUE_ON_BACKUP_PROMPT } else { CONTINUE_PROMPT };
            actions.send_continue(&row.session_id, prompt)
        })();
        let db = db.lock().unwrap();
        match result {
            Ok(()) => {
                let outcome = if row.switches_model { "switched to the backup model and continued" } else { "continued after the reset" };
                transition(&db, &row.session_id, &[LimitRecoveryState::Acting], LimitRecoveryState::Done, Some(outcome), now)?;
                let _ = crate::store::event(&db, "limit_recovery", "limit_recovery.acted", &row.session_id, outcome);
                report.acted.push(row.session_id.clone());
            }
            Err(error) => {
                let message = error.to_string();
                transition(&db, &row.session_id, &[LimitRecoveryState::Acting], LimitRecoveryState::Failed, Some(&message), now)?;
                let _ = crate::store::event(&db, "limit_recovery", "limit_recovery.failed", &row.session_id, &message);
                report.failed.push((row.session_id.clone(), message));
            }
        }
    }
    Ok(report)
}

/// The live [`Actions`]: the same model switch the composer's picker uses and
/// the same input path a person's message takes.
pub struct LiveActions<'a> {
    pub core: &'a std::sync::Arc<crate::runtime::BridgeCore>,
}

impl Actions for LiveActions<'_> {
    fn switch_model(&self, session_id: &str, target: &Selection) -> Result<(), BridgeError> {
        let harness = crate::model::Harness::parse(&target.harness)
            .map_err(|error| BridgeError::Invalid(format!("Backup harness `{}`: {error}", target.harness)))?;
        crate::api::update_chat_model(self.core, session_id, &harness, target.model.as_deref(), target.effort.as_deref())?;
        Ok(())
    }

    fn send_continue(&self, session_id: &str, text: &str) -> Result<(), BridgeError> {
        crate::live_turn::submit_recovery_turn(self.core, session_id.to_owned(), text.to_owned()).map(|_| ())
    }
}

/// How often the loop looks for due rows. Overridable for the end-to-end
/// test, which cannot wait twenty seconds a step.
fn tick_interval() -> std::time::Duration {
    std::env::var("BRIDGE_LIMIT_RECOVERY_TICK_MS")
        .ok()
        .and_then(|value| value.parse().ok())
        .map(std::time::Duration::from_millis)
        .unwrap_or(std::time::Duration::from_secs(20))
}

/// Start the loop. Part of data-directory ownership: the daemon and the
/// embedded host both call it, exactly once.
pub fn start_limit_recovery_maintenance(core: std::sync::Arc<crate::runtime::BridgeCore>) {
    let _ = std::thread::Builder::new().name("limit-recovery".into()).spawn(move || {
        if let Err(error) = recover_interrupted(&core.db.lock().unwrap(), Utc::now()) {
            eprintln!("[limit-recovery] could not recover interrupted rows: {error}");
        }
        let mut power = PowerAssertion::default();
        loop {
            match tick(&core.db, &LiveActions { core: &core }, Utc::now()) {
                Ok(report) if report.changed() => core.events.publish(crate::events::CoreEvent::StateChanged),
                Ok(_) => {}
                Err(error) => eprintln!("[limit-recovery] tick failed: {error}"),
            }
            power.hold(has_pending(&core.db.lock().unwrap()));
            std::thread::sleep(tick_interval());
        }
    });
}

/// Keeps the Mac from idle-sleeping while a recovery waits. A `caffeinate -i`
/// child tied to this process with `-w`, so it ends with Bridge even if Bridge
/// is killed. The display may still sleep; the lid still sleeps the machine.
#[derive(Default)]
pub struct PowerAssertion {
    child: Option<std::process::Child>,
}

impl PowerAssertion {
    pub fn hold(&mut self, wanted: bool) {
        if let Some(child) = &mut self.child {
            // A caffeinate that exited (killed by hand) is re-taken below.
            if matches!(child.try_wait(), Ok(Some(_))) {
                self.child = None;
            }
        }
        match (wanted, self.child.is_some()) {
            (true, false) => self.child = spawn_caffeinate(),
            (false, true) => {
                if let Some(mut child) = self.child.take() {
                    let _ = child.kill();
                    let _ = child.wait();
                }
            }
            _ => {}
        }
    }

    pub fn holding(&self) -> bool {
        self.child.is_some()
    }
}

impl Drop for PowerAssertion {
    fn drop(&mut self) {
        self.hold(false);
    }
}

#[cfg(target_os = "macos")]
fn spawn_caffeinate() -> Option<std::process::Child> {
    if std::env::var_os("BRIDGE_LIMIT_RECOVERY_NO_CAFFEINATE").is_some() {
        return None;
    }
    std::process::Command::new("/usr/bin/caffeinate")
        .args(["-i", "-w", &std::process::id().to_string()])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()
}

#[cfg(not(target_os = "macos"))]
fn spawn_caffeinate() -> Option<std::process::Child> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use std::sync::Mutex;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 15, 10, 0, 0).unwrap()
    }

    fn sel(harness: &str, model: &str) -> Selection {
        Selection { harness: harness.into(), model: Some(model.into()), effort: Some("high".into()) }
    }

    fn resume() -> LimitRecoverySettings {
        LimitRecoverySettings { mode: LimitRecoveryMode::Resume, backup: None }
    }

    fn backup(harness: &str, model: &str) -> LimitRecoverySettings {
        LimitRecoverySettings {
            mode: LimitRecoveryMode::Backup,
            backup: Some(LimitRecoveryTarget { harness: harness.into(), model: Some(model.into()), effort: Some(Effort::Medium) }),
        }
    }

    fn db() -> Connection {
        let db = crate::store::open(std::path::Path::new(":memory:")).unwrap();
        db.execute(
            "INSERT INTO sessions(id,workspace_id,harness,label,status,model,effort) VALUES('chat',NULL,'claude','Chat','failed','opus','high')",
            [],
        )
        .unwrap();
        db
    }

    fn set(db: &Connection, settings: &LimitRecoverySettings) {
        save_settings(db, settings, |_| true).unwrap();
    }

    /// Records what the tick asked for, and can be told to fail.
    #[derive(Default)]
    struct Recorder {
        calls: Mutex<Vec<String>>,
        fail_send: bool,
    }

    impl Actions for Recorder {
        fn switch_model(&self, session_id: &str, target: &Selection) -> Result<(), BridgeError> {
            self.calls.lock().unwrap().push(format!("switch {session_id} {}:{}", target.harness, target.model.clone().unwrap_or_default()));
            Ok(())
        }
        fn send_continue(&self, session_id: &str, text: &str) -> Result<(), BridgeError> {
            if self.fail_send {
                return Err(BridgeError::Invalid("provider is not running".into()));
            }
            self.calls.lock().unwrap().push(format!("send {session_id} {}", if text == CONTINUE_PROMPT { "continue" } else { "continue-on-backup" }));
            Ok(())
        }
    }

    // ------------------------------------------------------------ plan

    #[test]
    fn off_and_capacity_never_schedule() {
        let current = sel("claude", "opus");
        assert_eq!(plan(&LimitRecoverySettings::default(), LimitKind::Usage, None, &current, None, now()), Decision::Ignore("off"));
        assert_eq!(plan(&resume(), LimitKind::Capacity, None, &current, None, now()), Decision::Ignore("capacity"));
        assert_eq!(plan(&backup("codex", "gpt"), LimitKind::Capacity, None, &current, None, now()), Decision::Ignore("capacity"));
    }

    #[test]
    fn resume_waits_for_the_stated_reset_plus_grace_on_the_same_model() {
        let reset = now() + Duration::hours(3);
        let current = sel("claude", "opus");
        assert_eq!(
            plan(&resume(), LimitKind::Usage, Some(reset), &current, None, now()),
            Decision::Schedule { due_at: reset + RESET_GRACE, target: current.clone(), switches_model: false, attempts: 1 }
        );
    }

    #[test]
    fn resume_without_a_reset_probes_with_a_doubling_capped_backoff() {
        assert_eq!(probe_delay(1), Duration::minutes(15));
        assert_eq!(probe_delay(2), Duration::minutes(30));
        assert_eq!(probe_delay(3), Duration::minutes(60));
        assert_eq!(probe_delay(4), Duration::minutes(120));
        assert_eq!(probe_delay(5), Duration::minutes(120));
        assert_eq!(probe_delay(0), Duration::minutes(15));
        assert_eq!(probe_delay(u32::MAX), MAX_PROBE);
        let current = sel("codex", "gpt");
        let Decision::Schedule { due_at, .. } = plan(&resume(), LimitKind::Usage, None, &current, None, now()) else { panic!() };
        assert_eq!(due_at, now() + Duration::minutes(15));
    }

    #[test]
    fn a_reset_already_in_the_past_probes_rather_than_firing_at_once() {
        let current = sel("claude", "opus");
        let Decision::Schedule { due_at, .. } = plan(&resume(), LimitKind::Usage, Some(now() - Duration::minutes(1)), &current, None, now()) else { panic!() };
        assert_eq!(due_at, now() + FIRST_PROBE);
        let Decision::Schedule { due_at, .. } = plan(&resume(), LimitKind::Usage, Some(now()), &current, None, now()) else { panic!() };
        assert_eq!(due_at, now() + FIRST_PROBE, "a reset of exactly now is not ahead of now");
    }

    #[test]
    fn backup_switches_now_and_a_chat_already_on_the_backup_waits_for_its_reset() {
        let settings = backup("codex", "gpt-5.5");
        let on_primary = sel("claude", "opus");
        assert_eq!(
            plan(&settings, LimitKind::Usage, Some(now() + Duration::hours(1)), &on_primary, None, now()),
            Decision::Schedule {
                due_at: now(),
                target: Selection { harness: "codex".into(), model: Some("gpt-5.5".into()), effort: Some("medium".into()) },
                switches_model: true,
                attempts: 1,
            }
        );
        // On the backup at any effort is on the backup.
        let on_backup = Selection { harness: "codex".into(), model: Some("gpt-5.5".into()), effort: Some("low".into()) };
        let reset = now() + Duration::hours(2);
        assert_eq!(
            plan(&settings, LimitKind::Usage, Some(reset), &on_backup, None, now()),
            Decision::Schedule { due_at: reset + RESET_GRACE, target: on_backup.clone(), switches_model: false, attempts: 1 }
        );
    }

    #[test]
    fn same_model_on_another_harness_is_not_the_backup() {
        let settings = backup("opencode", "opus");
        let Decision::Schedule { switches_model, .. } = plan(&settings, LimitKind::Usage, None, &sel("claude", "opus"), None, now()) else { panic!() };
        assert!(switches_model);
    }

    fn row(state: LimitRecoveryState, attempts: u32, updated_at: DateTime<Utc>) -> Recovery {
        Recovery {
            session_id: "chat".into(),
            mode: LimitRecoveryMode::Resume,
            state,
            signal: "usage limit".into(),
            reset_at: None,
            due_at: updated_at,
            from: sel("claude", "opus"),
            target: sel("claude", "opus"),
            switches_model: false,
            attempts,
            outcome: None,
            observed_at: updated_at,
            updated_at,
        }
    }

    #[test]
    fn attempts_carry_only_across_recent_rows_bridge_acted_on() {
        let current = sel("claude", "opus");
        let attempts = |prior: &Recovery| match plan(&resume(), LimitKind::Usage, None, &current, Some(prior), now()) {
            Decision::Schedule { attempts, .. } => attempts,
            other => panic!("{other:?}"),
        };
        assert_eq!(attempts(&row(LimitRecoveryState::Done, 2, now() - Duration::hours(1))), 3);
        assert_eq!(attempts(&row(LimitRecoveryState::Done, 2, now() - STREAK_WINDOW)), 3, "the window is inclusive");
        assert_eq!(attempts(&row(LimitRecoveryState::Done, 2, now() - STREAK_WINDOW - Duration::seconds(1))), 1);
        for state in [LimitRecoveryState::Cancelled, LimitRecoveryState::Failed, LimitRecoveryState::Expired, LimitRecoveryState::GaveUp] {
            assert_eq!(attempts(&row(state, 5, now())), 1, "{state:?} does not count");
        }
    }

    #[test]
    fn the_streak_gives_up_after_max_attempts() {
        let current = sel("claude", "opus");
        let prior = row(LimitRecoveryState::Done, MAX_ATTEMPTS - 1, now());
        assert!(matches!(plan(&resume(), LimitKind::Usage, None, &current, Some(&prior), now()), Decision::Schedule { attempts, .. } if attempts == MAX_ATTEMPTS));
        let prior = row(LimitRecoveryState::Done, MAX_ATTEMPTS, now());
        assert_eq!(plan(&resume(), LimitKind::Usage, None, &current, Some(&prior), now()), Decision::GiveUp { attempts: MAX_ATTEMPTS });
    }

    // ------------------------------------------------------------ guard

    fn facts(status: &str, selection: Selection) -> ChatFacts {
        ChatFacts { exists: true, archived: false, status: status.into(), selection }
    }

    #[test]
    fn guard_refuses_gone_archived_late_running_and_rerouted_chats() {
        let due = row(LimitRecoveryState::Acting, 1, now());
        let ok = facts("failed", sel("claude", "opus"));
        assert_eq!(guard(&due, &ok, now()), Ok(()));
        assert_eq!(guard(&due, &facts("idle", sel("claude", "opus")), now()), Ok(()));
        assert_eq!(guard(&due, &ChatFacts { exists: false, ..ok.clone() }, now()), Err("the chat was deleted"));
        assert_eq!(guard(&due, &ChatFacts { archived: true, ..ok.clone() }, now()), Err("the chat was archived"));
        assert_eq!(guard(&due, &facts("working", sel("claude", "opus")), now()), Err("the chat is already running"));
        assert_eq!(guard(&due, &facts("waiting", sel("claude", "opus")), now()), Err("the chat is already running"));
        assert_eq!(guard(&due, &facts("failed", sel("claude", "sonnet")), now()), Err("the chat's model was changed"));
        assert_eq!(guard(&due, &ok, now() + CATCH_UP), Ok(()), "the catch-up window is inclusive");
        assert_eq!(guard(&due, &ok, now() + CATCH_UP + Duration::seconds(1)), Err("Bridge was closed past the reset"));
    }

    #[test]
    fn a_switch_row_accepts_the_original_or_the_backup_but_not_a_third_model() {
        let mut due = row(LimitRecoveryState::Acting, 1, now());
        due.switches_model = true;
        due.target = sel("codex", "gpt");
        assert_eq!(guard(&due, &facts("failed", sel("claude", "opus")), now()), Ok(()));
        assert_eq!(guard(&due, &facts("failed", sel("codex", "gpt")), now()), Ok(()));
        assert_eq!(guard(&due, &facts("failed", sel("opencode", "kimi")), now()), Err("the chat's model was changed"));
    }

    // ------------------------------------------------------------ settings

    #[test]
    fn settings_default_off_and_backup_mode_needs_a_backup() {
        let db = db();
        assert_eq!(load_settings(&db).unwrap(), LimitRecoverySettings::default());
        let missing = LimitRecoverySettings { mode: LimitRecoveryMode::Backup, backup: None };
        assert!(save_settings(&db, &missing, |_| true).is_err());
        let blank = LimitRecoverySettings {
            mode: LimitRecoveryMode::Backup,
            backup: Some(LimitRecoveryTarget { harness: "  ".into(), model: None, effort: None }),
        };
        assert!(save_settings(&db, &blank, |_| true).is_err(), "a blank harness is no backup");
        assert!(save_settings(&db, &backup("nope", "x"), |id| id == "codex").is_err());
        let saved = save_settings(
            &db,
            &LimitRecoverySettings {
                mode: LimitRecoveryMode::Backup,
                backup: Some(LimitRecoveryTarget { harness: " Codex ".into(), model: Some("  ".into()), effort: None }),
            },
            |id| id == "codex",
        )
        .unwrap();
        assert_eq!(saved.backup.unwrap(), LimitRecoveryTarget { harness: "codex".into(), model: None, effort: None });
        // Resume keeps a chosen backup for later.
        let kept = save_settings(&db, &LimitRecoverySettings { mode: LimitRecoveryMode::Resume, ..backup("codex", "gpt") }, |_| true).unwrap();
        assert_eq!(kept.backup.unwrap().model.as_deref(), Some("gpt"));
    }

    #[test]
    fn an_unreadable_settings_record_reads_as_off() {
        let db = db();
        db.execute(
            "INSERT INTO configuration_entries(kind,id,payload,created_at,updated_at) VALUES(?1,?2,'{\"mode\":\"sometimes\"}','x','x')",
            params![SETTINGS_KIND, SETTINGS_ID],
        )
        .unwrap();
        assert_eq!(load_settings(&db).unwrap().mode, LimitRecoveryMode::Off);
    }

    // ------------------------------------------------------------ observe

    #[test]
    fn observe_writes_nothing_when_off_and_one_row_when_on() {
        let db = db();
        assert_eq!(observe(&db, "chat", LimitKind::Usage, "usage limit", None, now()).unwrap(), Decision::Ignore("off"));
        assert!(get(&db, "chat").unwrap().is_none());
        set(&db, &resume());
        let reset = now() + Duration::hours(2);
        observe(&db, "chat", LimitKind::Usage, "usage limit", Some(reset), now()).unwrap();
        let row = get(&db, "chat").unwrap().unwrap();
        assert_eq!(row.state, LimitRecoveryState::Pending);
        assert_eq!(row.due_at, reset + RESET_GRACE);
        assert_eq!(row.target, Selection { harness: "claude".into(), model: Some("opus".into()), effort: Some("high".into()) });
        assert_eq!(row.from, row.target);
        assert!(has_pending(&db));
        // A second frame for the same limit does not reschedule or count.
        assert_eq!(
            observe(&db, "chat", LimitKind::Usage, "usage limit", Some(reset + Duration::hours(5)), now()).unwrap(),
            Decision::Ignore("already scheduled")
        );
        assert_eq!(get(&db, "chat").unwrap().unwrap(), row);
        let view = status(&db, "chat").unwrap().recovery.unwrap();
        assert_eq!(view.state, LimitRecoveryState::Pending);
        assert_eq!(view.target.effort, Some(Effort::High));
        assert_eq!(view.reset_at, Some(reset.to_rfc3339()));
    }

    #[test]
    fn observe_ignores_workers_and_unknown_chats() {
        let db = db();
        set(&db, &resume());
        db.execute(
            "INSERT INTO sessions(id,workspace_id,harness,label,status,depth,parent_session_id) VALUES('worker',NULL,'codex','W','failed',1,'chat')",
            [],
        )
        .unwrap();
        assert_eq!(observe(&db, "worker", LimitKind::Usage, "usage limit", None, now()).unwrap(), Decision::Ignore("worker"));
        assert_eq!(observe(&db, "ghost", LimitKind::Usage, "usage limit", None, now()).unwrap(), Decision::Ignore("unknown chat"));
        assert!(get(&db, "worker").unwrap().is_none());
    }

    #[test]
    fn observe_records_a_give_up_without_scheduling() {
        let db = db();
        set(&db, &resume());
        upsert(&db, &row(LimitRecoveryState::Done, MAX_ATTEMPTS, now())).unwrap();
        assert_eq!(observe(&db, "chat", LimitKind::Usage, "usage limit", None, now()).unwrap(), Decision::GiveUp { attempts: MAX_ATTEMPTS });
        let row = get(&db, "chat").unwrap().unwrap();
        assert_eq!(row.state, LimitRecoveryState::GaveUp);
        assert!(row.outcome.unwrap().contains("6 times"));
        assert!(!has_pending(&db));
    }

    // ------------------------------------------------------------ tick

    #[test]
    fn tick_waits_for_due_then_continues_on_the_same_model() {
        let db = db();
        set(&db, &resume());
        let reset = now() + Duration::hours(1);
        observe(&db, "chat", LimitKind::Usage, "usage limit", Some(reset), now()).unwrap();
        let db = Mutex::new(db);
        let actions = Recorder::default();
        let early = tick(&db, &actions, reset).unwrap();
        assert!(!early.changed(), "the reset itself is inside the grace");
        assert!(actions.calls.lock().unwrap().is_empty());
        let report = tick(&db, &actions, reset + RESET_GRACE).unwrap();
        assert_eq!(report.acted, vec!["chat".to_owned()]);
        assert_eq!(*actions.calls.lock().unwrap(), vec!["send chat continue".to_owned()]);
        let row = get(&db.lock().unwrap(), "chat").unwrap().unwrap();
        assert_eq!(row.state, LimitRecoveryState::Done);
        // Acting is once: a second pass does nothing.
        assert!(!tick(&db, &actions, reset + Duration::hours(5)).unwrap().changed());
        assert_eq!(actions.calls.lock().unwrap().len(), 1);
    }

    #[test]
    fn tick_switches_to_the_backup_before_continuing() {
        let db = db();
        set(&db, &backup("codex", "gpt-5.5"));
        observe(&db, "chat", LimitKind::Usage, "usage limit", None, now()).unwrap();
        let db = Mutex::new(db);
        let actions = Recorder::default();
        let report = tick(&db, &actions, now()).unwrap();
        assert_eq!(report.acted, vec!["chat".to_owned()]);
        assert_eq!(
            *actions.calls.lock().unwrap(),
            vec!["switch chat codex:gpt-5.5".to_owned(), "send chat continue-on-backup".to_owned()]
        );
    }

    #[test]
    fn a_backup_switch_already_made_is_not_made_twice() {
        let db = db();
        set(&db, &backup("codex", "gpt-5.5"));
        observe(&db, "chat", LimitKind::Usage, "usage limit", None, now()).unwrap();
        db.execute("UPDATE sessions SET harness='codex',model='gpt-5.5' WHERE id='chat'", []).unwrap();
        let db = Mutex::new(db);
        let actions = Recorder::default();
        tick(&db, &actions, now()).unwrap();
        assert_eq!(*actions.calls.lock().unwrap(), vec!["send chat continue-on-backup".to_owned()]);
    }

    #[test]
    fn a_person_writing_first_cancels_and_the_tick_then_does_nothing() {
        let db = db();
        set(&db, &resume());
        observe(&db, "chat", LimitKind::Usage, "usage limit", None, now()).unwrap();
        assert!(cancel_for_user_input(&db, "chat", now()).unwrap());
        assert!(!cancel_for_user_input(&db, "chat", now()).unwrap(), "only once");
        let db = Mutex::new(db);
        let actions = Recorder::default();
        assert!(!tick(&db, &actions, now() + Duration::days(1)).unwrap().changed());
        assert!(actions.calls.lock().unwrap().is_empty());
        let row = get(&db.lock().unwrap(), "chat").unwrap().unwrap();
        assert_eq!((row.state, row.outcome.as_deref()), (LimitRecoveryState::Cancelled, Some("you sent a message")));
    }

    #[test]
    fn the_tick_cancels_a_row_whose_chat_changed_under_it() {
        let db = db();
        set(&db, &resume());
        observe(&db, "chat", LimitKind::Usage, "usage limit", None, now()).unwrap();
        db.execute("UPDATE sessions SET model='sonnet' WHERE id='chat'", []).unwrap();
        let db = Mutex::new(db);
        let actions = Recorder::default();
        let report = tick(&db, &actions, now() + Duration::hours(1)).unwrap();
        assert_eq!(report.cancelled, vec![("chat".to_owned(), "the chat's model was changed")]);
        assert!(actions.calls.lock().unwrap().is_empty());
        assert_eq!(get(&db.lock().unwrap(), "chat").unwrap().unwrap().state, LimitRecoveryState::Cancelled);
    }

    #[test]
    fn a_row_overdue_past_catch_up_expires_instead_of_firing() {
        let db = db();
        set(&db, &resume());
        observe(&db, "chat", LimitKind::Usage, "usage limit", None, now()).unwrap();
        let db = Mutex::new(db);
        let actions = Recorder::default();
        tick(&db, &actions, now() + FIRST_PROBE + CATCH_UP + Duration::minutes(1)).unwrap();
        assert!(actions.calls.lock().unwrap().is_empty());
        assert_eq!(get(&db.lock().unwrap(), "chat").unwrap().unwrap().state, LimitRecoveryState::Expired);
    }

    #[test]
    fn a_failed_send_is_recorded_and_not_retried() {
        let db = db();
        set(&db, &resume());
        observe(&db, "chat", LimitKind::Usage, "usage limit", None, now()).unwrap();
        let db = Mutex::new(db);
        let actions = Recorder { fail_send: true, ..Recorder::default() };
        let report = tick(&db, &actions, now() + Duration::hours(1)).unwrap();
        assert_eq!(report.failed.len(), 1);
        let row = get(&db.lock().unwrap(), "chat").unwrap().unwrap();
        assert_eq!(row.state, LimitRecoveryState::Failed);
        assert!(row.outcome.unwrap().contains("provider is not running"));
        assert!(!tick(&db, &actions, now() + Duration::hours(2)).unwrap().changed());
    }

    #[test]
    fn a_crash_mid_act_is_retried_after_restart() {
        let db = db();
        set(&db, &resume());
        observe(&db, "chat", LimitKind::Usage, "usage limit", None, now()).unwrap();
        assert!(transition(&db, "chat", &[LimitRecoveryState::Pending], LimitRecoveryState::Acting, None, now()).unwrap());
        assert!(!has_pending(&db));
        assert_eq!(recover_interrupted(&db, now()).unwrap(), 1);
        assert!(has_pending(&db));
    }

    #[test]
    fn resume_now_makes_only_a_pending_row_due() {
        let db = db();
        assert!(!make_due_now(&db, "chat", now()).unwrap());
        set(&db, &resume());
        observe(&db, "chat", LimitKind::Usage, "usage limit", Some(now() + Duration::hours(4)), now()).unwrap();
        assert!(make_due_now(&db, "chat", now()).unwrap());
        let db = Mutex::new(db);
        let actions = Recorder::default();
        assert_eq!(tick(&db, &actions, now()).unwrap().acted.len(), 1);
        assert!(!make_due_now(&db.lock().unwrap(), "chat", now()).unwrap());
    }

    #[test]
    fn a_new_limit_after_a_successful_resume_counts_the_streak() {
        let db = db();
        set(&db, &resume());
        observe(&db, "chat", LimitKind::Usage, "usage limit", None, now()).unwrap();
        let db = Mutex::new(db);
        tick(&db, &Recorder::default(), now() + FIRST_PROBE).unwrap();
        let later = now() + FIRST_PROBE + Duration::minutes(1);
        let decision = observe(&db.lock().unwrap(), "chat", LimitKind::Usage, "usage limit", None, later).unwrap();
        assert!(matches!(decision, Decision::Schedule { attempts: 2, due_at, .. } if due_at == later + Duration::minutes(30)));
    }

    #[test]
    fn a_person_writing_after_a_recovery_restarts_the_count() {
        let db = db();
        set(&db, &resume());
        upsert(&db, &row(LimitRecoveryState::Done, 4, now())).unwrap();
        assert!(!cancel_for_user_input(&db, "chat", now()).unwrap(), "nothing pending to cancel");
        let decision = observe(&db, "chat", LimitKind::Usage, "usage limit", None, now()).unwrap();
        assert!(matches!(decision, Decision::Schedule { attempts: 1, .. }), "{decision:?}");
    }

    #[test]
    fn concurrent_ticks_act_exactly_once() {
        let db = db();
        set(&db, &resume());
        observe(&db, "chat", LimitKind::Usage, "usage limit", None, now()).unwrap();
        let db = std::sync::Arc::new(Mutex::new(db));
        let actions = std::sync::Arc::new(Recorder::default());
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let (db, actions) = (db.clone(), actions.clone());
                std::thread::spawn(move || tick(&db, actions.as_ref(), now() + Duration::hours(1)).unwrap())
            })
            .collect();
        let acted: usize = handles.into_iter().map(|handle| handle.join().unwrap().acted.len()).sum();
        assert_eq!(acted, 1);
        assert_eq!(actions.calls.lock().unwrap().len(), 1);
    }

    #[test]
    fn the_migration_installs_the_table() {
        let db = db();
        let exists: bool = db
            .query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='limit_recoveries')", [], |row| row.get(0))
            .unwrap();
        assert!(exists);
    }

    #[test]
    fn the_power_assertion_holds_only_while_wanted() {
        let mut power = PowerAssertion::default();
        power.hold(false);
        assert!(!power.holding());
        if cfg!(target_os = "macos") && std::env::var_os("BRIDGE_LIMIT_RECOVERY_NO_CAFFEINATE").is_none() {
            power.hold(true);
            assert!(power.holding());
            power.hold(true);
            assert!(power.holding(), "holding twice keeps one assertion");
        }
        power.hold(false);
        assert!(!power.holding());
    }

    // ------------------------------------------------------------ fuzz

    mod fuzz {
        use super::*;
        use proptest::prelude::*;

        fn any_state() -> impl Strategy<Value = LimitRecoveryState> {
            prop_oneof![
                Just(LimitRecoveryState::Pending),
                Just(LimitRecoveryState::Acting),
                Just(LimitRecoveryState::Done),
                Just(LimitRecoveryState::Cancelled),
                Just(LimitRecoveryState::GaveUp),
                Just(LimitRecoveryState::Expired),
                Just(LimitRecoveryState::Failed),
            ]
        }

        fn any_selection() -> impl Strategy<Value = Selection> {
            (prop::sample::select(vec!["claude", "codex", "opencode"]), prop::option::of(prop::sample::select(vec!["a", "b", "c"])), prop::option::of(prop::sample::select(vec!["low", "high"])))
                .prop_map(|(harness, model, effort)| Selection { harness: harness.into(), model: model.map(str::to_owned), effort: effort.map(str::to_owned) })
        }

        fn any_settings() -> impl Strategy<Value = LimitRecoverySettings> {
            (
                prop::sample::select(vec![LimitRecoveryMode::Off, LimitRecoveryMode::Resume, LimitRecoveryMode::Backup]),
                prop::option::of(any_selection()),
            )
                .prop_map(|(mode, backup)| LimitRecoverySettings {
                    mode,
                    backup: backup.map(|selection| LimitRecoveryTarget { harness: selection.harness, model: selection.model, effort: Some(Effort::High) }),
                })
        }

        proptest! {
            #![proptest_config(ProptestConfig { cases: 2048, ..ProptestConfig::default() })]

            /// For any settings, limit, chat and history: never schedules in
            /// the past, never more than the probe cap or reset+grace ahead,
            /// never past the attempt cap, switches only in backup mode to the
            /// backup, and otherwise keeps the chat on its own model.
            #[test]
            fn plan_invariants(
                settings in any_settings(),
                usage in any::<bool>(),
                reset_minutes in prop::option::of(-600i64..(60 * 24 * 30)),
                current in any_selection(),
                prior in prop::option::of((any_state(), 0u32..10, 0i64..(60 * 24))),
            ) {
                let kind = if usage { LimitKind::Usage } else { LimitKind::Capacity };
                let reset = reset_minutes.map(|minutes| now() + Duration::minutes(minutes));
                let prior = prior.map(|(state, attempts, age)| row(state, attempts, now() - Duration::minutes(age)));
                match plan(&settings, kind, reset, &current, prior.as_ref(), now()) {
                    Decision::Ignore(_) => prop_assert!(settings.mode == LimitRecoveryMode::Off || !usage),
                    Decision::GiveUp { attempts } => {
                        prop_assert!(attempts >= MAX_ATTEMPTS);
                        prop_assert!(prior.is_some_and(|row| row.state == LimitRecoveryState::Done));
                    }
                    Decision::Schedule { due_at, target, switches_model, attempts } => {
                        prop_assert!(usage && settings.mode != LimitRecoveryMode::Off);
                        prop_assert!((1..=MAX_ATTEMPTS).contains(&attempts));
                        prop_assert!(due_at >= now());
                        if switches_model {
                            prop_assert_eq!(settings.mode, LimitRecoveryMode::Backup);
                            prop_assert_eq!(due_at, now());
                            prop_assert!(!target.same_model(&current));
                        } else {
                            prop_assert_eq!(&target, &current);
                            let bound = match reset { Some(reset) if reset > now() => reset + RESET_GRACE, _ => now() + MAX_PROBE };
                            prop_assert!(due_at <= bound && due_at > now());
                        }
                    }
                }
            }

        }

        proptest! {
            // Each case opens a fresh store and runs every migration, so this
            // one is cheaper per case to keep at a lower count.
            #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]

            /// Any interleaving of limits, user messages, cancels, resume-now
            /// and ticks against the real store: the chat is sent at most one
            /// continue per scheduled row, never while a person's message
            /// cancelled it, and never anything at all when the setting is off.
            #[test]
            fn store_and_tick_interleavings(
                mode in prop::sample::select(vec![LimitRecoveryMode::Off, LimitRecoveryMode::Resume, LimitRecoveryMode::Backup]),
                steps in prop::collection::vec((0u8..6, 0i64..240), 1..30),
            ) {
                let db = db();
                set(&db, &LimitRecoverySettings { mode, backup: Some(LimitRecoveryTarget { harness: "codex".into(), model: Some("gpt".into()), effort: None }) });
                let db = Mutex::new(db);
                let actions = Recorder::default();
                let mut clock = now();
                let mut scheduled = 0usize;
                let mut sends = 0usize;
                for (step, minutes) in steps {
                    clock += Duration::minutes(minutes);
                    match step {
                        0 | 1 => {
                            let reset = (step == 0).then(|| clock + Duration::minutes(minutes));
                            if matches!(observe(&db.lock().unwrap(), "chat", LimitKind::Usage, "usage limit", reset, clock).unwrap(), Decision::Schedule { .. }) {
                                scheduled += 1;
                            }
                        }
                        2 => { cancel_for_user_input(&db.lock().unwrap(), "chat", clock).unwrap(); }
                        3 => { make_due_now(&db.lock().unwrap(), "chat", clock).unwrap(); }
                        _ => {
                            let report = tick(&db, &actions, clock).unwrap();
                            sends += report.acted.len();
                            // A switch the chat now runs on, as the live action would leave it.
                            for call in actions.calls.lock().unwrap().iter().filter(|call| call.starts_with("switch")) {
                                let _ = call;
                                db.lock().unwrap().execute("UPDATE sessions SET harness='codex',model='gpt' WHERE id='chat'", []).unwrap();
                            }
                        }
                    }
                }
                prop_assert!(sends <= scheduled, "sent {sends} for {scheduled} scheduled");
                if mode == LimitRecoveryMode::Off {
                    prop_assert_eq!(scheduled, 0);
                    prop_assert!(actions.calls.lock().unwrap().is_empty());
                }
                let calls = actions.calls.lock().unwrap();
                prop_assert_eq!(calls.iter().filter(|call| call.starts_with("send")).count(), sends);
            }
        }
    }
}
