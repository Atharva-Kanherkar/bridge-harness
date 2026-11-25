//! Handing a finished review to the chat that wrote the pull request, which
//! then works on the findings (opt-in: `ReviewJobSettings::handoff`).
//!
//! The chat is the newest top-level, unarchived chat in a workspace of the
//! job's project whose branch is the PR's head branch. The message goes in as
//! if the person wrote it (`ReviewSteps::hand_off`), so the chat starts a turn
//! or queues behind the one running. A job hands off once, and at most
//! [`MAX_HANDOFFS_PER_DAY`] reviews of one PR go to its chat in a day: with
//! auto-review on push, review, fix, push and review again could otherwise
//! chain without a person.

use std::sync::Arc;

use bridge_protocol::messages::{self as wire, ReviewFindingDecision as Decision, ReviewHandoff};
use chrono::{DateTime, Duration, Utc};
use rusqlite::{params, Connection, OptionalExtension};

use super::store::{self, JobRow};
use crate::runtime::BridgeCore;
use crate::BridgeError;

/// Reviews of one PR sent to its chat in 24 hours, at most.
pub const MAX_HANDOFFS_PER_DAY: u32 = 3;
/// Longest finding body a handoff carries, in characters.
const MAX_BODY_CHARS: usize = 1_500;

/// When the runner offers a job for handoff.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Moment {
    Drafted,
    Posted,
}

/// Hand the job off in the background if the settings ask for it at this
/// moment. Never fails the review: a failure is noted on the job.
pub fn after(core: &Arc<BridgeCore>, job_id: &str, moment: Moment) {
    let wanted = {
        let db = super::lock_db(core);
        super::settings::load(&db).map(|settings| matches(settings.handoff, moment)).unwrap_or(false)
    };
    if !wanted {
        return;
    }
    let core = core.clone();
    let job_id = job_id.to_owned();
    let spawned = std::thread::Builder::new().name(format!("review-handoff-{job_id}")).spawn(move || {
        if let Err(error) = hand_off(&core, &job_id, moment, Utc::now()) {
            eprintln!("[reviews] could not hand review {job_id} to its chat: {error}");
        }
    });
    if let Err(error) = spawned {
        eprintln!("[reviews] could not start a review handoff: {error}");
    }
}

fn matches(setting: ReviewHandoff, moment: Moment) -> bool {
    matches!((setting, moment), (ReviewHandoff::Drafted, Moment::Drafted) | (ReviewHandoff::Posted, Moment::Posted))
}

/// What happened to one offer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Settings do not ask for a handoff now, or the job already handed off.
    Skipped,
    /// Nothing to send.
    Empty,
    /// Not sent; the note says why (also on the job).
    Held(String),
    /// Sent to this chat.
    Sent { session_id: String, chat: String },
}

/// Offer `job_id` for handoff at `moment`. Synchronous: what [`after`] runs.
pub fn hand_off(core: &Arc<BridgeCore>, job_id: &str, moment: Moment, now: DateTime<Utc>) -> Result<Outcome, BridgeError> {
    // Decided under the store lock: sent nowhere yet.
    enum Plan {
        Done(Outcome),
        Note(String),
        Send(String, String, String),
    }
    let plan = {
        let db = super::lock_db(core);
        let settings = super::settings::load(&db)?;
        let job = store::require_job(&db, job_id)?;
        let findings = store::findings(&db, job_id)?;
        let chosen: Vec<&wire::ReviewFinding> = findings.iter().filter(|finding| to_send(finding, moment)).collect();
        if !matches(settings.handoff, moment) || job.handoff_at.is_some() {
            Plan::Done(Outcome::Skipped)
        } else if chosen.is_empty() {
            Plan::Done(Outcome::Empty)
        } else if let Some(branch) = job.pr_branch.as_deref() {
            match author_chat(&db, &job, branch)? {
                None => Plan::Note(format!("Not sent: no chat in this project works on the branch {branch}.")),
                Some(_) if store::handoffs_since(&db, &job, now - Duration::hours(24))? >= MAX_HANDOFFS_PER_DAY => Plan::Note(format!(
                    "Not sent: {MAX_HANDOFFS_PER_DAY} reviews of this pull request already went to its chat today. Send this one yourself if it should go."
                )),
                Some((session_id, chat)) => {
                    let harnesses = store::finished_harnesses(&db, job_id)?;
                    Plan::Send(session_id, chat, message(&job, &chosen, &harnesses, moment))
                }
            }
        } else {
            Plan::Note("Not sent: Bridge does not know this pull request's branch yet.".into())
        }
    };
    let (session_id, chat, text) = match plan {
        Plan::Done(outcome) => return Ok(outcome),
        Plan::Note(note) => {
            store::note_handoff(&super::lock_db(core), job_id, &note)?;
            super::publish(core, job_id);
            return Ok(Outcome::Held(note));
        }
        Plan::Send(session_id, chat, text) => (session_id, chat, text),
    };
    // Outside the store lock: starting a turn takes it.
    if let Err(error) = core.reviews.steps().hand_off(&session_id, &text) {
        let note = format!("Not sent to {chat}: {error}");
        store::note_handoff(&super::lock_db(core), job_id, &note)?;
        super::publish(core, job_id);
        return Ok(Outcome::Held(note));
    }
    let recorded = store::record_handoff(&super::lock_db(core), job_id, &session_id, &chat, now)?;
    super::publish(core, job_id);
    Ok(if recorded { Outcome::Sent { session_id, chat } } else { Outcome::Skipped })
}

/// Drafted: everything not dropped (nobody has triaged yet). Posted: what
/// went to GitHub.
fn to_send(finding: &wire::ReviewFinding, moment: Moment) -> bool {
    match moment {
        Moment::Drafted => finding.decision != Decision::Dropped,
        Moment::Posted => finding.posted_at.is_some() || matches!(finding.decision, Decision::Kept | Decision::Edited),
    }
}

/// The newest top-level, unarchived chat in a workspace of the job's project
/// on `branch` (latest started, then latest created): id and name.
fn author_chat(db: &Connection, job: &JobRow, branch: &str) -> Result<Option<(String, String)>, BridgeError> {
    Ok(db
        .query_row(
            "SELECT s.id, s.label FROM sessions s JOIN workspaces w ON w.id=s.workspace_id
             WHERE w.project_id=(SELECT project_id FROM workspaces WHERE id=?1) AND w.branch=?2
               AND s.parent_session_id IS NULL AND s.archived_at IS NULL
             ORDER BY COALESCE(s.started_at, '') DESC, s.rowid DESC
             LIMIT 1",
            params![job.workspace_id, branch],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?)
}

fn harness_name(harness: &str) -> String {
    match harness {
        "claude" => "Claude".into(),
        "codex" => "Codex".into(),
        "opencode" => "OpenCode".into(),
        other => other.to_owned(),
    }
}

fn cut(text: &str) -> String {
    match text.char_indices().nth(MAX_BODY_CHARS) {
        Some((index, _)) => format!("{}…", &text[..index]),
        None => text.to_owned(),
    }
}

/// The message the chat gets.
pub fn message(job: &JobRow, findings: &[&wire::ReviewFinding], harnesses: &[String], moment: Moment) -> String {
    let title = job.pr_title.as_deref().map(|title| format!(" ({title})")).unwrap_or_default();
    let by = if harnesses.is_empty() { String::new() } else { format!(" with {}", harnesses.iter().map(|h| harness_name(h)).collect::<Vec<_>>().join(" and ")) };
    let count = findings.len();
    let plural = if count == 1 { "finding" } else { "findings" };
    let mut text = match moment {
        Moment::Drafted => format!(
            "Bridge reviewed pull request #{}{title}{by}. {count} {plural} came back. Nobody has triaged them yet, so judge each one.\n",
            job.number
        ),
        Moment::Posted => format!("Bridge reviewed pull request #{}{title}{by} and posted {count} {plural} on GitHub.\n", job.number),
    };
    for (index, finding) in findings.iter().enumerate() {
        let lines = match finding.start_line.filter(|start| *start < finding.line) {
            Some(start) => format!("{start}-{}", finding.line),
            None => finding.line.to_string(),
        };
        let body = finding.edited_body.as_deref().unwrap_or(&finding.body);
        text.push_str(&format!(
            "\n{}. [{}] {}:{} {}\n{}\n",
            index + 1,
            finding.severity.as_str(),
            finding.path,
            lines,
            finding.title,
            cut(body.trim())
        ));
    }
    text.push_str("\nFix what you agree with on this branch, and say which findings you disagree with and why.");
    if let Some(url) = job.posted_review_url.as_deref().filter(|_| moment == Moment::Posted) {
        text.push_str(&format!(" The review is at {url}."));
    }
    text
}

#[cfg(test)]
#[path = "handoff_tests.rs"]
mod tests;
