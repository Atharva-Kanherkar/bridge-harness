//! The review runner: Bridge's own background loop that drives review jobs,
//! with its own slots, never the chat worker pool.
//!
//! Each tick reclaims expired leases, then claims queued jobs FIFO while a
//! slot is free (`maxConcurrentReviews`, default 2) and drives each on its own
//! thread. A driven job holds a lease its heartbeat renews; when renewal
//! fails (the job was cancelled or reclaimed) the job's cancel token trips and
//! its step stops. Transient step errors retry with backoff, a harness that
//! cannot run moves the pass to the next fallback harness, and every post is
//! preceded by a marker check, so a retried post never creates a second
//! review.

use std::collections::HashMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, RwLock};
use std::thread::JoinHandle;
use std::time::Duration;

use bridge_protocol::messages::{ReviewJobStatus as Status, ReviewPassStatus, ReviewPostMode};
use chrono::{DateTime, Utc};
use uuid::Uuid;

use super::steps::{CancelToken, PassRequest, Prepared, ReviewSteps, StepError, UnavailableSteps};
use super::store::{self, JobRow};
use crate::runtime::BridgeCore;
use crate::BridgeError;

/// The runner's live state in one process.
pub struct ReviewRuntime {
    steps: RwLock<Arc<dyn ReviewSteps>>,
    tokens: Mutex<HashMap<String, CancelToken>>,
    wake: (Mutex<bool>, Condvar),
}

impl Default for ReviewRuntime {
    fn default() -> Self {
        Self {
            steps: RwLock::new(Arc::new(UnavailableSteps)),
            tokens: Mutex::new(HashMap::new()),
            wake: (Mutex::new(false), Condvar::new()),
        }
    }
}

impl std::fmt::Debug for ReviewRuntime {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let driving = self.tokens.lock().map(|tokens| tokens.len()).unwrap_or_default();
        formatter.debug_struct("ReviewRuntime").field("driving", &driving).finish()
    }
}

impl ReviewRuntime {
    pub fn steps(&self) -> Arc<dyn ReviewSteps> {
        self.steps.read().unwrap_or_else(|poisoned| poisoned.into_inner()).clone()
    }

    /// Replace the steps jobs run. Hosts install the real pipeline; tests a
    /// scripted one.
    pub fn set_steps(&self, steps: Arc<dyn ReviewSteps>) {
        *self.steps.write().unwrap_or_else(|poisoned| poisoned.into_inner()) = steps;
    }

    fn register(&self, job_id: &str) -> CancelToken {
        let token = CancelToken::new();
        self.tokens
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(job_id.to_owned(), token.clone());
        token
    }

    fn unregister(&self, job_id: &str) {
        self.tokens.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).remove(job_id);
    }

    /// Trip the cancel token of a job this process is driving. False when no
    /// runner here holds it.
    pub fn trip(&self, job_id: &str) -> bool {
        let tokens = self.tokens.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        match tokens.get(job_id) {
            Some(token) => {
                token.cancel();
                true
            }
            None => false,
        }
    }

    /// Ask the loop to tick now: a job was queued or a slot freed.
    pub fn wake(&self) {
        let (flag, signal) = &self.wake;
        *flag.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = true;
        signal.notify_all();
    }

    fn wait(&self, timeout: Duration) {
        let (flag, signal) = &self.wake;
        let guard = flag.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let (mut guard, _) = signal
            .wait_timeout_while(guard, timeout, |woken| !*woken)
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *guard = false;
    }
}

/// Timings, adjustable so tests run in milliseconds.
#[derive(Debug, Clone)]
pub struct RunnerConfig {
    /// How often a driven job renews its lease. A third of the lease.
    pub renew_every: Duration,
    /// The first retry's delay; each later one doubles it.
    pub retry_backoff: Duration,
    /// Retries after the first attempt of a step that failed transiently.
    pub step_retries: u32,
    /// How long an idle loop waits for a wake before it ticks anyway.
    pub poll: Duration,
}

impl Default for RunnerConfig {
    fn default() -> Self {
        Self {
            renew_every: Duration::from_secs(15),
            retry_backoff: Duration::from_secs(2),
            step_retries: 3,
            poll: Duration::from_secs(2),
        }
    }
}

pub type Clock = Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>;

/// One runner: an owner id for its leases, its busy slots, its timings.
pub struct Runner {
    core: Arc<BridgeCore>,
    owner: String,
    active: Arc<AtomicUsize>,
    config: RunnerConfig,
    clock: Clock,
}

impl Runner {
    pub fn new(core: Arc<BridgeCore>) -> Self {
        Self::with(core, RunnerConfig::default(), Arc::new(Utc::now))
    }

    pub fn with(core: Arc<BridgeCore>, config: RunnerConfig, clock: Clock) -> Self {
        Self { core, owner: format!("reviews-{}", Uuid::new_v4()), active: Arc::new(AtomicUsize::new(0)), config, clock }
    }

    pub fn owner(&self) -> &str {
        &self.owner
    }

    /// Jobs this runner is driving now.
    pub fn active(&self) -> usize {
        self.active.load(Ordering::SeqCst)
    }

    /// At boot, before the first tick: nothing in flight survived the
    /// restart. Interrupt it and remove what it left on disk.
    pub fn recover(&self) -> Result<Vec<String>, BridgeError> {
        let interrupted = {
            let db = self.core.db.lock().unwrap();
            store::interrupt_in_flight(&db, (self.clock)())?
        };
        let steps = self.core.reviews.steps();
        for id in &interrupted {
            steps.cleanup(id);
            super::publish(&self.core, id);
        }
        Ok(interrupted)
    }

    /// Reclaim expired leases, then claim what fits in the free slots. The
    /// handles are for tests; the loop drops them.
    pub fn tick(&self) -> Result<Vec<JoinHandle<()>>, BridgeError> {
        let now = (self.clock)();
        let (reclaimed, limit) = {
            let db = self.core.db.lock().unwrap();
            let reclaimed = store::reclaim_expired(&db, now)?;
            let limit = crate::reviewer_settings::load(&db)
                .map(|settings| settings.max_concurrent_reviews)
                .unwrap_or(bridge_protocol::messages::DEFAULT_MAX_CONCURRENT_REVIEWS);
            (reclaimed, usize::from(limit.clamp(1, 4)))
        };
        for id in &reclaimed {
            super::publish(&self.core, id);
        }
        let mut handles = Vec::new();
        while self.active() < limit {
            let claimed = {
                let db = self.core.db.lock().unwrap();
                store::claim_next(&db, &self.owner, (self.clock)())?
            };
            let Some(job) = claimed else { break };
            super::publish(&self.core, &job.id);
            self.active.fetch_add(1, Ordering::SeqCst);
            let driver = Driver {
                core: self.core.clone(),
                owner: self.owner.clone(),
                config: self.config.clone(),
                clock: self.clock.clone(),
                slot: SlotGuard(self.active.clone()),
            };
            let handle = std::thread::Builder::new()
                .name(format!("review-{}", job.id))
                .spawn(move || driver.drive(job))
                .map_err(BridgeError::Io)?;
            handles.push(handle);
        }
        Ok(handles)
    }
}

/// Started once per host beside the other maintenance loops.
pub fn start_review_maintenance(core: Arc<BridgeCore>) {
    let _ = std::thread::Builder::new().name("review-runner".into()).spawn(move || {
        let runner = Runner::new(core.clone());
        if let Err(error) = runner.recover() {
            eprintln!("[reviews] startup recovery failed: {error}");
        }
        loop {
            if let Err(error) = runner.tick() {
                eprintln!("[reviews] tick failed: {error}");
            }
            core.reviews.wait(runner.config.poll);
        }
    });
}

/// Frees its slot however the driver ends, panics included.
struct SlotGuard(Arc<AtomicUsize>);

impl Drop for SlotGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

struct Driver {
    core: Arc<BridgeCore>,
    owner: String,
    config: RunnerConfig,
    clock: Clock,
    slot: SlotGuard,
}

impl Driver {
    fn now(&self) -> DateTime<Utc> {
        (self.clock)()
    }

    fn drive(self, job: JobRow) {
        let job_id = job.id.clone();
        let token = self.core.reviews.register(&job_id);
        let finished = Arc::new(AtomicBool::new(false));
        let heartbeat = self.heartbeat(&job_id, &token, &finished);
        let outcome = catch_unwind(AssertUnwindSafe(|| self.run(job, &token))).unwrap_or_else(|panic| {
            let detail = panic
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| panic.downcast_ref::<&str>().map(|text| text.to_string()))
                .unwrap_or_else(|| "unknown panic".into());
            Err(StepError::Fatal(format!("The review stopped unexpectedly: {detail}")))
        });
        finished.store(true, Ordering::SeqCst);
        if let Some(heartbeat) = heartbeat {
            let _ = heartbeat.join();
        }
        self.settle(&job_id, outcome);
        self.core.reviews.unregister(&job_id);
        drop(self.slot);
        self.core.reviews.wake();
    }

    /// Renew the lease until the job ends. A failed renewal trips the token:
    /// someone else owns the job now, or nobody should.
    fn heartbeat(&self, job_id: &str, token: &CancelToken, finished: &Arc<AtomicBool>) -> Option<JoinHandle<()>> {
        let core = self.core.clone();
        let owner = self.owner.clone();
        let every = self.config.renew_every;
        let clock = self.clock.clone();
        let token = token.clone();
        let finished = finished.clone();
        let job_id = job_id.to_owned();
        std::thread::Builder::new()
            .name(format!("review-lease-{job_id}"))
            .spawn(move || loop {
                let mut waited = Duration::ZERO;
                while waited < every {
                    if finished.load(Ordering::SeqCst) {
                        return;
                    }
                    let nap = (every - waited).min(Duration::from_millis(20));
                    std::thread::sleep(nap);
                    waited += nap;
                }
                if finished.load(Ordering::SeqCst) {
                    return;
                }
                let renewed = {
                    let db = core.db.lock().unwrap();
                    store::renew(&db, &job_id, &owner, clock()).unwrap_or(false)
                };
                if !renewed {
                    token.cancel();
                    return;
                }
            })
            .ok()
    }

    fn run(&self, job: JobRow, token: &CancelToken) -> Result<(), StepError> {
        let steps = self.core.reviews.steps();
        let mut job = job;
        if job.status == Status::Preparing {
            let prepared = self.retrying(token, || steps.prepare(&job, token))?;
            if !prepared.head_sha.is_empty() && prepared.head_sha != job.head_sha {
                let db = self.core.db.lock().unwrap();
                if store::set_head(&db, &job.id, &self.owner, &prepared.head_sha).map_err(fatal)? {
                    job.head_sha = prepared.head_sha.clone();
                }
            }
            self.advance(&job, Status::Preparing, Status::Reviewing, false)?;
            self.review(&job, &prepared, steps.as_ref(), token)?;
            // Posting reads the stored findings, not the worktree.
            steps.cleanup(&job.id);
            let auto = job.post_mode == ReviewPostMode::Auto;
            self.advance(&job, Status::Reviewing, Status::Drafted, !auto)?;
            if !auto {
                return Ok(());
            }
            self.advance(&job, Status::Drafted, Status::Posting, false)?;
        }
        self.post(&job, steps.as_ref(), token)
    }

    fn advance(&self, job: &JobRow, from: Status, to: Status, release: bool) -> Result<(), StepError> {
        let moved = {
            let db = self.core.db.lock().unwrap();
            store::advance(&db, &job.id, &self.owner, from, to, release).map_err(fatal)?
        };
        if !moved {
            // Cancelled or reclaimed: the job is not ours to move any more.
            return Err(StepError::Cancelled);
        }
        super::publish(&self.core, &job.id);
        Ok(())
    }

    /// Run a step, retrying transient failures with doubling backoff.
    fn retrying<T>(&self, token: &CancelToken, mut step: impl FnMut() -> Result<T, StepError>) -> Result<T, StepError> {
        let mut attempt = 0;
        loop {
            if token.is_cancelled() {
                return Err(StepError::Cancelled);
            }
            match step() {
                Err(StepError::Transient(_)) if attempt < self.config.step_retries => {
                    let delay = self.config.retry_backoff.saturating_mul(1 << attempt.min(16));
                    attempt += 1;
                    if token.sleep(delay) {
                        return Err(StepError::Cancelled);
                    }
                }
                Err(_) if token.is_cancelled() => return Err(StepError::Cancelled),
                other => return other,
            }
        }
    }

    /// One pass per requested harness. A harness that cannot run hands its
    /// pass to the next fallback harness not tried yet. The job goes on while
    /// at least one pass finished.
    fn review(&self, job: &JobRow, prepared: &Prepared, steps: &dyn ReviewSteps, token: &CancelToken) -> Result<(), StepError> {
        let fallbacks = {
            let db = self.core.db.lock().unwrap();
            crate::reviewer_settings::load(&db).map(|settings| settings.fallback_harnesses).unwrap_or_default()
        };
        let mut summaries = Vec::new();
        let mut finished = 0;
        let mut last_error = None;
        for (slot, requested) in job.harnesses.iter().enumerate() {
            let mut tried = vec![requested.clone()];
            let mut harness = requested.clone();
            loop {
                if token.is_cancelled() {
                    return Err(StepError::Cancelled);
                }
                let pass_id = {
                    let db = self.core.db.lock().unwrap();
                    store::insert_pass(&db, &job.id, slot as u32, &harness, self.now()).map_err(fatal)?
                };
                super::publish(&self.core, &job.id);
                let request = PassRequest { job, prepared, pass_id: &pass_id, harness: &harness };
                let result = self.retrying(token, || steps.review(request, token));
                let db = self.core.db.lock().unwrap();
                match result {
                    Ok(outcome) => {
                        store::finish_pass(&db, &pass_id, ReviewPassStatus::Done, None, Some(&outcome), self.now()).map_err(fatal)?;
                        store::insert_findings(&db, &job.id, &pass_id, &outcome.findings, self.now()).map_err(fatal)?;
                        drop(db);
                        super::publish(&self.core, &job.id);
                        finished += 1;
                        if !outcome.summary.trim().is_empty() {
                            summaries.push(outcome.summary);
                        }
                        break;
                    }
                    Err(StepError::Cancelled) => {
                        store::finish_pass(&db, &pass_id, ReviewPassStatus::Cancelled, None, None, self.now()).map_err(fatal)?;
                        drop(db);
                        super::publish(&self.core, &job.id);
                        return Err(StepError::Cancelled);
                    }
                    Err(StepError::Harness(message)) => {
                        store::finish_pass(&db, &pass_id, ReviewPassStatus::Failed, Some(&message), None, self.now()).map_err(fatal)?;
                        drop(db);
                        super::publish(&self.core, &job.id);
                        match fallbacks.iter().find(|candidate| !tried.contains(candidate)) {
                            Some(next) => {
                                tried.push(next.clone());
                                harness = next.clone();
                            }
                            None => {
                                last_error = Some(StepError::Fatal(format!("{harness} could not review: {message}")));
                                break;
                            }
                        }
                    }
                    Err(error) => {
                        store::finish_pass(&db, &pass_id, ReviewPassStatus::Failed, Some(&error.message()), None, self.now())
                            .map_err(fatal)?;
                        drop(db);
                        super::publish(&self.core, &job.id);
                        last_error = Some(error);
                        break;
                    }
                }
            }
        }
        if finished == 0 {
            return Err(last_error.unwrap_or_else(|| StepError::Fatal("No reviewer finished".into())));
        }
        let db = self.core.db.lock().unwrap();
        store::set_summary(&db, &job.id, &summaries.join("\n\n")).map_err(fatal)?;
        Ok(())
    }

    /// Post, after looking for a review this job already posted.
    fn post(&self, job: &JobRow, steps: &dyn ReviewSteps, token: &CancelToken) -> Result<(), StepError> {
        let posted = self.retrying(token, || match steps.find_posted(job)? {
            Some(posted) => Ok(posted),
            None => steps.post(job, token),
        })?;
        let marked = {
            let db = self.core.db.lock().unwrap();
            store::mark_posted(&db, &job.id, &self.owner, &posted.review_id, &posted.url, self.now()).map_err(fatal)?
        };
        if !marked {
            return Err(StepError::Cancelled);
        }
        Ok(())
    }

    /// Write how the drive ended, clean up after a terminal job, and tell
    /// clients.
    fn settle(&self, job_id: &str, outcome: Result<(), StepError>) {
        let now = self.now();
        let settled = {
            let db = self.core.db.lock().unwrap();
            match &outcome {
                // Cancelled by a person (already written) or the lease was
                // lost (another runner owns the job): nothing to write.
                Ok(()) | Err(StepError::Cancelled) => Ok(()),
                Err(error) => store::get_job(&db, job_id).and_then(|job| match job {
                    // A post a person asked for failed: back to the draft,
                    // where they can post again.
                    Some(job) if job.status == Status::Posting && job.post_mode == ReviewPostMode::Draft => {
                        store::return_to_draft(&db, job_id, &self.owner, &error.message()).map(|_| ())
                    }
                    _ => store::finish(&db, job_id, Some(&self.owner), Status::Failed, Some(&error.message()), now).map(|_| ()),
                }),
            }
        };
        if let Err(error) = settled {
            eprintln!("[reviews] could not settle {job_id}: {error}");
        }
        let terminal = {
            let db = self.core.db.lock().unwrap();
            store::get_job(&db, job_id).ok().flatten().is_some_and(|job| job.status.is_terminal())
        };
        if terminal {
            self.core.reviews.steps().cleanup(job_id);
        }
        super::publish(&self.core, job_id);
    }
}

fn fatal(error: BridgeError) -> StepError {
    StepError::Fatal(error.to_string())
}

#[cfg(test)]
#[path = "runner_tests.rs"]
mod tests;
