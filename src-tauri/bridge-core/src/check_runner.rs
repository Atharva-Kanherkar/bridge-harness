//! The executor behind `bridge.shell` completion checks.
//!
//! Contracts plan deterministic checks with executor `bridge.shell`, but nothing
//! used to run them: `record_check` is a passive recording endpoint, so an
//! attempt sat at "Verifying 0/7" indefinitely. The only thing that ever
//! satisfied a shell check was a verification worker's self-reported `tests[]`,
//! recorded under `bridge.worker_result` — a digest of the JSON that was
//! received, which proves nothing about whether the command ran.
//!
//! This module closes both halves:
//!
//! * [`pending_shell_checks`], [`claim`], and [`run_claimed_check`] execute the
//!   planned command inside the attempt's exact `repository_path`, capture
//!   bounded stdout/stderr and the exit status, digest the real output, and
//!   record the verdict.
//! * [`escalate_stalled_attempts`] gives `verifying` a deadline, so an attempt
//!   with a check nothing can run fails terminally and explains which checks
//!   never ran instead of blocking the parent forever.
//!
//! Commands are allowlisted by program *and verb* — the program alone is not a
//! boundary, since `git` also spells `reset --hard` and `npm` also spells
//! `install` — and rejected outright if they carry shell metacharacters or point
//! a path flag outside the checkout. A planned check is a repository command,
//! never a shell script, and it is executed without a shell.

use crate::{
    completion::{self, CheckRun, CheckStatus, EvalKind},
    store, BridgeError, COMPLETION_VERIFY_TIMEOUT_SECONDS,
};
use chrono::Utc;
use rusqlite::{params, Connection};
use sha2::{Digest, Sha256};
use std::{
    io::Read,
    path::Path,
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

/// What each allowlisted program may be asked to do.
///
/// The program alone is not a safety boundary: `git` also spells `reset --hard`,
/// `clean -fdx`, and `push`; `npm` also spells `install` and `publish`. A
/// verification check inspects and builds — it does not mutate history, install
/// dependencies, or reach a registry — so the *verb* is allowlisted too. An empty
/// verb list means the program takes a script or file rather than a subcommand.
///
/// `npx`, `npm exec`, and the `dlx` verbs are absent throughout: they download and
/// execute arbitrary registry code, and a planned command is LLM-authored.
const ALLOWED_SUBCOMMANDS: &[(&str, &[&str])] = &[
    ("bun", &["build", "check", "run", "test", "tsc", "vitest"]),
    (
        "cargo",
        &["bench", "build", "check", "clippy", "fmt", "test"],
    ),
    (
        "git",
        &[
            "branch",
            "diff",
            "log",
            "ls-files",
            "merge-base",
            "rev-list",
            "rev-parse",
            "show",
            "status",
        ],
    ),
    ("node", &[]),
    ("npm", &["run", "run-script", "test"]),
    ("pnpm", &["build", "check", "run", "test"]),
    ("python3", &[]),
    ("tsc", &[]),
    ("vitest", &[]),
    ("yarn", &["build", "check", "run", "test"]),
];

/// Programs a planned check is allowed to invoke.
pub const ALLOWED_PROGRAMS: &[&str] = &[
    "bun", "cargo", "git", "node", "npm", "pnpm", "python3", "tsc", "vitest", "yarn",
];

/// `bun x`, `npm exec`, `pnpm dlx`, `yarn dlx` — fetch-and-run.
const REMOTE_EXECUTION_SUBCOMMANDS: &[&str] = &["dlx", "exec", "x"];

/// Flags whose value names a directory or project root. Their *value* is what
/// matters: `--manifest-path src-tauri/Cargo.toml` stays inside the checkout and
/// is exactly what the planner emits, while `--manifest-path /elsewhere` or
/// `../other` would verify a different tree than the one the proof names.
const PATH_VALUED_FLAGS: &[&str] = &[
    "-C",
    "--cwd",
    "--directory",
    "--exec-path",
    "--git-dir",
    "--manifest-path",
    "--prefix",
    "--project",
    "--work-tree",
];

/// Characters that only make sense to a shell. A planned check is executed
/// directly, so their presence means the command was never going to work — and
/// accepting them would turn an allowlist into a suggestion.
const SHELL_METACHARACTERS: &[char] = &[
    ';', '&', '|', '>', '<', '`', '$', '(', ')', '{', '}', '\n', '\r', '*', '?', '!', '\\',
];

/// Output kept per stream. Enough to diagnose a failure; bounded so a runaway
/// build cannot fill the database.
const MAX_STREAM_BYTES: usize = 64 * 1024;

/// How long one planned command may run before its process group is killed and
/// the check is recorded failed.
pub const CHECK_TIMEOUT_SECONDS: u64 = 20 * 60;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellCommand {
    pub program: String,
    pub arguments: Vec<String>,
}

/// Is this a repository-relative path that stays inside the checkout?
fn stays_in_repository(value: &str) -> bool {
    let value = value.trim();
    if value.is_empty() || value.starts_with('/') || value.starts_with('~') {
        return false;
    }
    // Windows-style absolute (`C:\...`) and UNC paths are equally out of bounds.
    if value.len() >= 2 && value.as_bytes()[1] == b':' {
        return false;
    }
    !value.split(['/', '\\']).any(|segment| segment == "..")
}

/// Parse and authorize a planned command. Returns why it was refused rather than
/// a bare `None`, so a blocked check can explain itself.
pub fn authorize(command: &str) -> Result<ShellCommand, String> {
    let command = command.trim();
    if command.is_empty() {
        return Err("the planned check has an empty command".into());
    }
    if let Some(character) = command.chars().find(|c| SHELL_METACHARACTERS.contains(c)) {
        return Err(format!(
            "the planned command contains the shell metacharacter {character:?}; Bridge runs checks directly, not through a shell"
        ));
    }
    if command.contains('\'') || command.contains('"') {
        return Err("the planned command contains quotes; pass arguments as plain tokens".into());
    }
    let mut tokens = command.split_whitespace();
    let program = tokens.next().unwrap_or_default().to_owned();
    let Some((_, verbs)) = ALLOWED_SUBCOMMANDS
        .iter()
        .find(|(name, _)| *name == program.as_str())
    else {
        return Err(format!(
            "{program} is not an allowlisted check program ({})",
            ALLOWED_PROGRAMS.join(", ")
        ));
    };
    let arguments = tokens.map(str::to_owned).collect::<Vec<_>>();
    // Pass 1: every path-valued flag must stay inside the checkout, in both the
    // `--flag value` and `--flag=value` forms. This is a separate, exhaustive pass
    // rather than part of verb detection: verb detection stops at the verb, so a
    // flag written after it would never be examined.
    let mut expect_path: Option<&str> = None;
    for argument in &arguments {
        if let Some(flag) = expect_path.take() {
            if !stays_in_repository(argument) {
                return Err(format!(
                    "{flag} points at {argument}, outside the repository being verified; a check must run against the attempt's own checkout"
                ));
            }
            continue;
        }
        let Some(flag) = PATH_VALUED_FLAGS
            .iter()
            .find(|flag| argument == *flag || argument.starts_with(&format!("{flag}=")))
        else {
            continue;
        };
        match argument.strip_prefix(&format!("{flag}=")) {
            Some(value) if !stays_in_repository(value) => {
                return Err(format!(
                    "{flag} points at {value}, outside the repository being verified; a check must run against the attempt's own checkout"
                ))
            }
            Some(_) => {}
            None => expect_path = Some(flag),
        }
    }
    if let Some(flag) = expect_path {
        return Err(format!("the planned command ends with {flag} and no value"));
    }
    // Pass 2: the first non-flag token is the verb. Flags may precede it (`cargo
    // --locked test`), and a separated flag's value must not be mistaken for it.
    let mut verb: Option<&str> = None;
    let mut skip_value = false;
    for argument in &arguments {
        if skip_value {
            skip_value = false;
            continue;
        }
        if PATH_VALUED_FLAGS.iter().any(|flag| argument == *flag) {
            skip_value = true;
            continue;
        }
        if argument.starts_with('-') {
            continue;
        }
        verb = Some(argument);
        break;
    }
    if let Some(verb) = verb {
        if REMOTE_EXECUTION_SUBCOMMANDS.contains(&verb) {
            return Err(format!(
                "{program} {verb} downloads and runs code from a registry, which cannot be a verification check"
            ));
        }
        if !verbs.is_empty() && !verbs.contains(&verb) {
            return Err(format!(
                "{program} {verb} is not a verification action; {program} checks may only {}",
                verbs.join(", ")
            ));
        }
    } else if !verbs.is_empty() {
        return Err(format!(
            "the planned {program} command names no subcommand; expected one of {}",
            verbs.join(", ")
        ));
    }
    Ok(ShellCommand { program, arguments })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutcome {
    pub status: CheckStatus,
    pub detail: String,
    pub output_digest: String,
}

/// Execute one authorized command in `worktree` and turn it into a verdict. The
/// digest covers the real exit status and captured output, so it is evidence the
/// command ran rather than evidence a message was received.
pub fn execute(worktree: &Path, command: &ShellCommand) -> CommandOutcome {
    let started = Instant::now();
    let mut builder = Command::new(&command.program);
    builder
        .args(&command.arguments)
        .current_dir(worktree)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // Own the process group so a timeout can kill the whole tree. `cargo test`
    // and `bun run` both spawn children; killing only the direct child leaves a
    // grandchild holding the pipes open, and the drain threads would never end.
    // Verification runs the real build (`cargo test`, `bun run check`), so this
    // is where a shared cache pays off most: a check in a fresh worker worktree
    // reuses the repository's artifacts instead of compiling the world.
    crate::build_cache::apply(&mut builder, worktree);
    crate::adapters::configure_process_group(&mut builder);
    let spawned = builder.spawn();
    let mut child = match spawned {
        Ok(child) => child,
        Err(error) => {
            let detail = format!("could not start {}: {error}", command.program);
            return CommandOutcome {
                status: CheckStatus::Blocked,
                output_digest: digest(&detail),
                detail,
            };
        }
    };
    // Both streams must be drained *concurrently* with the wait. A build that
    // writes more than the OS pipe buffer (tens of KB — `cargo test` and
    // `bun run build` both exceed it easily) blocks in `write()` until someone
    // reads, so waiting first and reading after would hang every real check
    // until the timeout killed it and recorded a passing command as failed.
    let stdout = child.stdout.take().map(drain_on_thread);
    let stderr = child.stderr.take().map(drain_on_thread);
    let deadline = Duration::from_secs(CHECK_TIMEOUT_SECONDS);
    let exit = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if started.elapsed() >= deadline => {
                // Terminate the group, not just the leader, so no descendant
                // survives holding stdout/stderr open.
                if !crate::adapters::terminate_process_group(child.id()) {
                    let _ = child.kill();
                }
                let _ = child.wait();
                break None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(error) => {
                let detail = format!("could not wait for {}: {error}", command.program);
                return CommandOutcome {
                    status: CheckStatus::Blocked,
                    output_digest: digest(&detail),
                    detail,
                };
            }
        }
    };
    let out = stdout.map(join_drain).unwrap_or_default();
    let err = stderr.map(join_drain).unwrap_or_default();
    let elapsed = started.elapsed().as_secs();
    let Some(exit) = exit else {
        let detail = format!(
            "timed out after {CHECK_TIMEOUT_SECONDS}s and was killed\n--- stdout ---\n{out}\n--- stderr ---\n{err}"
        );
        return CommandOutcome {
            status: CheckStatus::Failed,
            output_digest: digest(&detail),
            detail,
        };
    };
    let code = exit.code();
    let detail = format!(
        "exit {} after {elapsed}s\n--- stdout ---\n{out}\n--- stderr ---\n{err}",
        code.map_or_else(|| "signal".to_owned(), |code| code.to_string())
    );
    CommandOutcome {
        status: if exit.success() {
            CheckStatus::Passed
        } else {
            CheckStatus::Failed
        },
        output_digest: digest(&detail),
        detail,
    }
}

/// How long to wait for a drain thread after the child has exited. Bounded
/// because a leaked descendant can still hold the write end of the pipe, and the
/// check runner must not be frozen by one.
const DRAIN_JOIN_TIMEOUT: Duration = Duration::from_secs(10);

struct Drain {
    handle: thread::JoinHandle<()>,
    captured: Arc<Mutex<String>>,
}

/// Read one child stream to EOF on its own thread, so the child is never blocked
/// waiting for us to empty its pipe. The text is published through a mutex rather
/// than the thread's return value so it can be read even if the thread is still
/// blocked on a pipe a survivor holds open.
fn drain_on_thread<S: Read + Send + 'static>(mut stream: S) -> Drain {
    let captured = Arc::new(Mutex::new(String::new()));
    let sink = captured.clone();
    Drain {
        handle: thread::spawn(move || {
            let text = bounded_read(&mut stream);
            *sink.lock().unwrap() = text;
        }),
        captured,
    }
}

fn join_drain(drain: Drain) -> String {
    let deadline = Instant::now() + DRAIN_JOIN_TIMEOUT;
    while !drain.handle.is_finished() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(20));
    }
    if !drain.handle.is_finished() {
        // The thread is parked on a pipe an escaped descendant still owns. Take
        // what was captured and move on; the thread ends when that process does.
        let mut text = drain.captured.lock().unwrap().clone();
        text.push_str(
            "\n[Bridge stopped waiting for this stream: a descendant process kept it open]",
        );
        return text;
    }
    match drain.handle.join() {
        Ok(()) => drain.captured.lock().unwrap().clone(),
        Err(_) => "[Bridge could not capture this stream]".to_owned(),
    }
}

/// Keep the first [`MAX_STREAM_BYTES`] and *discard the rest*, but keep reading
/// to EOF either way: stopping at the cap would leave the child blocked on a full
/// pipe, which is the same hang the cap is meant to avoid.
fn bounded_read(stream: &mut impl Read) -> String {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 8192];
    let mut truncated = false;
    loop {
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(read) => {
                let room = MAX_STREAM_BYTES.saturating_sub(buffer.len());
                if room == 0 {
                    truncated = true;
                    continue;
                }
                buffer.extend_from_slice(&chunk[..read.min(room)]);
                truncated |= read > room;
            }
        }
    }
    let mut text = String::from_utf8_lossy(&buffer).into_owned();
    if truncated {
        text.push_str("\n[output truncated by Bridge]");
    }
    text
}

fn digest(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

/// One `bridge.shell` check waiting to be executed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingCheck {
    pub attempt_id: String,
    pub session_id: String,
    pub repository_path: String,
    pub check_id: String,
    pub required: bool,
    pub command: String,
}

/// Every pending `bridge.shell` check on a live attempt, oldest attempt first.
pub fn pending_shell_checks(db: &Connection) -> Result<Vec<PendingCheck>, BridgeError> {
    let mut statement = db.prepare(
        "SELECT a.id,a.session_id,a.repository_path,r.check_id,r.required,r.command
         FROM eval_check_runs r
         JOIN eval_attempts a ON a.id=r.attempt_id
         WHERE r.executor='bridge.shell' AND r.status='pending' AND r.command IS NOT NULL
           AND a.status IN ('verifying','changes_requested')
         ORDER BY a.started_at,r.rowid",
    )?;
    let rows = statement.query_map([], |row| {
        Ok(PendingCheck {
            attempt_id: row.get(0)?,
            session_id: row.get(1)?,
            repository_path: row.get(2)?,
            check_id: row.get(3)?,
            required: row.get(4)?,
            command: row.get(5)?,
        })
    })?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

/// Claim a pending check so a second pass cannot run the same command twice.
/// Returns false when another pass already claimed it.
pub fn claim(db: &Connection, check: &PendingCheck) -> Result<bool, BridgeError> {
    let claimed = db.execute(
        "UPDATE eval_check_runs SET status='running',started_at=?3 WHERE attempt_id=?1 AND check_id=?2 AND status='pending'",
        params![check.attempt_id, check.check_id, Utc::now().to_rfc3339()],
    )?;
    Ok(claimed == 1)
}

/// Run a claimed check to a verdict **without touching the database**.
///
/// Deliberately split from [`record_outcome`]: a planned command is a full build
/// or test suite, and holding the global SQLite lock across it would freeze every
/// other session for minutes.
pub fn run_claimed_check_offline(check: &PendingCheck) -> CommandOutcome {
    let worktree = Path::new(&check.repository_path);
    if !worktree.is_dir() {
        let detail = format!(
            "the attempt's repository path {} does not exist, so this check could not run",
            check.repository_path
        );
        return CommandOutcome {
            status: CheckStatus::Blocked,
            output_digest: digest(&detail),
            detail,
        };
    }
    match authorize(&check.command) {
        Ok(command) => execute(worktree, &command),
        Err(reason) => CommandOutcome {
            status: CheckStatus::Blocked,
            output_digest: digest(&reason),
            detail: reason,
        },
    }
}

/// Persist a verdict produced by [`run_claimed_check_offline`].
pub fn record_outcome(
    db: &Connection,
    check: &PendingCheck,
    outcome: &CommandOutcome,
) -> Result<(), BridgeError> {
    completion::record_check(
        db,
        &check.attempt_id,
        &CheckRun {
            check_id: check.check_id.clone(),
            kind: EvalKind::Deterministic,
            required: check.required,
            status: outcome.status,
            executor: completion::SHELL_EXECUTOR.into(),
            command: Some(check.command.clone()),
            verifier_family: None,
            detail: Some(outcome.detail.clone()),
            output_digest: Some(outcome.output_digest.clone()),
            artifact_refs: vec![],
        },
    )?;
    store::event(
        db,
        "completion",
        "completion.check_executed",
        &check.session_id,
        &serde_json::json!({
            "attemptId": check.attempt_id,
            "checkId": check.check_id,
            "command": check.command,
            "status": outcome.status.as_str(),
        })
        .to_string(),
    )?;
    Ok(())
}

/// Test/convenience seam: execute and record in one call. Production goes through
/// the split pair so the command never runs under the database lock.
pub fn run_claimed_check(
    db: &Connection,
    check: &PendingCheck,
) -> Result<CheckStatus, BridgeError> {
    let outcome = run_claimed_check_offline(check);
    record_outcome(db, check, &outcome)?;
    Ok(outcome.status)
}

/// A check that reached a terminal state and the attempt it belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutedCheck {
    pub attempt_id: String,
    pub session_id: String,
    pub check_id: String,
    pub status: CheckStatus,
}

/// Attempts whose planned checks have not settled within the verify deadline,
/// with the check ids that never reached a terminal state.
pub fn stalled_attempts(
    db: &Connection,
) -> Result<Vec<(String, String, Vec<String>)>, BridgeError> {
    let candidates = {
        let mut statement = db.prepare(
            "SELECT id,session_id,started_at FROM eval_attempts WHERE status IN ('verifying','changes_requested') ORDER BY started_at",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        rows.collect::<Result<Vec<_>, _>>()?
    };
    let now = Utc::now();
    let mut stalled = Vec::new();
    for (attempt_id, session_id, started_at) in candidates {
        let Some(age) = chrono::DateTime::parse_from_rfc3339(&started_at)
            .ok()
            .map(|started| {
                now.signed_duration_since(started.with_timezone(&Utc))
                    .num_seconds()
            })
        else {
            continue;
        };
        if age < COMPLETION_VERIFY_TIMEOUT_SECONDS {
            continue;
        }
        let unresolved = {
            let mut statement = db.prepare(
                "SELECT check_id FROM eval_check_runs WHERE attempt_id=?1 AND required=1 AND status IN ('pending','running') ORDER BY rowid",
            )?;
            let rows = statement.query_map(params![attempt_id], |row| row.get::<_, String>(0))?;
            rows.collect::<Result<Vec<_>, _>>()?
        };
        if !unresolved.is_empty() {
            stalled.push((attempt_id, session_id, unresolved));
        }
    }
    Ok(stalled)
}

/// Fail every attempt past the verify deadline, naming the checks that never
/// ran. Terminal failure is what unblocks the parent: an attempt stuck in
/// `verifying` keeps the session `waiting` forever.
pub fn escalate_stalled_attempts(db: &Connection) -> Result<Vec<String>, BridgeError> {
    let stalled = stalled_attempts(db)?;
    let mut escalated = Vec::new();
    for (attempt_id, session_id, unresolved) in stalled {
        let minutes = COMPLETION_VERIFY_TIMEOUT_SECONDS / 60;
        let reason = format!(
            "no executor produced a verdict for this check within {minutes} minutes. Unresolved required checks: {}",
            unresolved.join(", ")
        );
        for check_id in &unresolved {
            let (required, command, kind, executor): (bool, Option<String>, String, String) = db
                .query_row(
                    "SELECT required,command,kind,executor FROM eval_check_runs WHERE attempt_id=?1 AND check_id=?2",
                    params![attempt_id, check_id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                )?;
            let detail = if executor == completion::WORKER_EXECUTOR {
                format!("{reason} No eligible verifier claimed this semantic check.")
            } else {
                reason.clone()
            };
            completion::record_check(
                db,
                &attempt_id,
                &CheckRun {
                    check_id: check_id.clone(),
                    kind: match kind.as_str() {
                        "scrutiny" => EvalKind::Scrutiny,
                        "user_testing" => EvalKind::UserTesting,
                        _ => EvalKind::Deterministic,
                    },
                    required,
                    status: CheckStatus::Blocked,
                    executor: completion::SYSTEM_EXECUTOR.into(),
                    command,
                    verifier_family: None,
                    detail: Some(detail),
                    output_digest: Some(digest(&reason)),
                    artifact_refs: vec![],
                },
            )?;
        }
        db.execute(
            "UPDATE eval_attempts SET status='failed',escalation=?3,completed_at=?2 WHERE id=?1",
            params![
                attempt_id,
                Utc::now().to_rfc3339(),
                completion::VERIFY_DEADLINE_ESCALATION
            ],
        )?;
        store::event(
            db,
            "completion",
            "completion.verify_deadline_expired",
            &session_id,
            &reason,
        )?;
        completion::reconcile_parent_readiness(db, &session_id)?;
        escalated.push(attempt_id);
    }
    Ok(escalated)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::completion::{CompletionContract, EvalCheck, EvalPlan, RepositoryStamp, RiskTier};
    use std::process::Command as StdCommand;

    /// The planner's own commands must authorize. Rejecting `--manifest-path`
    /// outright meant every Rust check was blocked instead of producing evidence.
    #[test]
    fn every_command_the_planner_generates_authorizes() {
        for command in [
            "cargo test --manifest-path src-tauri/Cargo.toml --workspace",
            "cargo check --manifest-path src-tauri/Cargo.toml --workspace",
            "bun run test",
            "bun run build",
            "git diff --check",
        ] {
            assert!(
                authorize(command).is_ok(),
                "{command}: {:?}",
                authorize(command)
            );
        }
        assert_eq!(
            authorize("cargo test --manifest-path src-tauri/Cargo.toml --workspace").unwrap(),
            ShellCommand {
                program: "cargo".into(),
                arguments: vec![
                    "test".into(),
                    "--manifest-path".into(),
                    "src-tauri/Cargo.toml".into(),
                    "--workspace".into(),
                ],
            }
        );
    }

    #[test]
    fn shell_syntax_and_unlisted_programs_are_refused() {
        assert!(authorize("cargo test; rm -rf /")
            .unwrap_err()
            .contains("metacharacter"));
        assert!(authorize("cargo test && curl evil.example")
            .unwrap_err()
            .contains("metacharacter"));
        assert!(authorize("cargo test | tee /tmp/out")
            .unwrap_err()
            .contains("metacharacter"));
        assert!(authorize("cargo test $(whoami)")
            .unwrap_err()
            .contains("metacharacter"));
        assert!(authorize("rm -rf target")
            .unwrap_err()
            .contains("not an allowlisted"));
        assert!(authorize("sh -c cargo")
            .unwrap_err()
            .contains("not an allowlisted"));
        assert!(authorize("   ").unwrap_err().contains("empty command"));
    }

    /// The program is not the safety boundary: `git` also spells `reset --hard`.
    /// A verification check inspects and builds; it does not mutate history,
    /// install dependencies, or reach a registry.
    #[test]
    fn destructive_and_mutating_subcommands_are_refused() {
        for command in [
            "git reset --hard HEAD~5",
            "git clean -fdx",
            "git push origin main",
            "git checkout main",
            "git commit -am wip",
            "npm install",
            "npm publish",
            "cargo publish",
            "cargo install cargo-audit",
            "yarn add left-pad",
            "pnpm add left-pad",
        ] {
            let error = authorize(command).unwrap_err();
            assert!(
                error.contains("is not a verification action"),
                "{command} was allowed: {error}"
            );
        }
        // And the read/verify verbs still pass.
        for command in [
            "git status --porcelain",
            "git diff --name-only HEAD",
            "git log -1",
            "cargo clippy --workspace",
            "cargo fmt --check",
            "npm run lint",
            "vitest run",
            "tsc -b",
        ] {
            assert!(authorize(command).is_ok(), "{command} was refused");
        }
        // A program that takes a script needs no verb; one that takes a verb needs it.
        assert!(authorize("node scripts/check.js").is_ok());
        assert!(authorize("cargo")
            .unwrap_err()
            .contains("names no subcommand"));
    }

    /// The flag's *value* is what decides whether the check leaves the checkout.
    #[test]
    fn a_command_pointed_outside_the_attempt_repository_is_refused() {
        for command in [
            "git -C /etc status",
            "cargo test --manifest-path ../../other/Cargo.toml",
            "cargo test --manifest-path=/elsewhere/Cargo.toml",
            "npm --prefix /elsewhere run build",
            "yarn --cwd /elsewhere test",
            "git --git-dir=/elsewhere/.git status",
            "git -C ~/other status",
        ] {
            let error = authorize(command).unwrap_err();
            assert!(
                error.contains("outside the repository being verified"),
                "{command}: {error}"
            );
        }
        assert!(authorize("cargo test --manifest-path")
            .unwrap_err()
            .contains("and no value"));
        // Registry code fetched at check time is not verification evidence.
        assert!(authorize("npx some-package")
            .unwrap_err()
            .contains("not an allowlisted"));
        for command in [
            "npm exec some-package",
            "pnpm dlx some-package",
            "yarn dlx some-package",
            "bun x some-package",
        ] {
            assert!(
                authorize(command)
                    .unwrap_err()
                    .contains("downloads and runs code"),
                "{command}"
            );
        }
        // Relative paths inside the checkout are exactly the intended use.
        assert!(authorize("git -C src-tauri status").is_ok());
        assert!(authorize("cargo test --manifest-path=src-tauri/Cargo.toml").is_ok());
    }

    /// A verification worker that reports *after* the deadline expired must not
    /// re-open the terminal gate. `finalize` would recompute the verdict as
    /// `verifying` (blockers are `blocked`, not `failed`), re-pinning the parent
    /// with no path back out: the deadline pass only looks for pending/running
    /// checks, so it could never escalate again.
    #[test]
    fn a_late_verification_result_cannot_reopen_an_escalated_attempt() {
        let (_dir, db, repo) = fixture();
        let attempt_id = attempt(
            &db,
            &repo,
            vec![EvalCheck {
                id: "scrutiny-review".into(),
                label: "Independent scrutiny review".into(),
                kind: EvalKind::Scrutiny,
                required: true,
                executor: completion::WORKER_EXECUTOR.into(),
                command: None,
                required_capabilities: vec!["code_review".into()],
                different_model_family: true,
                reason: "test".into(),
            }],
        );
        let expired = (Utc::now()
            - chrono::Duration::seconds(COMPLETION_VERIFY_TIMEOUT_SECONDS + 60))
        .to_rfc3339();
        db.execute(
            "UPDATE eval_attempts SET started_at=?2 WHERE id=?1",
            params![attempt_id, expired],
        )
        .unwrap();
        assert_eq!(
            escalate_stalled_attempts(&db).unwrap(),
            vec![attempt_id.clone()]
        );
        assert_eq!(
            completion::latest_summary(&db, "s")
                .unwrap()
                .unwrap()
                .verdict,
            completion::CompletionVerdict::Failed
        );
        // The parent is released, which is the point of the escalation.
        assert!(completion::completion_allows_ready(&db, "s").unwrap());

        // The late verifier's evidence is refused outright.
        let refused = completion::record_check(
            &db,
            &attempt_id,
            &CheckRun {
                check_id: "scrutiny-review".into(),
                kind: EvalKind::Scrutiny,
                required: true,
                status: CheckStatus::Passed,
                executor: completion::WORKER_RESULT_EXECUTOR.into(),
                command: None,
                verifier_family: Some("codex".into()),
                detail: Some("looks fine".into()),
                output_digest: Some("digest".into()),
                artifact_refs: vec![],
            },
        )
        .unwrap_err();
        assert!(refused.to_string().contains("terminal"), "{refused}");
        let settled = completion::latest_summary(&db, "s").unwrap().unwrap();
        assert_eq!(settled.verdict, completion::CompletionVerdict::Failed);
        assert!(completion::completion_allows_ready(&db, "s").unwrap());
    }

    /// A command that writes far more than the OS pipe buffer must still be read
    /// to completion. Draining only after the wait deadlocked on a full pipe, so
    /// every passing build hung for the full timeout and was recorded failed.
    #[test]
    fn output_larger_than_the_pipe_buffer_does_not_hang_a_passing_command() {
        let directory = tempfile::tempdir().unwrap();
        let command = ShellCommand {
            program: "python3".into(),
            arguments: vec![
                "-c".into(),
                // 1 MB on stdout and 1 MB on stderr — far past any pipe buffer.
                "import sys; sys.stdout.write('o'*1000000); sys.stderr.write('e'*1000000); sys.exit(0)".into(),
            ],
        };
        let started = Instant::now();
        let outcome = execute(directory.path(), &command);
        assert_eq!(outcome.status, CheckStatus::Passed, "{}", outcome.detail);
        assert!(
            started.elapsed() < Duration::from_secs(60),
            "a passing command must not wait on the timeout"
        );
        assert!(outcome.detail.starts_with("exit 0"));
        // Output is bounded, and the bound is disclosed rather than silent.
        assert!(outcome.detail.contains("[output truncated by Bridge]"));
        assert!(outcome.detail.len() < 4 * MAX_STREAM_BYTES);
    }

    fn fixture() -> (tempfile::TempDir, Connection, String) {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("task");
        std::fs::create_dir(&repo).unwrap();
        for args in [
            vec!["init", "-q", "-b", "main"],
            vec!["config", "user.email", "bridge-test@example.invalid"],
            vec!["config", "user.name", "Bridge Test"],
            vec!["config", "commit.gpgsign", "false"],
        ] {
            assert!(StdCommand::new("git")
                .args(&args)
                .current_dir(&repo)
                .status()
                .unwrap()
                .success());
        }
        std::fs::write(repo.join("file.txt"), "base\n").unwrap();
        for args in [vec!["add", "."], vec!["commit", "-q", "-m", "base"]] {
            assert!(StdCommand::new("git")
                .args(&args)
                .current_dir(&repo)
                .status()
                .unwrap()
                .success());
        }
        let db = store::open(Path::new(":memory:")).unwrap();
        db.execute(
            "INSERT INTO projects(id,name,path,created_at) VALUES('p','Demo',?1,'now')",
            params![repo.to_string_lossy()],
        )
        .unwrap();
        db.execute("INSERT INTO workspaces(id,project_id,city,title,branch,path,status,created_at) VALUES('w','p','Oslo','Task','main',?1,'ready','now')", params![repo.to_string_lossy()]).unwrap();
        db.execute("INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,kind) VALUES('s','w','codex','Parent','waiting','reported','orchestrator')", []).unwrap();
        (dir, db, repo.to_string_lossy().into_owned())
    }

    fn shell_check(id: &str, command: &str) -> EvalCheck {
        EvalCheck {
            id: id.into(),
            label: command.into(),
            kind: EvalKind::Deterministic,
            required: true,
            executor: completion::SHELL_EXECUTOR.into(),
            command: Some(command.into()),
            required_capabilities: vec!["shell".into()],
            different_model_family: false,
            reason: "test".into(),
        }
    }

    fn attempt(db: &Connection, repository_path: &str, checks: Vec<EvalCheck>) -> String {
        let contract = CompletionContract {
            id: uuid::Uuid::new_v4().to_string(),
            workspace_id: "w".into(),
            session_id: "s".into(),
            schema_version: 1,
            acceptance_criteria: vec!["The command passes".into()],
            markdown_projection: None,
            markdown_committed: false,
        };
        let plan = EvalPlan {
            id: uuid::Uuid::new_v4().to_string(),
            contract_id: contract.id.clone(),
            schema_version: 1,
            risk: RiskTier::Low,
            checks,
        };
        let repository = RepositoryStamp {
            head: "abc123".into(),
            dirty_digest: "deadbeef".into(),
        };
        completion::create_flow(
            db,
            &contract,
            &plan,
            "s",
            repository_path,
            &repository,
            Some("claude"),
        )
        .unwrap()
    }

    /// The whole point of #5: a planned `bridge.shell` check must actually run
    /// and record a verdict backed by real output.
    #[test]
    fn a_planned_command_runs_in_the_attempt_repository_and_records_a_real_verdict() {
        let (_dir, db, repo) = fixture();
        let attempt_id = attempt(
            &db,
            &repo,
            vec![shell_check("deterministic-0", "git diff --check")],
        );
        let pending = pending_shell_checks(&db).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].attempt_id, attempt_id);

        assert!(claim(&db, &pending[0]).unwrap());
        // A second pass cannot claim the same check.
        assert!(!claim(&db, &pending[0]).unwrap());
        assert_eq!(
            run_claimed_check(&db, &pending[0]).unwrap(),
            CheckStatus::Passed
        );

        let summary = completion::latest_summary(&db, "s").unwrap().unwrap();
        assert_eq!(summary.passed_required, 1);
        assert_eq!(summary.total_required, 1);
        let check = &summary.checks[0];
        assert_eq!(check.executor, completion::SHELL_EXECUTOR);
        assert!(check.detail.as_ref().unwrap().starts_with("exit 0"));
        // The digest covers the real exit status and output, not a received JSON.
        assert_eq!(
            check.output_digest.as_deref().unwrap(),
            digest(check.detail.as_deref().unwrap())
        );
        assert!(pending_shell_checks(&db).unwrap().is_empty());
        assert!(db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM events WHERE kind='completion.check_executed' AND entity_id='s')",
                [],
                |row| row.get::<_, bool>(0),
            )
            .unwrap());
    }

    #[test]
    fn a_failing_command_records_failed_with_its_captured_output() {
        let (_dir, db, repo) = fixture();
        attempt(
            &db,
            &repo,
            vec![shell_check(
                "deterministic-0",
                "git rev-parse --verify no-such-ref",
            )],
        );
        let pending = pending_shell_checks(&db).unwrap();
        assert!(claim(&db, &pending[0]).unwrap());
        assert_eq!(
            run_claimed_check(&db, &pending[0]).unwrap(),
            CheckStatus::Failed
        );
        let summary = completion::latest_summary(&db, "s").unwrap().unwrap();
        assert_eq!(summary.verdict, completion::CompletionVerdict::Verifying);
        assert!(summary.checks[0]
            .detail
            .as_ref()
            .unwrap()
            .contains("--- stderr ---"));
    }

    #[test]
    fn a_command_outside_the_allowlist_is_blocked_with_the_reason_not_executed() {
        let (_dir, db, repo) = fixture();
        attempt(
            &db,
            &repo,
            vec![shell_check("deterministic-0", "rm -rf src")],
        );
        let pending = pending_shell_checks(&db).unwrap();
        assert!(claim(&db, &pending[0]).unwrap());
        assert_eq!(
            run_claimed_check(&db, &pending[0]).unwrap(),
            CheckStatus::Blocked
        );
        assert!(std::path::Path::new(&repo).join("file.txt").exists());
        let summary = completion::latest_summary(&db, "s").unwrap().unwrap();
        assert!(summary.checks[0]
            .detail
            .as_ref()
            .unwrap()
            .contains("not an allowlisted"));
    }

    #[test]
    fn a_missing_repository_path_blocks_the_check_instead_of_hanging() {
        let (_dir, db, _repo) = fixture();
        attempt(
            &db,
            "/bridge/definitely-not-here",
            vec![shell_check("deterministic-0", "git diff --check")],
        );
        let pending = pending_shell_checks(&db).unwrap();
        assert!(claim(&db, &pending[0]).unwrap());
        assert_eq!(
            run_claimed_check(&db, &pending[0]).unwrap(),
            CheckStatus::Blocked
        );
        let summary = completion::latest_summary(&db, "s").unwrap().unwrap();
        assert!(summary.checks[0]
            .detail
            .as_ref()
            .unwrap()
            .contains("does not exist"));
    }

    /// "Verifying 0/7" must not be a steady state. Past the deadline the attempt
    /// fails, names the checks that never ran, and releases the parent.
    #[test]
    fn an_attempt_past_the_verify_deadline_fails_terminally_and_unblocks_the_parent() {
        let (_dir, db, repo) = fixture();
        let attempt_id = attempt(
            &db,
            &repo,
            vec![
                shell_check("deterministic-0", "git diff --check"),
                EvalCheck {
                    id: "scrutiny-review".into(),
                    label: "Independent scrutiny review".into(),
                    kind: EvalKind::Scrutiny,
                    required: true,
                    executor: completion::WORKER_EXECUTOR.into(),
                    command: None,
                    required_capabilities: vec!["code_review".into()],
                    different_model_family: true,
                    reason: "test".into(),
                },
            ],
        );
        assert!(escalate_stalled_attempts(&db).unwrap().is_empty());

        let expired = (Utc::now()
            - chrono::Duration::seconds(COMPLETION_VERIFY_TIMEOUT_SECONDS + 60))
        .to_rfc3339();
        db.execute(
            "UPDATE eval_attempts SET started_at=?2 WHERE id=?1",
            params![attempt_id, expired],
        )
        .unwrap();

        assert_eq!(
            escalate_stalled_attempts(&db).unwrap(),
            vec![attempt_id.clone()]
        );
        let summary = completion::latest_summary(&db, "s").unwrap().unwrap();
        assert_eq!(summary.verdict, completion::CompletionVerdict::Failed);
        assert!(summary
            .checks
            .iter()
            .all(|check| check.status == CheckStatus::Blocked));
        assert!(summary.checks.iter().any(|check| check
            .detail
            .as_ref()
            .unwrap()
            .contains("No eligible verifier claimed")));
        assert!(summary.checks.iter().all(|check| check
            .detail
            .as_ref()
            .unwrap()
            .contains("deterministic-0, scrutiny-review")));
        // Terminal failure releases the session rather than pinning it in waiting.
        let status: String = db
            .query_row("SELECT status FROM sessions WHERE id='s'", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_ne!(status, "waiting");
        // Escalation is idempotent: the attempt is terminal now.
        assert!(escalate_stalled_attempts(&db).unwrap().is_empty());
    }
}
