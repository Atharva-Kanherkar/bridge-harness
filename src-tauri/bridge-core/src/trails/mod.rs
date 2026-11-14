//! Trails: one page per piece of work. See `docs/trails.md`.
//!
//! A trail joins records Bridge already keeps (chats, notes pages, pull
//! request links, drafted messages) without copying them. `store` owns the
//! rows, `timeline` derives what a page shows, `auto` follows hard links,
//! `brief` carries the trail into a new chat, and `live` runs the sweep when
//! the records move.

pub mod auto;
pub mod brief;
#[cfg(test)]
mod eval;
pub mod live;
pub mod store;
pub mod timeline;

pub use timeline::NoteSource;
