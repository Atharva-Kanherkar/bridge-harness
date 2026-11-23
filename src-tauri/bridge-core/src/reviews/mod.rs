//! Pull-request reviews as stored background jobs.
//!
//! A review is a row in `review_jobs`, driven by Bridge's own runner with its
//! own slots, never a worker of a chat. See `state` for the job's lifecycle,
//! `store` for the durable guarantees, and `steps` for the work a job does.

pub mod state;
pub mod steps;
pub mod store;
