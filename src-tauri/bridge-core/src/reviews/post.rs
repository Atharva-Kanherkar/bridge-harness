//! Posting a review. Bridge posts, not the reviewer: one `COMMENT` review at
//! the reviewed commit, with the chosen findings inline and the rest of the
//! review in its body.
//!
//! The event is not a choice anywhere. [`ReviewPayload::event`] is
//! [`CommentEvent`], a unit type whose only serialization is `"COMMENT"`, and
//! [`crate::github_surface::GithubSurface::create_pr_review`] takes nothing
//! but a [`ReviewPayload`]. Approving, requesting changes or merging cannot
//! be expressed.
//!
//! What posts: in draft mode the findings a person kept or edited; in auto
//! mode every finding not dropped. Inline findings become comments; findings
//! outside the diff go in the body. Every posted body carries
//! [`post_marker`], which a retry looks for before posting again.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use bridge_protocol::messages::{self as wire, ReviewFindingDecision, ReviewPostMode, ReviewSeverity, ReviewSide};
use serde::{Serialize, Serializer};

use super::steps::{post_marker, CancelToken, PostedFinding, PostedReview, StepError};
use super::store::{self, JobRow};
use crate::github_surface::{GithubRepository, GithubSurfaceError};
use crate::runtime::BridgeCore;

/// GitHub refuses a comment or review body over 65,536 characters.
pub const MAX_COMMENT_CHARS: usize = 65_000;
/// The review body, with room for the footer and the marker.
pub const MAX_REVIEW_BODY_CHARS: usize = 64_000;
/// How much of the reviewers' summary the body keeps.
const MAX_SUMMARY_CHARS: usize = 16_000;
const CUT: &str = "\n\n_(cut to fit GitHub's limit)_";

/// The only review event Bridge sends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CommentEvent;

impl Serialize for CommentEvent {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str("COMMENT")
    }
}

/// The body of `POST repos/{owner}/{repo}/pulls/{n}/reviews`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReviewPayload {
    pub commit_id: String,
    pub event: CommentEvent,
    pub body: String,
    pub comments: Vec<InlineComment>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InlineComment {
    pub path: String,
    pub line: u32,
    pub side: ReviewSide,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_line: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_side: Option<ReviewSide>,
    pub body: String,
}

/// What a post is built from.
#[derive(Debug, Clone)]
pub struct PostInput<'a> {
    pub job: &'a JobRow,
    /// Every finding of the job, each with the harnesses that found it (two
    /// when both reviewers of a second opinion flagged it).
    pub findings: Vec<(wire::ReviewFinding, Vec<String>)>,
    /// The harnesses whose passes finished, for the footer.
    pub harnesses: Vec<String>,
    /// Attribution hiding: no footer naming Bridge or a harness.
    pub hide_attribution: bool,
    /// The PR's head now, when it could be read.
    pub current_head: Option<String>,
}

/// A built post: the payload, and which findings it carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostPlan {
    pub payload: ReviewPayload,
    /// Inline findings, in the order of `payload.comments`.
    pub inline: Vec<String>,
    /// Outside-diff findings in the body, each with its heading line.
    pub outside: Vec<(String, String)>,
}

/// The findings a job posts: kept and edited in draft mode, everything not
/// dropped in auto mode.
pub fn chosen(post_mode: ReviewPostMode, finding: &wire::ReviewFinding) -> bool {
    match post_mode {
        ReviewPostMode::Draft => matches!(finding.decision, ReviewFindingDecision::Kept | ReviewFindingDecision::Edited),
        ReviewPostMode::Auto => finding.decision != ReviewFindingDecision::Dropped,
    }
}

fn label(severity: ReviewSeverity) -> &'static str {
    match severity {
        ReviewSeverity::Critical => "Critical",
        ReviewSeverity::Major => "Major",
        ReviewSeverity::Minor => "Minor",
        ReviewSeverity::Nit => "Nit",
    }
}

/// The words a finding posts with: a person's edit wins.
fn text(finding: &wire::ReviewFinding) -> &str {
    match (finding.decision, finding.edited_body.as_deref()) {
        (ReviewFindingDecision::Edited, Some(edited)) => edited,
        _ => &finding.body,
    }
}

fn footer(harnesses: &[&str], hide: bool) -> String {
    if hide || harnesses.is_empty() {
        return String::new();
    }
    format!("\n\n<sub>Bridge review · {}</sub>", harnesses.join(", "))
}

/// At most `limit` characters, with a note when anything was cut.
fn fit(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_owned();
    }
    let keep = limit.saturating_sub(CUT.chars().count());
    let mut cut: String = text.chars().take(keep).collect();
    cut.push_str(CUT);
    cut
}

fn short(sha: &str) -> &str {
    sha.get(..12).unwrap_or(sha)
}

/// One inline comment's body: label, title, text, footer.
pub fn comment_body(finding: &wire::ReviewFinding, harnesses: &[String], hide: bool) -> String {
    let harnesses: Vec<&str> = harnesses.iter().map(String::as_str).collect();
    let footer = footer(&harnesses, hide);
    let head = format!("**{}** · {}\n\n", label(finding.severity), finding.title);
    let room = MAX_COMMENT_CHARS.saturating_sub(head.chars().count() + footer.chars().count());
    format!("{head}{}{footer}", fit(text(finding), room))
}

/// How an outside-diff finding is headed in the body. A review found by its
/// marker holds a finding when its body holds this line.
pub fn outside_heading(finding: &wire::ReviewFinding) -> String {
    format!("**{}** · `{}:{}` · {}", label(finding.severity), finding.path, finding.line, finding.title)
}

/// Build the review a job posts.
pub fn build(input: &PostInput<'_>) -> PostPlan {
    let job = input.job;
    let mut chosen_findings: Vec<&(wire::ReviewFinding, Vec<String>)> =
        input.findings.iter().filter(|(finding, _)| chosen(job.post_mode, finding)).collect();
    // Stored order is already severity then confidence; keep it stable.
    chosen_findings.sort_by_key(|(finding, _)| finding.outside_diff);

    let mut comments = Vec::new();
    let mut inline = Vec::new();
    let mut outside_entries = Vec::new();
    for (finding, harnesses) in &chosen_findings {
        if finding.outside_diff {
            outside_entries.push(finding);
            continue;
        }
        let start_line = finding.start_line.filter(|start| *start < finding.line);
        comments.push(InlineComment {
            path: finding.path.clone(),
            line: finding.line,
            side: finding.side,
            start_line,
            start_side: start_line.map(|_| finding.side),
            body: comment_body(finding, harnesses, input.hide_attribution),
        });
        inline.push(finding.id.clone());
    }

    let ran: Vec<&str> = input.harnesses.iter().map(String::as_str).collect();
    let tail = format!("{}\n\n{}", footer(&ran, input.hide_attribution), post_marker(&job.publication_key));
    let mut body = String::new();
    // Reviewer names reach GitHub only while attribution is shown, read now.
    let summary = match (&job.named_summary, input.hide_attribution) {
        (Some(named), false) => named.as_str(),
        _ => job.summary.as_deref().unwrap_or_default(),
    }
    .trim();
    if !summary.is_empty() {
        body.push_str(&fit(summary, MAX_SUMMARY_CHARS));
    } else if chosen_findings.is_empty() {
        body.push_str("No issues found.");
    } else {
        body.push_str(&format!("Review of `{}`.", short(&job.head_sha)));
    }
    if let Some(now) = input.current_head.as_deref().filter(|now| !now.is_empty() && *now != job.head_sha) {
        body.push_str(&format!(
            "\n\nReviewed at `{}`. The pull request has moved on to `{}` since, so comments on lines that changed show as outdated.",
            short(&job.head_sha),
            short(now)
        ));
    }

    let mut outside = Vec::new();
    if !outside_entries.is_empty() {
        body.push_str("\n\n### Outside the diff");
        // Room for the "more did not fit" line as well as the tail.
        let budget = MAX_REVIEW_BODY_CHARS.saturating_sub(tail.chars().count() + 120);
        let mut left_out = 0;
        for finding in &outside_entries {
            let heading = outside_heading(finding);
            let entry = format!("\n\n{heading}\n\n{}", text(finding));
            if left_out == 0 && body.chars().count() + entry.chars().count() <= budget {
                body.push_str(&entry);
                outside.push((finding.id.clone(), heading));
            } else {
                left_out += 1;
            }
        }
        if left_out > 0 {
            body.push_str(&format!("\n\n_{left_out} more outside the diff did not fit in this review._"));
        }
    }
    body.push_str(&tail);

    PostPlan { payload: ReviewPayload { commit_id: job.head_sha.clone(), event: CommentEvent, body, comments }, inline, outside }
}

/// A review on GitHub, as the reviews list returns it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoundReview {
    pub id: String,
    pub url: String,
    pub body: String,
    /// The commit the review is on, as GitHub reports it.
    pub commit_id: Option<String>,
}

/// An inline comment of a posted review, as GitHub returns it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReturnedComment {
    pub id: String,
    pub path: String,
    /// `original_line`, else `line`: where it was posted, even once outdated.
    pub line: Option<u32>,
    pub side: Option<ReviewSide>,
    pub body: String,
}

/// Each inline finding's comment: same path, line and side, the exact body
/// first, then any.
fn match_comments(plan: &PostPlan, returned: &[ReturnedComment]) -> HashMap<String, String> {
    let mut taken = HashSet::new();
    let mut matched = HashMap::new();
    for exact in [true, false] {
        for (finding_id, comment) in plan.inline.iter().zip(&plan.payload.comments) {
            if matched.contains_key(finding_id) {
                continue;
            }
            let found = returned.iter().find(|candidate| {
                !taken.contains(&candidate.id)
                    && candidate.path == comment.path
                    && candidate.line == Some(comment.line)
                    && candidate.side.is_none_or(|side| side == comment.side)
                    && (!exact || candidate.body.trim() == comment.body.trim())
            });
            if let Some(found) = found {
                taken.insert(found.id.clone());
                matched.insert(finding_id.clone(), found.id.clone());
            }
        }
    }
    matched
}

/// What a review this job just created holds: every finding it carried,
/// inline ones with their comment ids when GitHub returned them.
pub fn posted_now(plan: &PostPlan, returned: &[ReturnedComment]) -> Vec<PostedFinding> {
    let matched = match_comments(plan, returned);
    plan.inline
        .iter()
        .map(|id| PostedFinding { finding_id: id.clone(), comment_id: matched.get(id).cloned() })
        .chain(plan.outside.iter().map(|(id, _)| PostedFinding { finding_id: id.clone(), comment_id: None }))
        .collect()
}

/// What a review found by its marker holds of this job's findings: an inline
/// finding with a matching comment, an outside one whose heading is in the
/// body. A review an earlier job posted may hold none of them.
pub fn held_by(plan: &PostPlan, found: &FoundReview, returned: &[ReturnedComment]) -> Vec<PostedFinding> {
    let matched = match_comments(plan, returned);
    plan.inline
        .iter()
        .filter_map(|id| matched.get(id).map(|comment| PostedFinding { finding_id: id.clone(), comment_id: Some(comment.clone()) }))
        .chain(
            plan.outside
                .iter()
                .filter(|(_, heading)| found.body.contains(heading.as_str()))
                .map(|(id, _)| PostedFinding { finding_id: id.clone(), comment_id: None }),
        )
        .collect()
}

/// Whether a `gh` failure is worth another try. The runner looks for the
/// marker before every attempt, so a retried post never doubles one GitHub
/// took.
pub fn gh_error(error: GithubSurfaceError) -> StepError {
    match error {
        // Throttled: wait as long as GitHub asked, then try again.
        GithubSurfaceError::RateLimited { message, retry_after, .. } => {
            eprintln!("[reviews] GitHub is rate limiting Bridge: {message}");
            StepError::RateLimited("GitHub is rate limiting Bridge. It will try again shortly.".into(), retry_after)
        }
        GithubSurfaceError::CommandFailed { ref stderr, .. } => {
            let lower = stderr.to_ascii_lowercase();
            // A 403 can be throttling too; one that says so waits, not fails.
            if lower.contains("rate limit") || lower.contains("http 429") {
                eprintln!("[reviews] GitHub is rate limiting Bridge: {}", stderr.trim());
                return StepError::RateLimited("GitHub is rate limiting Bridge. It will try again shortly.".into(), Duration::from_secs(60));
            }
            let refused = ["http 401", "http 403", "http 404", "http 422"].iter().any(|code| lower.contains(code));
            let passing = [
                "timed out",
                "timeout",
                "http 500",
                "http 502",
                "http 503",
                "http 504",
                "connection",
                "error connecting",
                "network",
                "could not resolve host",
                "eof",
                "tls",
            ]
            .iter()
            .any(|sign| lower.contains(sign));
            if lower.contains("http 422") {
                StepError::fatal_logged("GitHub refused the review.", stderr.trim())
            } else if passing && !refused {
                StepError::transient_logged(error.user_message(), &error)
            } else {
                StepError::fatal_logged(error.user_message(), &error)
            }
        }
        GithubSurfaceError::Io(_) => StepError::transient_logged(error.user_message(), &error),
        other => StepError::fatal_logged(other.user_message(), &other),
    }
}

/// The job's workspace, and the repository it reviews on that workspace's
/// host.
fn target(core: &BridgeCore, job: &JobRow) -> Result<(PathBuf, GithubRepository), StepError> {
    let workspace = PathBuf::from(
        core.workspace_path(&job.workspace_id).map_err(|error| StepError::fatal_logged("The review's project is no longer in Bridge.", error))?,
    );
    let repository = super::prepare::job_repository(core, &workspace, job);
    Ok((workspace, repository))
}

/// The post a job makes now, from what is stored.
///
/// The job is read again: the runner's copy was claimed before the review,
/// so its summary (and in auto mode everything the review wrote) is stale.
fn plan(core: &BridgeCore, job: &JobRow, current_head: Option<String>) -> Result<PostPlan, StepError> {
    let fatal = |error: crate::BridgeError| StepError::fatal_logged("Bridge couldn't read the review it saved. Try again.", error);
    let (job, findings, harness_of, harnesses, hide_attribution) = {
        let db = super::lock_db(core);
        (
            store::require_job(&db, &job.id).map_err(fatal)?,
            store::findings(&db, &job.id).map_err(fatal)?,
            store::finding_harnesses(&db, &job.id).map_err(fatal)?,
            store::finished_harnesses(&db, &job.id).map_err(fatal)?,
            crate::attribution_settings::hide_enabled(&db),
        )
    };
    let findings = findings
        .into_iter()
        .map(|finding| {
            let harnesses = harness_of.get(&finding.id).cloned().unwrap_or_default();
            (finding, harnesses)
        })
        .collect();
    Ok(build(&PostInput { job: &job, findings, harnesses, hide_attribution, current_head }))
}

/// The review this job, or the job it retried, already posted: the one whose
/// body carries the job's marker, with the findings it holds.
pub fn find_posted(core: &Arc<BridgeCore>, job: &JobRow) -> Result<Option<PostedReview>, StepError> {
    let (workspace, repository) = target(core, job)?;
    let marker = post_marker(&job.publication_key);
    let reviews = core.github_surface.pr_reviews(&workspace, &repository, job.number).map_err(gh_error)?;
    let Some(found) = reviews.into_iter().find(|review| review.body.contains(&marker)) else {
        return Ok(None);
    };
    // Best effort: without its comments, the review still counts as posted.
    let comments = core.github_surface.pr_review_comments(&workspace, &repository, job.number, &found.id).unwrap_or_default();
    let findings = held_by(&plan(core, job, None)?, &found, &comments);
    Ok(Some(PostedReview { review_id: found.id, url: found.url, commit_id: found.commit_id, findings }))
}

/// Post the job's review at the reviewed commit.
pub fn post(core: &Arc<BridgeCore>, job: &JobRow, cancel: &CancelToken) -> Result<PostedReview, StepError> {
    let (workspace, repository) = target(core, job)?;
    // Best effort: a head that cannot be read only leaves out the "moved on" line.
    let current_head = core.github_surface.pr_head(&workspace, job.number).ok().map(|head| head.head_sha);
    let plan = plan(core, job, current_head)?;
    if cancel.is_cancelled() {
        return Err(StepError::Cancelled);
    }
    let created = core.github_surface.create_pr_review(&workspace, &repository, job.number, &plan.payload).map_err(gh_error)?;
    let comments = core.github_surface.pr_review_comments(&workspace, &repository, job.number, &created.id).unwrap_or_default();
    let commit_id = created.commit_id.or_else(|| Some(plan.payload.commit_id.clone()));
    Ok(PostedReview { review_id: created.id, url: created.url, commit_id, findings: posted_now(&plan, &comments) })
}

#[cfg(test)]
#[path = "post_tests.rs"]
mod tests;
