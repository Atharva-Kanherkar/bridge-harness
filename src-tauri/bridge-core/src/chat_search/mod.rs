//! Find a chat from a vague memory, across every chat.
//!
//! A funnel, cheapest stage first:
//!
//! - **T0** [`parse`]: time phrases, harness words, phrases and terms, ranked
//!   by rarity from the FTS vocabulary. No model.
//! - **T1** [`retrieve`]: one global MATCH over entries and one over the
//!   per-chat [`index`] digests, fused per chat. Tens of milliseconds, no
//!   model, and most queries stop here.
//! - **T2** [`agent`]: only when the index is unsure and the caller asked. A
//!   hidden, tool-free Claude turn sees small cards, may ask Bridge for three
//!   bounded lookups ([`tools`]), and names chats it was shown. Every budget is
//!   enforced here, not by the prompt.
//!
//! Search is read-only end to end. It asks the policy engine for nothing, and
//! a result widens nothing: opening a hit is an ordinary navigation.

pub mod agent;
pub mod index;
pub mod parse;
pub mod retrieve;
pub mod settings;
pub mod tools;

#[cfg(test)]
mod eval;

/// The hidden session a deep search runs its model turns in.
pub const CHAT_SEARCH_SESSION_KIND: &str = "chat_search";
