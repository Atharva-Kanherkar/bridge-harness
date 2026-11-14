//! Apps: the features Bridge shows through the launcher, the dock, the sidebar
//! pins and chat cards, each described by a manifest that core owns.
//!
//! One registry decides what every client and every harness sees of an app:
//! its entry points, its agent tools and their prompt text, its badge, and
//! what memory may keep from it. Everything an app contributes hangs off its
//! registry entry, so turning the app off removes all of it together and no
//! surface keeps its own on/off logic.
//!
//! A manifest can only narrow. It can make a tool ask first; nothing in it can
//! grant a permission, widen a scope or skip an approval. Privileges a
//! built-in has (its badge source, whether it can be turned off, its prompt
//! text) live in [`builtin`], never in a manifest.

pub mod badges;
pub mod builtin;
pub mod manifest;
pub mod registry;

use rusqlite::Connection;

use crate::attention::AttentionKind;
use crate::BridgeError;

pub use builtin::{builtin, builtins, BadgeSource, BuiltinApp};

/// `bridge` server tools that belong to Bridge itself, not to an app. They
/// cannot be turned off. `(name, asks_first)`.
pub const CORE_TOOLS: [(&str, bool); 4] = [
    ("visualize", false),
    ("settings_read", false),
    ("settings_change", false),
    ("notify_user", true),
];

/// The app an attention kind's badge belongs to. Every current kind is about
/// one chat and shows on the chat list, which is not an app, so all map to
/// `None`. The match is exhaustive on purpose: a new kind must be classified
/// here before it compiles.
pub fn attention_owner(kind: AttentionKind) -> Option<&'static str> {
    match kind {
        AttentionKind::Finished
        | AttentionKind::Failed
        | AttentionKind::NeedsYou
        | AttentionKind::Escalation
        | AttentionKind::Posted => None,
    }
}

/// The built-in app that owns a `bridge` server tool, or `None` for a core
/// tool or a name no app declares.
pub fn tool_owner(tool: &str) -> Option<&'static BuiltinApp> {
    builtins().iter().find(|app| app.manifest.tools.iter().any(|owned| owned.name == tool))
}

/// Whether `tool` may be listed and called right now: a core tool, or a tool
/// whose owning app is on. A name nobody owns is left to the route, which
/// answers it as it always has.
pub fn tool_allowed(db: &Connection, tool: &str) -> Result<bool, BridgeError> {
    match tool_owner(tool) {
        Some(app) => registry::is_enabled(db, &app.manifest.id),
        None => Ok(true),
    }
}

/// `tools` without those whose owning app is off.
pub fn filter_tools<'a>(db: &Connection, tools: Vec<&'a str>) -> Result<Vec<&'a str>, BridgeError> {
    let off = registry::disabled_ids(db)?;
    Ok(tools
        .into_iter()
        .filter(|tool| tool_owner(tool).is_none_or(|app| !off.contains(&app.manifest.id)))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attention_owner_covers_every_kind() {
        for kind in [
            AttentionKind::Finished,
            AttentionKind::Failed,
            AttentionKind::NeedsYou,
            AttentionKind::Escalation,
            AttentionKind::Posted,
        ] {
            assert_eq!(attention_owner(kind), None, "{} shows on the chat list, not an app", kind.as_str());
        }
    }

    #[test]
    fn tools_belong_to_their_apps_and_core_tools_to_none() {
        assert_eq!(tool_owner("notes_create").unwrap().manifest.id, "notes");
        assert_eq!(tool_owner("memory_propose").unwrap().manifest.id, "memory");
        assert_eq!(tool_owner("outgoing_draft").unwrap().manifest.id, "outgoing");
        for (core_tool, _) in CORE_TOOLS {
            assert!(tool_owner(core_tool).is_none(), "{core_tool} is a core tool");
        }
    }
}
