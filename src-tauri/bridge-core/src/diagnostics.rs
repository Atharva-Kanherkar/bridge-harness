//! Best-effort runtime notes on stderr.
//!
//! `eprintln!` panics when the write fails, and Bridge does not own the far end
//! of its stderr: under `bun run tauri dev` it is a pipe held by the dev server,
//! and under a Finder launch it can be closed outright. When that pipe breaks,
//! the write returns `EPIPE` and `eprintln!` panics on whatever thread happened
//! to be logging — including one holding a workspace serialization lock, which
//! poisons that lock for the rest of the process and turns every later session
//! start into a `PoisonError`. A diagnostic line is never worth a panic, so
//! every write here is best-effort and a failed one is dropped.

use std::io::Write;

/// Write one diagnostic line to stderr, ignoring a closed or broken pipe.
pub fn record(message: &str) {
    let _ = writeln!(std::io::stderr().lock(), "{message}");
}
