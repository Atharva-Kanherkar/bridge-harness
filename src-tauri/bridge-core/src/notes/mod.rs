//! Bridge Notes: a local-first notes store that people and agents both write.
//!
//! Every page is a `yrs` document whose changes are appended to a log in
//! `notes.db`, mirrored to a Markdown file. People capture and edit through
//! the `notes/*` protocol methods; agents use the same write path through the
//! notes tools on Bridge's tool server. Model passes (Structure, Turn into
//! tasks) run only when someone asks. See issue #865 and
//! `testing/feat-notes-backend.md`.

pub mod doc;
pub mod guards;
pub mod ids;
pub mod jobs;
pub mod markdown;
pub mod projector;
pub mod service;
pub mod store;
pub mod structure;
pub mod tasks;
