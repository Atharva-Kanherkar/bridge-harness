//! The work a review job does, behind one trait so the runner's guarantees
//! (slots, leases, retries, fallback, idempotent posting, cancellation) are
//! tested with scripted steps.
//!
//! Preparing the PR worktree, running the hidden reviewer and posting the
//! review implement [`ReviewSteps`]. Until they exist, [`UnavailableSteps`]
//! fails a job with a plain reason.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use bridge_protocol::messages::{ReviewSeverity, ReviewSide};

use super::store::JobRow;

/// Set when a job is cancelled or its runner loses the lease. A step checks it
/// and stops; a step running a harness kills the process.
#[derive(Debug, Clone, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }

    /// Sleep up to `duration`, waking early on cancellation. True when it was
    /// cancelled.
    pub fn sleep(&self, duration: Duration) -> bool {
        let step = Duration::from_millis(20);
        let mut left = duration;
        while !left.is_zero() {
            if self.is_cancelled() {
                return true;
            }
            let nap = left.min(step);
            std::thread::sleep(nap);
            left -= nap;
        }
        self.is_cancelled()
    }
}

/// How a step failed. The runner decides what each means.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepError {
    /// Worth trying again: a git fetch, a `gh` network error, a provider rate
    /// limit. Retried with backoff.
    Transient(String),
    /// The harness itself cannot run this pass: sign-in, usage limit, crash.
    /// The pass moves to the next fallback harness.
    Harness(String),
    /// Trying again will not help. The job fails with this reason.
    Fatal(String),
    /// The cancel token was set.
    Cancelled,
}

impl StepError {
    pub fn message(&self) -> String {
        match self {
            StepError::Transient(message) | StepError::Harness(message) | StepError::Fatal(message) => message.clone(),
            StepError::Cancelled => "The review was cancelled".into(),
        }
    }
}

/// Runs its cleanup once, when the last holder drops it: after success,
/// error, cancellation or a panic unwinding through the step that held it.
pub struct Guard(std::sync::Mutex<Option<Box<dyn FnOnce() + Send>>>);

impl Guard {
    pub fn new(cleanup: impl FnOnce() + Send + 'static) -> Self {
        Self(std::sync::Mutex::new(Some(Box::new(cleanup))))
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        let cleanup = self.0.get_mut().unwrap_or_else(|poisoned| poisoned.into_inner()).take();
        if let Some(cleanup) = cleanup {
            cleanup();
        }
    }
}

impl std::fmt::Debug for Guard {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Guard")
    }
}

/// What preparing a job produced.
#[derive(Debug, Clone, Default)]
pub struct Prepared {
    /// The worktree the reviewer reads, at the PR head.
    pub worktree: Option<PathBuf>,
    /// The folder of prefetched context (diff, PR body, threads).
    pub context: Option<PathBuf>,
    /// The head actually checked out. When the PR moved since the job was
    /// queued, the job's `head_sha` is updated to this.
    pub head_sha: String,
    /// Removes the checkout and the job folder when the step that holds it
    /// ends, however it ends.
    pub guard: Option<Arc<Guard>>,
}

/// What a pass spent, across every attempt of it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PassUsage {
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub cost_microusd: u64,
    pub denied_requests: u32,
}

impl PassUsage {
    pub fn plus(self, other: PassUsage) -> PassUsage {
        PassUsage {
            tokens_in: self.tokens_in + other.tokens_in,
            tokens_out: self.tokens_out + other.tokens_out,
            cost_microusd: self.cost_microusd + other.cost_microusd,
            denied_requests: self.denied_requests + other.denied_requests,
        }
    }
}

/// Where a step records what each attempt spent, whether or not the attempt
/// succeeds: a paid review that fails still cost what it cost, and a retried
/// one cost every try.
#[derive(Debug, Default)]
pub struct UsageMeter(std::sync::Mutex<PassUsage>);

impl UsageMeter {
    pub fn add(&self, usage: PassUsage) {
        let mut total = self.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        *total = total.plus(usage);
    }

    pub fn total(&self) -> PassUsage {
        *self.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// One pass the runner asks for.
#[derive(Debug, Clone, Copy)]
pub struct PassRequest<'a> {
    pub job: &'a JobRow,
    pub prepared: &'a Prepared,
    pub pass_id: &'a str,
    pub harness: &'a str,
    /// Add each attempt's usage here, success or failure.
    pub usage: &'a UsageMeter,
}

/// A finding as the reviewer produced it, validated, before it is stored.
#[derive(Debug, Clone, PartialEq)]
pub struct DraftFinding {
    pub path: String,
    pub line: u32,
    pub start_line: Option<u32>,
    pub side: ReviewSide,
    pub severity: ReviewSeverity,
    pub title: String,
    pub body: String,
    pub confidence: f64,
    pub fingerprint: String,
    pub outside_diff: bool,
}

/// What a finished pass produced.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PassOutcome {
    pub summary: String,
    pub findings: Vec<DraftFinding>,
    pub session_id: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub cost_microusd: u64,
    pub denied_requests: u32,
}

/// A review on GitHub, posted now or found by its marker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostedReview {
    pub review_id: String,
    pub url: String,
}

/// The marker every posted review body carries, so a retry can tell it
/// already posted. It names the job's `publication_key`, which a retry
/// inherits, so a retry also finds what its original posted.
pub fn post_marker(publication_key: &str) -> String {
    format!("<!-- bridge-review:{publication_key} -->")
}

pub trait ReviewSteps: Send + Sync {
    /// Check the PR head out and write the context folder.
    fn prepare(&self, job: &JobRow, cancel: &CancelToken) -> Result<Prepared, StepError>;
    /// Run one reviewer pass on one harness.
    fn review(&self, request: PassRequest<'_>, cancel: &CancelToken) -> Result<PassOutcome, StepError>;
    /// The review this job, or the job it retried, already posted: the one
    /// whose body carries `post_marker(&job.publication_key)`.
    fn find_posted(&self, job: &JobRow) -> Result<Option<PostedReview>, StepError>;
    /// Post the job's review.
    fn post(&self, job: &JobRow, cancel: &CancelToken) -> Result<PostedReview, StepError>;
    /// Remove whatever the job left on disk, every attempt of it. Idempotent.
    fn cleanup(&self, job_id: &str);
    /// Whether one more job may prepare a checkout now. The runner leaves
    /// queued jobs queued while this is false.
    fn has_capacity(&self) -> bool {
        true
    }
    /// Remove what no live job owns. Run at startup and periodically.
    fn reap(&self) {}
}

/// The steps a build without a reviewer pipeline runs: every job fails at
/// prepare with a plain reason, and nothing is touched.
#[derive(Debug, Clone, Copy, Default)]
pub struct UnavailableSteps;

pub const UNAVAILABLE: &str = "This build cannot run PR reviews yet";

impl ReviewSteps for UnavailableSteps {
    fn prepare(&self, _job: &JobRow, _cancel: &CancelToken) -> Result<Prepared, StepError> {
        Err(StepError::Fatal(UNAVAILABLE.into()))
    }

    fn review(&self, _request: PassRequest<'_>, _cancel: &CancelToken) -> Result<PassOutcome, StepError> {
        Err(StepError::Fatal(UNAVAILABLE.into()))
    }

    fn find_posted(&self, _job: &JobRow) -> Result<Option<PostedReview>, StepError> {
        Ok(None)
    }

    fn post(&self, _job: &JobRow, _cancel: &CancelToken) -> Result<PostedReview, StepError> {
        Err(StepError::Fatal(UNAVAILABLE.into()))
    }

    fn cleanup(&self, _job_id: &str) {}
}
