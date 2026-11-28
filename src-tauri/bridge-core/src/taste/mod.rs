//! The taste layer: what a person keeps, edits and drops, learned once and
//! taught to every agent (#1196). This module holds its decision log; rules
//! live on the memory ledger with `layer = taste`.

pub mod replay;
pub mod signals;
pub mod suppress;
