//! The live part of the eval. It runs real Claude or Codex chats through the
//! seams that the app uses: `chat_defaults::create`, `live_turn::submit_input`
//! and the maintenance loop of `start_queued_input_maintenance`.
//!
//! Each arm has its own core, data directory and maintenance loop. The two
//! arms of one harness run at the same time, so that they see the same
//! provider conditions. The harnesses run one after the other.

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use bridge_protocol::messages::{
    ChatAccess, ChatDefaults, ChatDefaultsRevision, ChatLaunchTarget, ChatModelChoice, CreateConfiguredChatParams,
    InputDisposition, SaveEarlyCompactionSettingsParams,
};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};

use crate::adapters::{AdapterRegistry, ShutdownReason};
use crate::early_compaction::{IDLE_DELAY, SETTLE_LIMIT, WARM_LIMIT};
use crate::live_turn::{self, QUEUED_INPUT_SWEEP_INTERVAL};
use crate::runtime::BridgeCore;
use crate::usage_pricing::Pricing;

use super::report::{Arm, ArmRun, AttemptRow, LedgerRow, WaitEnd};
use super::workload::{score_turn, Phase, Rubric, Turn, TurnScore, Workload};

/// The default model of each harness.
pub(super) const DEFAULTS: [(&str, &str); 2] = [("claude", "claude-haiku-5-5"), ("codex", "gpt-6.2-sol")];
/// The gate of the live eval, and the prefix of its other values.
pub(super) const ENV_PREFIX: &str = "BRIDGE_EARLY_COMPACTION_EVAL";
/// A scripted turn that runs longer than this fails the arm.
const TURN_LIMIT: Duration = Duration::from_secs(600);
/// The check command of a recall turn that runs longer than this fails.
const CHECK_LIMIT: Duration = Duration::from_secs(60);
/// Adapter discovery runs in the background. The other live evals give it
/// this much time.
const DISCOVERY: Duration = Duration::from_secs(8);
/// The chat create tries again for this long while discovery runs.
const CREATE_LIMIT: Duration = Duration::from_secs(40);
const POLL: Duration = Duration::from_secs(1);
/// The check output that the report keeps, in characters.
const OUTPUT_TAIL: usize = 300;
/// A chat in one of these statuses has no turn and no compaction.
const IDLE_STATUSES: [&str; 2] = ["ready", "idle"];
/// A chat in one of these statuses cannot take the next turn.
const ENDED_STATUSES: [&str; 4] = ["stopped", "failed", "completed", "cancelled"];

/// `BRIDGE_EARLY_COMPACTION_EVAL_{name}`, or `None` when it is blank.
pub(super) fn env(name: &str) -> Option<String> {
    std::env::var(format!("{ENV_PREFIX}_{name}"))
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn text(error: impl std::fmt::Display) -> String {
    error.to_string()
}

fn db(core: &BridgeCore) -> MutexGuard<'_, Connection> {
    core.db.lock().unwrap_or_else(PoisonError::into_inner)
}

/// One harness and model, with the arms to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Target {
    pub harness: String,
    pub model: String,
    pub arms: Vec<Arm>,
}

/// The targets of a run, from the values of `_HARNESS`, `_MODEL` and
/// `_ONLY`. `_ONLY` names one harness or one arm. `_MODEL` needs one harness.
pub(super) fn plan(harness: Option<&str>, model: Option<&str>, only: Option<&str>) -> Result<Vec<Target>, String> {
    let known = |name: &str| DEFAULTS.iter().any(|(id, _)| *id == name);
    if let Some(name) = harness.filter(|name| !known(name)) {
        return Err(format!("{ENV_PREFIX}_HARNESS must be claude or codex, not {name}"));
    }
    let (only_harness, arms) = match only {
        None => (None, vec![Arm::On, Arm::Off]),
        Some("on") => (None, vec![Arm::On]),
        Some("off") => (None, vec![Arm::Off]),
        Some(name) if known(name) => (Some(name), vec![Arm::On, Arm::Off]),
        Some(other) => return Err(format!("{ENV_PREFIX}_ONLY must be claude, codex, on or off, not {other}")),
    };
    let targets: Vec<Target> = DEFAULTS
        .iter()
        .filter(|(id, _)| harness.is_none_or(|name| name == *id) && only_harness.is_none_or(|name| name == *id))
        .map(|(id, default)| Target {
            harness: (*id).to_owned(),
            model: model.unwrap_or(default).to_owned(),
            arms: arms.clone(),
        })
        .collect();
    if targets.is_empty() {
        return Err(format!("{ENV_PREFIX}_HARNESS and {ENV_PREFIX}_ONLY name two different harnesses"));
    }
    if model.is_some() && targets.len() > 1 {
        return Err(format!("set {ENV_PREFIX}_HARNESS when you set {ENV_PREFIX}_MODEL"));
    }
    Ok(targets)
}

/// What the wait after a turn sees at one poll.
#[derive(Debug, Clone, Copy)]
pub(super) struct Watch {
    /// The time since the turn end.
    pub since_end: Duration,
    /// The chat has a due mark.
    pub mark: bool,
    /// The chat has an open attempt now.
    pub open: bool,
    /// The time since the wait first saw an open attempt.
    pub since_open: Option<Duration>,
    /// The chat is idle: status `ready` or `idle`, and no active turn.
    pub idle: bool,
}

/// The wait rule after each turn, the same in both arms. The first sweep
/// that can decide comes after `IDLE_DELAY` and one sweep interval, so the
/// wait never ends before that time plus one more interval. Then it ends
/// when the due mark has a final decision and the chat is idle. A mark that
/// stays ends the wait at `WARM_LIMIT`. An attempt that opens gets up to
/// `SETTLE_LIMIT`, so that the next send does not stop the compaction.
pub(super) fn wait_step(watch: &Watch) -> Option<WaitEnd> {
    let margin = QUEUED_INPUT_SWEEP_INTERVAL * 2;
    if watch.since_end < IDLE_DELAY + margin {
        return None;
    }
    if !watch.mark && !watch.open && watch.idle {
        return Some(WaitEnd::Decided);
    }
    if let Some(since_open) = watch.since_open {
        return (since_open >= SETTLE_LIMIT + margin).then_some(WaitEnd::SettleLimit);
    }
    (watch.since_end >= WARM_LIMIT + margin).then_some(WaitEnd::WarmLimit)
}

/// The arms of one target, at the same time.
pub(super) fn run_target(workload: &Workload, rubric: &Rubric, target: &Target) -> Vec<ArmRun> {
    std::thread::scope(|scope| {
        let handles: Vec<_> = target
            .arms
            .iter()
            .map(|arm| (*arm, scope.spawn(move || run_arm(workload, rubric, target, *arm))))
            .collect();
        handles
            .into_iter()
            .map(|(arm, handle)| {
                handle.join().unwrap_or_else(|_| {
                    let mut run = ArmRun::new(arm, workload.turns.len(), workload.max_points());
                    run.failure = Some("the thread of the arm stopped with a panic".into());
                    run
                })
            })
            .collect()
    })
}

fn run_arm(workload: &Workload, rubric: &Rubric, target: &Target, arm: Arm) -> ArmRun {
    let mut run = ArmRun::new(arm, workload.turns.len(), workload.max_points());
    if let Err(error) = drive(workload, rubric, target, &mut run) {
        run.failure = Some(error);
    }
    run
}

fn drive(workload: &Workload, rubric: &Rubric, target: &Target, run: &mut ArmRun) -> Result<(), String> {
    // The directory stays after the run, so that a person can read the chat files.
    let data = tempfile::Builder::new()
        .prefix("bridge-early-compaction-")
        .tempdir()
        .map_err(text)?
        .keep();
    run.data_dir = Some(data.display().to_string());
    let core = start_core(&data, run.arm)?;
    let session_id = create_chat(&core, target)?;
    let cwd = session_cwd(&db(&core), &session_id)?;
    workload.write_fixture(&cwd)?;
    commit_fixture(&cwd)?;
    let played = play(&core, &session_id, &cwd, workload, rubric, run, &target.harness);
    // The figures come after the last turn and before the stop. A stop
    // through `stop_session` runs a checkpoint turn with its own ledger rows.
    let collected = collect(&db(&core), &session_id, &target.model, run);
    release(&core, &session_id);
    played.and(collected)
}

fn start_core(data: &Path, arm: Arm) -> Result<Arc<BridgeCore>, String> {
    let registry = Arc::new(AdapterRegistry::built_in().map_err(text)?);
    std::thread::sleep(DISCOVERY);
    let mut core = BridgeCore::for_tests(data);
    core.adapter_registry = registry;
    let core = Arc::new(core);
    if arm == Arm::On {
        let params = SaveEarlyCompactionSettingsParams { session_id: None, enabled: Some(true) };
        crate::early_compaction_settings::save(&core, &params).map_err(text)?;
    }
    live_turn::start_queued_input_maintenance(Arc::clone(&core));
    Ok(core)
}

fn create_chat(core: &Arc<BridgeCore>, target: &Target) -> Result<String, String> {
    let params = CreateConfiguredChatParams {
        workspace_id: None,
        overrides: ChatDefaults {
            target: Some(ChatLaunchTarget::Direct),
            model: Some(ChatModelChoice {
                harness: target.harness.clone(),
                model: Some(target.model.clone()),
                account: None,
            }),
            access: Some(ChatAccess::Full),
            ..ChatDefaults::default()
        },
        expected_revision: ChatDefaultsRevision::default(),
        title: Some(format!("Early compaction eval, {}", target.harness)),
        tool_selection: None,
    };
    let deadline = Instant::now() + CREATE_LIMIT;
    loop {
        match crate::chat_defaults::create(core, &params, None) {
            Ok(created) => return Ok(created.session_id),
            Err(_) if Instant::now() < deadline => std::thread::sleep(Duration::from_secs(5)),
            Err(error) => return Err(format!("the chat did not start: {error}")),
        }
    }
}

fn session_cwd(db: &Connection, session_id: &str) -> Result<PathBuf, String> {
    let cwd: Option<String> = db
        .query_row("SELECT cwd FROM sessions WHERE id=?1", params![session_id], |row| row.get(0))
        .map_err(text)?;
    let cwd = PathBuf::from(cwd.ok_or("the chat has no directory")?);
    std::fs::create_dir_all(&cwd).map_err(text)?;
    Ok(cwd)
}

/// A Git repository with one commit, so that each harness sees a normal
/// project.
fn commit_fixture(cwd: &Path) -> Result<(), String> {
    let steps: [&[&str]; 3] = [
        &["init", "--quiet"],
        &["add", "--all"],
        &[
            "-c",
            "user.name=Bridge eval",
            "-c",
            "user.email=eval@bridge.invalid",
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "--quiet",
            "--no-gpg-sign",
            "-m",
            "fixture",
        ],
    ];
    for args in steps {
        let status = crate::git::git_command(cwd).args(args).status().map_err(text)?;
        if !status.success() {
            return Err(format!("git {} failed", args.join(" ")));
        }
    }
    Ok(())
}

fn play(
    core: &Arc<BridgeCore>,
    session_id: &str,
    cwd: &Path,
    workload: &Workload,
    rubric: &Rubric,
    run: &mut ArmRun,
    harness: &str,
) -> Result<(), String> {
    for (index, turn) in workload.turns.iter().enumerate() {
        let number = index + 1;
        let baseline = max_sequence(&db(core), session_id)?;
        run.submissions.push(Utc::now());
        let sent = live_turn::submit_input(core, session_id.to_owned(), turn.text.clone())
            .map_err(|error| format!("turn {number}: {error}"))?;
        // A send into a busy chat gives a turn end that is not the end of this
        // turn, so the eval cannot read the turn.
        if sent.disposition != InputDisposition::StartedNewTurn {
            return Err(format!("turn {number}: the send was {}, not a new turn", sent.disposition.as_str()));
        }
        wait_turn(core, session_id, baseline).map_err(|error| format!("turn {number}: {error}"))?;
        let ended = Instant::now();
        run.turns_finished = number;
        if turn.phase == Phase::Recall {
            run.scores.push(score(cwd, rubric, number, turn));
        }
        let wait = wait_rule(core, session_id, ended)?;
        let window = latest_reading(&db(core), session_id)?;
        run.waits.push(wait);
        run.readings.push(window);
        println!("[{harness} {}] turn {number}: window {window:?}, wait {wait:?}", run.arm.as_str());
    }
    Ok(())
}

fn max_sequence(db: &Connection, session_id: &str) -> Result<i64, String> {
    db.query_row(
        "SELECT COALESCE(MAX(sequence),0) FROM session_entries WHERE session_id=?1",
        params![session_id],
        |row| row.get(0),
    )
    .map_err(text)
}

/// The kind and the status of the first turn end after `baseline`.
fn turn_end(db: &Connection, session_id: &str, baseline: i64) -> Result<Option<(String, Option<String>)>, String> {
    db.query_row(
        "SELECT kind,json_extract(payload,'$.status') FROM session_entries
         WHERE session_id=?1 AND sequence>?2 AND kind IN ('turn.completed','turn.failed')
         ORDER BY sequence LIMIT 1",
        params![session_id, baseline],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .optional()
    .map_err(text)
}

/// The status of the chat, and whether it has no active turn.
fn session_state(db: &Connection, session_id: &str) -> Result<(String, bool), String> {
    db.query_row(
        "SELECT status,active_turn_id IS NULL FROM sessions WHERE id=?1",
        params![session_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .map_err(text)
}

fn failed(kind: &str, status: Option<&str>) -> bool {
    kind == "turn.failed"
        || status.is_some_and(|status| {
            matches!(status, "failed" | "interrupted" | "cancelled") || status.starts_with("error")
        })
}

fn wait_turn(core: &BridgeCore, session_id: &str, baseline: i64) -> Result<(), String> {
    let deadline = Instant::now() + TURN_LIMIT;
    loop {
        let end = turn_end(&db(core), session_id, baseline)?;
        if let Some((kind, status)) = end {
            if failed(&kind, status.as_deref()) {
                return Err(format!("the turn ended as {kind}, status {}", status.unwrap_or_default()));
            }
            return Ok(());
        }
        let (status, _) = session_state(&db(core), session_id)?;
        if ENDED_STATUSES.contains(&status.as_str()) {
            return Err(format!("the chat ended with status {status}"));
        }
        if Instant::now() >= deadline {
            return Err(format!("the turn did not end in {} s", TURN_LIMIT.as_secs()));
        }
        std::thread::sleep(POLL);
    }
}

/// The wait after one turn. A `Decided` poll must hold two times in a row:
/// the sweep drops the mark a moment before it opens the attempt.
fn wait_rule(core: &BridgeCore, session_id: &str, ended: Instant) -> Result<WaitEnd, String> {
    let mut seen_open: Option<Instant> = None;
    let mut calm = false;
    loop {
        let mark = core.early_compaction.due_mark(session_id).is_some();
        let open = core.early_compaction.open_attempt(session_id).is_some() || open_row(&db(core), session_id)?;
        if open && seen_open.is_none() {
            seen_open = Some(Instant::now());
        }
        let (status, no_turn) = session_state(&db(core), session_id)?;
        let watch = Watch {
            since_end: ended.elapsed(),
            mark,
            open,
            since_open: seen_open.map(|at| at.elapsed()),
            idle: no_turn && IDLE_STATUSES.contains(&status.as_str()),
        };
        match wait_step(&watch) {
            Some(WaitEnd::Decided) if !calm => calm = true,
            Some(end) => return Ok(end),
            None => calm = false,
        }
        std::thread::sleep(POLL);
    }
}

/// The sweep writes the attempt row before the tracker holds the attempt.
fn open_row(db: &Connection, session_id: &str) -> Result<bool, String> {
    db.query_row(
        "SELECT EXISTS(SELECT 1 FROM early_compaction_attempts WHERE session_id=?1 AND outcome IS NULL)",
        params![session_id],
        |row| row.get(0),
    )
    .map_err(text)
}

fn latest_reading(db: &Connection, session_id: &str) -> Result<Option<i64>, String> {
    db.query_row(
        "SELECT used_tokens FROM context_readings WHERE session_id=?1 ORDER BY id DESC LIMIT 1",
        params![session_id],
        |row| row.get(0),
    )
    .optional()
    .map_err(text)
}

/// R1 and R2 of one recall turn, from the files as the turn left them.
fn score(cwd: &Path, rubric: &Rubric, number: usize, turn: &Turn) -> TurnScore {
    let command = turn.check.as_deref().unwrap_or_default();
    let (passed, output) = run_check(cwd, command).unwrap_or_else(|error| (false, error));
    let file = turn.file.as_ref().and_then(|file| std::fs::read_to_string(cwd.join(file)).ok());
    score_turn(number, passed, rubric.keeps(file.as_deref()), output)
}

/// Run `command` in `cwd`. True when it exits with 0. The output goes to a
/// file, so a large output cannot block the child.
fn run_check(cwd: &Path, command: &[String]) -> Result<(bool, String), String> {
    let (program, args) = command.split_first().ok_or("the turn has no check command")?;
    let mut log = tempfile::tempfile().map_err(text)?;
    let mut child = Command::new(program)
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(log.try_clone().map_err(text)?)
        .stderr(log.try_clone().map_err(text)?)
        .spawn()
        .map_err(text)?;
    let deadline = Instant::now() + CHECK_LIMIT;
    let passed = loop {
        if let Some(status) = child.try_wait().map_err(text)? {
            break status.success();
        }
        if Instant::now() >= deadline {
            child.kill().map_err(text)?;
            let _reaped = child.wait().map_err(text)?;
            return Ok((false, format!("the check did not end in {} s", CHECK_LIMIT.as_secs())));
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    let mut bytes = Vec::new();
    log.seek(SeekFrom::Start(0)).map_err(text)?;
    log.read_to_end(&mut bytes).map_err(text)?;
    Ok((passed, tail(&String::from_utf8_lossy(&bytes))))
}

fn tail(output: &str) -> String {
    let trimmed = output.trim();
    let skip = trimmed.chars().count().saturating_sub(OUTPUT_TAIL);
    trimmed.chars().skip(skip).collect()
}

fn time(value: Option<String>) -> Option<DateTime<Utc>> {
    value
        .and_then(|value| DateTime::parse_from_rfc3339(&value).ok())
        .map(|value| value.with_timezone(&Utc))
}

fn collect(db: &Connection, session_id: &str, model: &str, run: &mut ArmRun) -> Result<(), String> {
    run.rows = ledger_rows(db, session_id)?;
    run.attempts = attempt_rows(db, session_id)?;
    run.rate = Pricing::load(db).map_err(text)?.lookup_rate(model);
    Ok(())
}

/// Each provider row of the chat, the compaction turn included.
fn ledger_rows(db: &Connection, session_id: &str) -> Result<Vec<LedgerRow>, String> {
    let mut statement = db
        .prepare(
            "SELECT created_at,COALESCE(uncached_input_tokens,0),COALESCE(cache_write_tokens,0),
                    COALESCE(cache_read_tokens,0),COALESCE(output_tokens,0)
             FROM usage_ledger WHERE session_id=?1 AND source LIKE 'provider.%' ORDER BY id",
        )
        .map_err(text)?;
    let rows = statement
        .query_map(params![session_id], |row| {
            Ok(LedgerRow {
                created_at: time(row.get(0)?),
                uncached_input: row.get(1)?,
                cache_write: row.get(2)?,
                cache_read: row.get(3)?,
                output: row.get(4)?,
            })
        })
        .map_err(text)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(text)
}

fn attempt_rows(db: &Connection, session_id: &str) -> Result<Vec<AttemptRow>, String> {
    let mut statement = db
        .prepare(
            "SELECT outcome,created_at,dispatched_at,settled_at,trigger_reading_id,trigger_used_tokens,
                    boundary_pre_tokens,boundary_post_tokens,next_reading_id,next_used_tokens,error
             FROM early_compaction_attempts WHERE session_id=?1 ORDER BY created_at,rowid",
        )
        .map_err(text)?;
    let rows = statement
        .query_map(params![session_id], |row| {
            Ok(AttemptRow {
                outcome: row.get(0)?,
                created_at: time(row.get(1)?),
                dispatched_at: time(row.get(2)?),
                settled_at: time(row.get(3)?),
                trigger_reading_id: row.get(4)?,
                trigger_used_tokens: row.get(5)?,
                boundary_pre_tokens: row.get(6)?,
                boundary_post_tokens: row.get(7)?,
                next_reading_id: row.get(8)?,
                next_used_tokens: row.get(9)?,
                error: row.get(10)?,
            })
        })
        .map_err(text)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(text)
}

/// Stop the harness process with no Bridge checkpoint, as the idle release
/// does.
fn release(core: &BridgeCore, session_id: &str) {
    let runtime = core.adapters.lock().unwrap_or_else(PoisonError::into_inner).remove(session_id);
    if let Some(mut runtime) = runtime {
        runtime.stop(ShutdownReason::Completed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arms(targets: &[Target]) -> Vec<(String, String, Vec<Arm>)> {
        targets
            .iter()
            .map(|target| (target.harness.clone(), target.model.clone(), target.arms.clone()))
            .collect()
    }

    #[test]
    fn the_plan_runs_both_harnesses_and_both_arms_by_default() -> Result<(), String> {
        let both = arms(&plan(None, None, None)?);
        let expected = vec![
            ("claude".to_owned(), "claude-haiku-5-5".to_owned(), vec![Arm::On, Arm::Off]),
            ("codex".to_owned(), "gpt-6.2-sol".to_owned(), vec![Arm::On, Arm::Off]),
        ];
        if both != expected {
            return Err(format!("{both:?}"));
        }
        Ok(())
    }

    #[test]
    fn only_names_one_harness_or_one_arm_and_the_model_needs_one_harness() -> Result<(), String> {
        let cases = [
            (plan(Some("codex"), Some("gpt-6.1-terra"), None)?, vec![("codex", "gpt-6.1-terra", vec![Arm::On, Arm::Off])]),
            (plan(None, None, Some("claude"))?, vec![("claude", "claude-haiku-5-5", vec![Arm::On, Arm::Off])]),
            (plan(Some("claude"), None, Some("off"))?, vec![("claude", "claude-haiku-5-5", vec![Arm::Off])]),
        ];
        for (targets, expected) in cases {
            let expected: Vec<(String, String, Vec<Arm>)> = expected
                .into_iter()
                .map(|(harness, model, arms)| (harness.to_owned(), model.to_owned(), arms))
                .collect();
            if arms(&targets) != expected {
                return Err(format!("{targets:?}"));
            }
        }
        for refused in [
            plan(None, Some("claude-sonnet-5"), None),
            plan(Some("opencode"), None, None),
            plan(None, None, Some("both")),
            plan(Some("claude"), None, Some("codex")),
        ] {
            if refused.is_ok() {
                return Err(format!("{refused:?} must be refused"));
            }
        }
        Ok(())
    }

    fn watch(since_end: u64, mark: bool, open: bool, since_open: Option<u64>, idle: bool) -> Watch {
        Watch {
            since_end: Duration::from_secs(since_end),
            mark,
            open,
            since_open: since_open.map(Duration::from_secs),
            idle,
        }
    }

    #[test]
    fn the_wait_ends_after_the_first_deciding_sweep_and_at_the_shipped_limits() {
        let floor = (IDLE_DELAY + QUEUED_INPUT_SWEEP_INTERVAL * 2).as_secs();
        let warm = (WARM_LIMIT + QUEUED_INPUT_SWEEP_INTERVAL * 2).as_secs();
        let settle = (SETTLE_LIMIT + QUEUED_INPUT_SWEEP_INTERVAL * 2).as_secs();
        // Before the first sweep that can decide, the wait goes on.
        assert_eq!(wait_step(&watch(floor - 1, false, false, None, true)), None);
        assert_eq!(wait_step(&watch(floor, false, false, None, true)), Some(WaitEnd::Decided));
        // A kept mark or a busy chat waits until the warm limit.
        assert_eq!(wait_step(&watch(floor, true, false, None, true)), None);
        assert_eq!(wait_step(&watch(floor, false, false, None, false)), None);
        assert_eq!(wait_step(&watch(warm, true, false, None, true)), Some(WaitEnd::WarmLimit));
        // An open attempt waits past the warm limit, until it settles.
        assert_eq!(wait_step(&watch(warm, false, true, Some(80), false)), None);
        assert_eq!(wait_step(&watch(warm + 60, false, false, Some(140), false)), None);
        assert_eq!(wait_step(&watch(warm + 60, false, false, Some(140), true)), Some(WaitEnd::Decided));
        assert_eq!(wait_step(&watch(settle + 30, false, true, Some(settle), false)), Some(WaitEnd::SettleLimit));
    }

    #[test]
    fn a_failed_turn_end_and_a_long_check_output_are_read_as_such() {
        assert!(failed("turn.failed", None));
        assert!(failed("turn.completed", Some("error_max_turns")));
        assert!(failed("turn.completed", Some("interrupted")));
        assert!(!failed("turn.completed", Some("completed")));
        assert!(!failed("turn.completed", None));
        let long = format!("start {}", "x".repeat(OUTPUT_TAIL));
        assert_eq!(tail(&long).chars().count(), OUTPUT_TAIL);
        assert_eq!(tail("  ok\n"), "ok");
    }
}
