//! The volatile half of Bridge's application context, and when a provider
//! thread is owed it.
//!
//! Two things Bridge must tell a harness change for reasons that have nothing
//! to do with the conversation: the credential-proxy capability contract
//! (`credential_broker::instructions`, whose token is two fresh UUIDs on every
//! Bridge process start) and the memory packet (re-ranked on every memory
//! edit). Both used to be variable sections of the compiled prompt, which is
//! Claude's `systemPrompt.append`, Codex's `developerInstructions` and
//! OpenCode's `system` — i.e. inside the block that precedes every message in
//! a prefix cache. A few changed bytes there re-write the whole conversation.
//!
//! So they are delivered here instead: one Bridge-authored frame in the
//! conversation *tail*, beside the user's message and never folded into it.
//! The compiled prompt keeps only what is genuinely fixed for the launch, and
//! stays byte-identical across restarts.

use crate::prompt_compiler::canonical_text;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::HashMap;

pub const SESSION_CONTEXT_SCHEMA_VERSION: u32 = 1;

/// One rendered frame plus the digest the delivery ledger compares on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionContext {
    text: String,
    digest: String,
}

impl SessionContext {
    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn digest(&self) -> &str {
        &self.digest
    }
}

#[derive(Serialize)]
struct NamedText<'a> {
    name: &'a str,
    text: &'a str,
}

#[derive(Serialize)]
struct Envelope<'a> {
    sections: &'a [NamedText<'a>],
}

/// Render the frame for a launch. Section order is fixed by construction, an
/// absent or blank section is omitted rather than emitted empty, and nothing
/// to say is `None` rather than an empty frame.
pub fn build(capabilities: &str, memory_packet: Option<&str>) -> Option<SessionContext> {
    let capabilities = canonical_text(capabilities);
    let memory_packet = memory_packet.map(canonical_text).unwrap_or_default();
    let mut sections = Vec::with_capacity(2);
    if !capabilities.is_empty() {
        sections.push(NamedText {
            name: "session_capabilities",
            text: &capabilities,
        });
    }
    if !memory_packet.is_empty() {
        sections.push(NamedText {
            name: "memory_packet",
            text: &memory_packet,
        });
    }
    if sections.is_empty() {
        return None;
    }
    // `serde_json` cannot fail on a struct of borrowed strings, but a panic in
    // the turn path would be a worse answer than skipping the frame.
    let body = serde_json::to_string(&Envelope {
        sections: &sections,
    })
    .ok()?;
    let text = format!(
        "<bridge-session-context schema=\"{SESSION_CONTEXT_SCHEMA_VERSION}\">\n{body}\n</bridge-session-context>"
    );
    let digest = Sha256::digest(text.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    Some(SessionContext { text, digest })
}

#[derive(Debug, Clone)]
struct Pending {
    provider_session_id: String,
    context: SessionContext,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Delivered {
    provider_session_id: String,
    digest: String,
}

#[derive(Debug, Default)]
struct Entry {
    /// Armed by a launch, owed to the next turn Bridge sends on this session.
    pending: Option<Pending>,
    /// What this session's provider thread has actually been handed.
    delivered: Option<Delivered>,
}

/// Which sessions are owed a frame and which already hold one.
///
/// In-memory on purpose: its lifetime is the Bridge process, which is exactly
/// the lifetime of the proxy token it exists to protect. A new process cannot
/// inherit a claim that a dead token was already delivered.
#[derive(Debug, Default)]
pub struct SessionContextLedger {
    entries: HashMap<String, Entry>,
}

impl SessionContextLedger {
    /// Record that a launch compiled `context` for `provider_session_id`, and
    /// owe it to the next turn — unless that exact frame is already in that
    /// exact thread's history, which is the same-process native resume case.
    pub fn arm(
        &mut self,
        session_id: &str,
        provider_session_id: &str,
        context: Option<SessionContext>,
    ) {
        let entry = self.entries.entry(session_id.to_owned()).or_default();
        let Some(context) = context else {
            // Nothing volatile to say. Leave the thread with whatever it holds
            // rather than claiming a frame was owed and then never sent.
            entry.pending = None;
            return;
        };
        let already_holds = entry.delivered.as_ref().is_some_and(|delivered| {
            delivered.provider_session_id == provider_session_id
                && delivered.digest == context.digest
        });
        entry.pending = (!already_holds).then_some(Pending {
            provider_session_id: provider_session_id.to_owned(),
            context,
        });
    }

    /// The frame the next turn on this session must carry, if any.
    pub fn pending(&self, session_id: &str) -> Option<SessionContext> {
        self.entries
            .get(session_id)?
            .pending
            .as_ref()
            .map(|pending| pending.context.clone())
    }

    /// Called once the turn carrying the frame actually reached the provider.
    ///
    /// Acknowledges one specific frame, by digest. A caller reads the pending
    /// frame, releases the lock, and only then sends — so between those two
    /// points a relaunch can arm a *newer* frame for a *different* thread.
    /// Acknowledging whatever happens to be pending would consume that newer
    /// frame on the strength of having sent the older one, and the new thread
    /// would silently never receive its proxy token or memory packet. A digest
    /// that no longer matches means the frame this caller sent is stale, so
    /// the acknowledgement is dropped and the newer one stays owed.
    pub fn record_delivered(&mut self, session_id: &str, digest: &str) {
        let Some(entry) = self.entries.get_mut(session_id) else {
            return;
        };
        if entry.pending.as_ref().map(|pending| pending.context.digest()) != Some(digest) {
            return;
        }
        if let Some(pending) = entry.pending.take() {
            entry.delivered = Some(Delivered {
                provider_session_id: pending.provider_session_id,
                digest: pending.context.digest,
            });
        }
    }

    /// The conversation is gone (`/clear`), so nothing Bridge believes it
    /// delivered survives either.
    pub fn forget(&mut self, session_id: &str) {
        self.entries.remove(session_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CAPABILITIES: &str = "Authorize the call with the request header `x-bridge-proxy-auth: aaaa`.";
    const OTHER_CAPABILITIES: &str =
        "Authorize the call with the request header `x-bridge-proxy-auth: bbbb`.";

    #[test]
    fn the_frame_is_deterministic_and_names_both_sections() {
        let first = build(CAPABILITIES, Some("Prefers tabs")).unwrap();
        let second = build(CAPABILITIES, Some("Prefers tabs")).unwrap();
        assert_eq!(first, second);
        assert!(first.text().starts_with("<bridge-session-context schema=\"1\">"));
        assert!(first.text().ends_with("</bridge-session-context>"));
        // Capabilities first, then memory: order is fixed by construction, not
        // by whatever the caller happened to pass.
        let capabilities_at = first.text().find("session_capabilities").unwrap();
        let memory_at = first.text().find("memory_packet").unwrap();
        assert!(capabilities_at < memory_at);
        assert!(first.text().contains("x-bridge-proxy-auth"));
        assert!(first.text().contains("Prefers tabs"));
    }

    #[test]
    fn the_digest_moves_with_either_volatile_half() {
        let base = build(CAPABILITIES, Some("Prefers tabs")).unwrap();
        // A restarted Bridge process regenerates the proxy token. This is the
        // byte change that used to re-write the provider's whole prefix.
        let rotated = build(OTHER_CAPABILITIES, Some("Prefers tabs")).unwrap();
        // An edited memory record re-ranks the packet.
        let edited = build(CAPABILITIES, Some("Prefers spaces")).unwrap();
        assert_ne!(base.digest(), rotated.digest());
        assert_ne!(base.digest(), edited.digest());
    }

    #[test]
    fn an_absent_or_blank_section_is_omitted_not_emitted_empty() {
        let no_packet = build(CAPABILITIES, None).unwrap();
        assert!(!no_packet.text().contains("memory_packet"));
        assert_eq!(no_packet, build(CAPABILITIES, Some("   \n ")).unwrap());

        let packet_only = build("", Some("Prefers tabs")).unwrap();
        assert!(!packet_only.text().contains("session_capabilities"));

        assert!(build("", None).is_none());
        assert!(build("  ", Some("")).is_none());
    }

    #[test]
    fn a_fresh_session_is_owed_the_frame() {
        let mut ledger = SessionContextLedger::default();
        assert!(ledger.pending("chat").is_none());
        let context = build(CAPABILITIES, Some("Prefers tabs")).unwrap();
        ledger.arm("chat", "thread-a", Some(context.clone()));
        assert_eq!(ledger.pending("chat").as_ref(), Some(&context));
        ledger.record_delivered("chat", context.digest());
        assert!(ledger.pending("chat").is_none());
    }

    #[test]
    fn a_same_thread_relaunch_with_the_same_frame_owes_nothing() {
        let mut ledger = SessionContextLedger::default();
        let context = build(CAPABILITIES, Some("Prefers tabs")).unwrap();
        ledger.arm("chat", "thread-a", Some(context.clone()));
        ledger.record_delivered("chat", context.digest());

        // The slice-2 model switch: same process, same provider thread, so the
        // frame is still in the provider's history and still valid.
        ledger.arm("chat", "thread-a", Some(context.clone()));
        assert!(ledger.pending("chat").is_none());

        // A fresh thread cannot hold it, however unchanged the bytes are.
        ledger.arm("chat", "thread-b", Some(context.clone()));
        assert_eq!(ledger.pending("chat").as_ref(), Some(&context));
    }

    #[test]
    fn a_changed_frame_is_owed_again_on_the_same_thread() {
        let mut ledger = SessionContextLedger::default();
        ledger.arm(
            "chat",
            "thread-a",
            build(CAPABILITIES, Some("Prefers tabs")),
        );
        let first = build(CAPABILITIES, Some("Prefers tabs")).unwrap();
        ledger.record_delivered("chat", first.digest());

        let rotated = build(OTHER_CAPABILITIES, Some("Prefers tabs")).unwrap();
        ledger.arm("chat", "thread-a", Some(rotated.clone()));
        assert_eq!(ledger.pending("chat").as_ref(), Some(&rotated));
    }

    #[test]
    fn nothing_to_say_leaves_the_thread_alone_and_clear_forgets_it() {
        let mut ledger = SessionContextLedger::default();
        let context = build(CAPABILITIES, Some("Prefers tabs")).unwrap();
        ledger.arm("chat", "thread-a", Some(context.clone()));
        ledger.arm("chat", "thread-a", None);
        assert!(ledger.pending("chat").is_none());

        ledger.arm("chat", "thread-a", Some(context.clone()));
        ledger.record_delivered("chat", context.digest());
        ledger.forget("chat");
        // Forgotten means forgotten: the next launch owes the frame again.
        ledger.arm("chat", "thread-a", Some(context.clone()));
        assert_eq!(ledger.pending("chat").as_ref(), Some(&context));
    }

    #[test]
    fn record_delivered_without_a_pending_frame_is_a_no_op() {
        let mut ledger = SessionContextLedger::default();
        ledger.record_delivered("unknown", "digest");
        assert!(ledger.pending("unknown").is_none());
    }

    /// A caller reads the pending frame, releases the lock, then sends. A
    /// relaunch in that window arms a newer frame for a newer thread, and the
    /// late acknowledgement must not consume it — otherwise the new thread
    /// never receives its proxy token or memory packet.
    #[test]
    fn a_late_acknowledgement_cannot_consume_a_newer_frame() {
        let mut ledger = SessionContextLedger::default();
        let old = build(CAPABILITIES, Some("Prefers tabs")).unwrap();
        ledger.arm("chat", "thread-a", Some(old.clone()));
        // The sender snapshots `old` here and releases the lock.

        let new = build(OTHER_CAPABILITIES, Some("Prefers spaces")).unwrap();
        ledger.arm("chat", "thread-b", Some(new.clone()));

        // The in-flight send finally lands and acknowledges what it sent.
        ledger.record_delivered("chat", old.digest());
        assert_eq!(
            ledger.pending("chat").as_ref(),
            Some(&new),
            "the newer frame is still owed"
        );

        // The newer send acknowledges its own frame and clears it.
        ledger.record_delivered("chat", new.digest());
        assert!(ledger.pending("chat").is_none());
    }
}
