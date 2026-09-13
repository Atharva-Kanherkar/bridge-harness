//! Which connector families a harness can actually reach, and what a rendered
//! notification card is allowed to contain.
//!
//! Bridge is a manager, not an MCP host. It owns no connector credential, runs
//! no MCP client, and performs no OAuth. Everything here is derived from two
//! things Bridge already has: the health map it parses out of the harness's own
//! `mcp list`, and the family resolvers `work_connectors` already reviewed.
//!
//! The card model is the other half. A notification card is *harness-generated
//! content*, which means it is the output of a model that just read
//! attacker-controllable text. So the card is not markup — it is a small, closed
//! set of typed blocks with bounded lengths, validated on the way in. Bridge
//! draws it with its own components. There is no path by which a Slack message
//! becomes markup, script, or a button that does something the user did not read.

use serde::{Deserialize, Serialize};

use crate::work_connectors::ConnectorFamily;

/// Longest single text run a card block may carry. Long enough for a real Slack
/// message, short enough that a pathological one cannot dominate the pane.
pub const MAX_BLOCK_TEXT: usize = 2_000;
/// Most blocks one card may hold.
pub const MAX_CARD_BLOCKS: usize = 24;
/// Most suggested replies a card may offer. They are drafts the user edits, not
/// actions, but an unbounded list is still a UI denial-of-service.
pub const MAX_SUGGESTED_REPLIES: usize = 3;
/// Longest suggested reply. Anything longer is the model writing an essay into
/// a one-line composer.
pub const MAX_REPLY_TEXT: usize = 500;

/// Why a connector family is not on offer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum UnavailableReason {
    /// The harness lists the server but it needs the user to sign in. This is
    /// the one reason that is actionable in the harness, not in Bridge.
    AuthRequired,
    /// The harness lists the server and it failed to connect.
    Unreachable,
    /// No server for this family is configured in the harness at all.
    NotConfigured,
    /// Bridge has no deterministic evidence resolver for this family, so it
    /// cannot say where a result came from. `work_connectors` refuses to offer
    /// those, and so does this.
    NoResolver,
}

impl UnavailableReason {
    /// The line the pane shows. Written to tell the user where the fix lives,
    /// because for three of these four reasons the fix is not inside Bridge.
    pub fn explanation(&self, family: ConnectorFamily) -> String {
        let name = family.display_name();
        match self {
            Self::AuthRequired => {
                format!("{name} is configured but signed out. Sign in from your harness — Bridge never holds the credential.")
            }
            Self::Unreachable => {
                format!("{name} is configured but its MCP server failed to connect.")
            }
            Self::NotConfigured => {
                format!("No {name} MCP server is configured in this harness.")
            }
            // The registry's own sentence, because "unsupported" tells a user
            // nothing and the three not-yet families and the one never family
            // are genuinely different situations.
            Self::NoResolver => profile(family)
                .no_inbox_reason
                .unwrap_or("This connector has no in-app inbox in this build.")
                .to_owned(),
        }
    }
}

/// One connector family as the inbox sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorAvailability {
    pub family: ConnectorFamily,
    /// The MCP server name the harness knows this family by, when one exists.
    /// Carried because the connector run scopes its policy to this exact name.
    pub server: Option<String>,
    /// **Which harness holds this connection.** A connector run has to go to the
    /// harness whose own MCP configuration owns the credential — sending it to
    /// another one sends it somewhere that cannot see the account at all. This
    /// is how the run layer knows, rather than assuming.
    pub harness: Option<String>,
    pub available: bool,
    pub reason: Option<UnavailableReason>,
}

// ── The family registry ──────────────────────────────────────────────────────
//
// Everything that differs between one connector and the next lives in exactly
// one place: the [`profile`] table below. Nothing else in this feature matches
// on a family — not the prompts, not the API layer, not the UI. Adding a family
// is therefore filling in one row, not finding every `if slack` in the tree.
//
// That is the whole design constraint. The first version of this module had
// `matches!(self, Self::Slack)` scattered through it, and each one was a place a
// later family would silently do the wrong thing.
//
// See `docs/connector-families.md` for the step-by-step.

/// How one product names the things it can notify you about.
///
/// A DM in Slack, an email in Gmail, and an assigned issue in Linear are the
/// same *shape* — someone wants you — and completely different words. The
/// prompts and the UI both read these rather than saying "DM" everywhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InboxProfile {
    /// What the ingress run should go looking for, in this product's own terms.
    /// Interpolated into the prompt, so it reads as an instruction.
    pub attention_items: &'static str,
    /// What a one-to-one message is called here. Slack: "direct message".
    /// Gmail: "email".
    pub direct_label: &'static str,
    /// What being named in a shared space is called. Slack: "mention".
    /// Linear: "assignment".
    pub mention_label: &'static str,
    /// What a follow-up in an existing conversation is called.
    pub thread_label: &'static str,
    /// Where a conversation lives. Slack: "channel". Gmail: "mailbox".
    pub container_noun: &'static str,
    /// The verb for answering. Slack: "send". Linear: "comment".
    pub reply_verb: &'static str,
    /// How a lightweight acknowledgement works here, when one exists at all.
    /// `None` means the UI offers no react affordance for this family.
    pub reaction: Option<ReactionProfile>,
}

/// A one-click acknowledgement, for products that have such a thing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReactionProfile {
    /// What it is called. Slack: "reaction".
    pub noun: &'static str,
    /// The default the UI's one-click control sends. Slack: `eyes`.
    pub default_token: &'static str,
}

/// One family's full description.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConnectorProfile {
    pub family: ConnectorFamily,
    pub display_name: &'static str,
    /// `None` means this build has no in-app inbox for the family. That is a
    /// statement about Bridge, not about the product: it means nobody has
    /// written the row below yet, or — for GitHub — that routing it here would
    /// be worse than the surface it already has.
    pub inbox: Option<InboxProfile>,
    /// Why there is no inbox, when there is none. Shown to the user, so it has
    /// to say something truer than "unsupported".
    pub no_inbox_reason: Option<&'static str>,
}

/// The registry. **This is the extension point.**
pub fn profile(family: ConnectorFamily) -> ConnectorProfile {
    match family {
        ConnectorFamily::Slack => ConnectorProfile {
            family,
            display_name: "Slack",
            inbox: Some(InboxProfile {
                attention_items:
                    "direct messages, @-mentions of the account owner, and replies in threads the owner is part of",
                direct_label: "direct message",
                mention_label: "mention",
                thread_label: "thread reply",
                container_noun: "channel",
                reply_verb: "send",
                reaction: Some(ReactionProfile { noun: "reaction", default_token: "eyes" }),
            }),
            no_inbox_reason: None,
        },
        ConnectorFamily::Gmail => ConnectorProfile {
            family,
            display_name: "Gmail",
            inbox: None,
            no_inbox_reason: Some("Gmail has no in-app inbox yet — its ingress query and card template are not written."),
        },
        ConnectorFamily::Linear => ConnectorProfile {
            family,
            display_name: "Linear",
            inbox: None,
            no_inbox_reason: Some("Linear has no in-app inbox yet — its ingress query and card template are not written."),
        },
        ConnectorFamily::Notion => ConnectorProfile {
            family,
            display_name: "Notion",
            inbox: None,
            no_inbox_reason: Some("Notion has no in-app inbox yet — its ingress query and card template are not written."),
        },
        // Deliberately permanent, unlike the three above. GitHub has a native
        // `gh`-backed surface that needs no model turn; routing it through a
        // harness turn would be a strictly worse version of something Bridge
        // already does well.
        ConnectorFamily::GitHub => ConnectorProfile {
            family,
            display_name: "GitHub",
            inbox: None,
            no_inbox_reason: Some("GitHub is served by Bridge's own pull-request surface, which needs no model turn."),
        },
    }
}

impl ConnectorFamily {
    pub fn display_name(self) -> &'static str {
        profile(self).display_name
    }

    /// The inbox description, when this family has one.
    pub fn inbox(self) -> Option<InboxProfile> {
        profile(self).inbox
    }

    /// Whether this build can render and act on the family. A family reaches the
    /// inbox when its registry row describes one — not merely because an MCP
    /// server for it exists.
    pub fn has_inbox_support(self) -> bool {
        profile(self).inbox.is_some()
    }

    /// The label for one item kind, in this family's vocabulary.
    pub fn kind_label(self, kind: ItemKind) -> &'static str {
        match self.inbox() {
            Some(inbox) => match kind {
                ItemKind::DirectMessage => inbox.direct_label,
                ItemKind::Mention => inbox.mention_label,
                ItemKind::ThreadReply => inbox.thread_label,
            },
            // Unreachable for a family with an inbox; a neutral word beats a
            // panic for one without.
            None => "message",
        }
    }

    /// Match an MCP server name to a family. Matching is on a word-ish boundary
    /// rather than a bare substring so `claude.ai Slack` resolves and a server
    /// merely *mentioning* a family in a longer word does not.
    pub fn from_server_name(server: &str) -> Option<Self> {
        let lowered = server.to_ascii_lowercase();
        Self::ALL.into_iter().find(|family| {
            let needle = family.as_str();
            lowered.split(|c: char| !c.is_ascii_alphanumeric()).any(|word| word == needle)
        })
    }
}

/// One harness's view of what it has connected.
///
/// A list rather than a single map because connectors do not all live in one
/// harness: Slack may be a claude.ai connector while a Linear server is
/// configured in Codex. Resolution takes every harness's view at once so the
/// answer names *which* harness owns each connection.
#[derive(Debug, Clone)]
pub struct HarnessConnectors {
    pub harness: String,
    /// Server name → `Some(true)` connected, `Some(false)` failed or signed out,
    /// `None` no verdict. Exactly what `marketplace::claude_sdk_configuration`
    /// already parses out of `mcp list`.
    pub health: std::collections::BTreeMap<String, Option<bool>>,
}

/// Resolve every family's availability across every harness.
///
/// Performs no I/O and no model turn — knowing *which* connectors exist must
/// never cost a token. Where two harnesses both report a family, a connected one
/// wins over a signed-out one; ties break on harness name so the answer is
/// stable rather than dependent on iteration order.
pub fn resolve_availability(harnesses: &[HarnessConnectors]) -> Vec<ConnectorAvailability> {
    ConnectorFamily::ALL
        .into_iter()
        .map(|family| {
            if !family.has_inbox_support() {
                return ConnectorAvailability {
                    family,
                    server: None,
                    harness: None,
                    available: false,
                    reason: Some(UnavailableReason::NoResolver),
                };
            }
            // Every (harness, server) pair claiming this family, best first:
            // connected over not, then stable by harness and server name.
            let mut matched: Vec<(&str, &str, Option<bool>)> = harnesses
                .iter()
                .flat_map(|entry| {
                    entry.health.iter().map(move |(server, verdict)| {
                        (entry.harness.as_str(), server.as_str(), *verdict)
                    })
                })
                .filter(|(_, server, _)| ConnectorFamily::from_server_name(server) == Some(family))
                .collect();
            matched.sort_by_key(|(harness, server, verdict)| {
                (*verdict != Some(true), *harness, *server)
            });
            match matched.first() {
                Some((harness, server, verdict)) => ConnectorAvailability {
                    family,
                    server: Some((*server).to_owned()),
                    harness: Some((*harness).to_owned()),
                    available: *verdict == Some(true),
                    reason: match verdict {
                        Some(true) => None,
                        Some(false) => Some(UnavailableReason::AuthRequired),
                        None => Some(UnavailableReason::Unreachable),
                    },
                },
                None => ConnectorAvailability {
                    family,
                    server: None,
                    harness: None,
                    available: false,
                    reason: Some(UnavailableReason::NotConfigured),
                },
            }
        })
        .collect()
}

// ── The inbox item ───────────────────────────────────────────────────────────

/// What kind of attention an item wants. The distinction is the whole point of a
/// notification surface: a DM is a question aimed at you, a mention is a room
/// pulling you in, a thread reply is a conversation you are already inside.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemKind {
    DirectMessage,
    Mention,
    ThreadReply,
}

/// One unread thing, as the ingress run reported it.
///
/// Every field here is *structured* — an id, a name, a timestamp, a body. None
/// of it is model prose about the message, so Bridge can always author its own
/// fallback card from these fields when a render run fails or misbehaves.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InboxItem {
    pub family: ConnectorFamily,
    pub channel_id: String,
    /// Human label: `#eng-alerts`, or a DM partner's display name.
    pub channel_label: String,
    pub message_ts: String,
    pub author: String,
    pub kind: ItemKind,
    /// The message body. **Untrusted.** Displayed, fenced in prompts, never obeyed.
    pub text: String,
    pub permalink: Option<String>,
    pub received_at: String,
}

impl InboxItem {
    /// The identity an item is deduplicated by, forever. Channel plus Slack's
    /// own message timestamp is unique per workspace and stable across re-reads,
    /// which is what makes "announce once" survive a restart.
    pub fn key(&self) -> String {
        format!("{}:{}:{}", self.family.as_str(), self.channel_id, self.message_ts)
    }
}

// ── The card ─────────────────────────────────────────────────────────────────

/// One renderable block. A closed set — the harness picks from these, it does
/// not author markup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum CardBlock {
    /// The message itself, or a quoted earlier one.
    Message { author: String, text: String, timestamp: Option<String> },
    /// One line of Bridge-side framing: "3 earlier replies in this thread".
    Context { text: String },
    /// The model's read of what is being asked. Labelled as a summary in the UI
    /// so it is never mistaken for something the sender wrote.
    Summary { text: String },
    /// A short labelled value — "Due", "Priority", "Channel".
    Fact { label: String, value: String },
}

impl CardBlock {
    fn validate(&self) -> Result<(), CardRejection> {
        let too_long = |text: &String| text.chars().count() > MAX_BLOCK_TEXT;
        match self {
            Self::Message { author, text, .. } => {
                if author.trim().is_empty() {
                    return Err(CardRejection::EmptyField { field: "message.author" });
                }
                if too_long(text) {
                    return Err(CardRejection::TooLong { field: "message.text" });
                }
            }
            Self::Context { text } | Self::Summary { text } => {
                if text.trim().is_empty() {
                    return Err(CardRejection::EmptyField { field: "text" });
                }
                if too_long(text) {
                    return Err(CardRejection::TooLong { field: "text" });
                }
            }
            Self::Fact { label, value } => {
                if label.trim().is_empty() {
                    return Err(CardRejection::EmptyField { field: "fact.label" });
                }
                if too_long(value) {
                    return Err(CardRejection::TooLong { field: "fact.value" });
                }
            }
        }
        Ok(())
    }
}

/// Why a harness-emitted card was refused. Every variant ends the same way: the
/// item falls back to a card Bridge authored from the structured item fields, so
/// a bad render degrades the presentation and never the notification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum CardRejection {
    NotJson,
    EmptyField { field: &'static str },
    TooLong { field: &'static str },
    TooManyBlocks { count: usize },
    NoBlocks,
    TooManyReplies { count: usize },
    /// The card claims to be about an item other than the one that was asked
    /// for. A render run is scoped to exactly one item; anything else is drift.
    WrongItem { expected: String, got: String },
}

impl CardRejection {
    pub fn detail(&self) -> String {
        match self {
            Self::NotJson => "the render run did not return a JSON card".into(),
            Self::EmptyField { field } => format!("`{field}` was empty"),
            Self::TooLong { field } => format!("`{field}` exceeded the card length budget"),
            Self::TooManyBlocks { count } => {
                format!("{count} blocks exceeds the {MAX_CARD_BLOCKS}-block budget")
            }
            Self::NoBlocks => "the card had no blocks".into(),
            Self::TooManyReplies { count } => {
                format!("{count} suggested replies exceeds the {MAX_SUGGESTED_REPLIES} allowed")
            }
            Self::WrongItem { expected, got } => {
                format!("the card was about `{got}`, not the requested `{expected}`")
            }
        }
    }
}

/// A rendered notification card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorCard {
    /// The item this card renders. Checked against the request.
    pub item_key: String,
    /// One-line headline. Shown in the toast and at the top of the pane.
    pub headline: String,
    pub blocks: Vec<CardBlock>,
    /// Drafts, not actions. Clicking one fills the composer; sending still needs
    /// the same approval every other reply needs.
    pub suggested_replies: Vec<String>,
    /// False when Bridge authored this card itself after rejecting the run's.
    pub harness_rendered: bool,
}

impl ConnectorCard {
    /// Validate a harness-emitted card against the requested item.
    pub fn validate(self, expected_key: &str) -> Result<Self, CardRejection> {
        if self.item_key != expected_key {
            return Err(CardRejection::WrongItem {
                expected: expected_key.to_owned(),
                got: self.item_key.clone(),
            });
        }
        if self.headline.trim().is_empty() {
            return Err(CardRejection::EmptyField { field: "headline" });
        }
        if self.headline.chars().count() > MAX_BLOCK_TEXT {
            return Err(CardRejection::TooLong { field: "headline" });
        }
        if self.blocks.is_empty() {
            return Err(CardRejection::NoBlocks);
        }
        if self.blocks.len() > MAX_CARD_BLOCKS {
            return Err(CardRejection::TooManyBlocks { count: self.blocks.len() });
        }
        for block in &self.blocks {
            block.validate()?;
        }
        if self.suggested_replies.len() > MAX_SUGGESTED_REPLIES {
            return Err(CardRejection::TooManyReplies { count: self.suggested_replies.len() });
        }
        for reply in &self.suggested_replies {
            if reply.trim().is_empty() {
                return Err(CardRejection::EmptyField { field: "suggestedReplies[]" });
            }
            if reply.chars().count() > MAX_REPLY_TEXT {
                return Err(CardRejection::TooLong { field: "suggestedReplies[]" });
            }
        }
        Ok(self)
    }

    /// The card Bridge draws when it will not use the harness's.
    ///
    /// Built only from the structured ingress fields — author, channel, body —
    /// so it carries no model prose and cannot itself be the thing that was
    /// wrong. A notification always arrives; only its polish is contingent.
    pub fn fallback(item: &InboxItem) -> Self {
        let headline = match item.kind {
            ItemKind::DirectMessage => format!("{} sent you a direct message", item.author),
            ItemKind::Mention => format!("{} mentioned you in {}", item.author, item.channel_label),
            ItemKind::ThreadReply => format!("{} replied in {}", item.author, item.channel_label),
        };
        Self {
            item_key: item.key(),
            headline,
            blocks: vec![
                CardBlock::Message {
                    author: item.author.clone(),
                    text: truncate(&item.text, MAX_BLOCK_TEXT),
                    timestamp: Some(item.received_at.clone()),
                },
                CardBlock::Fact { label: "Channel".into(), value: item.channel_label.clone() },
            ],
            suggested_replies: Vec::new(),
            harness_rendered: false,
        }
    }
}

fn truncate(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_owned();
    }
    text.chars().take(limit.saturating_sub(1)).collect::<String>() + "…"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn health(entries: &[(&str, Option<bool>)]) -> Vec<HarnessConnectors> {
        vec![HarnessConnectors {
            harness: "claude".into(),
            health: entries.iter().map(|(name, verdict)| ((*name).to_owned(), *verdict)).collect(),
        }]
    }

    fn on(harness: &str, entries: &[(&str, Option<bool>)]) -> HarnessConnectors {
        HarnessConnectors {
            harness: harness.into(),
            health: entries.iter().map(|(name, verdict)| ((*name).to_owned(), *verdict)).collect(),
        }
    }

    fn slack(availability: &[ConnectorAvailability]) -> &ConnectorAvailability {
        availability.iter().find(|entry| entry.family == ConnectorFamily::Slack).unwrap()
    }

    fn item() -> InboxItem {
        InboxItem {
            family: ConnectorFamily::Slack,
            channel_id: "D0BV4LADFGB".into(),
            channel_label: "Nina Alvarez".into(),
            message_ts: "1757000000.000100".into(),
            author: "Nina Alvarez".into(),
            kind: ItemKind::DirectMessage,
            text: "can you take a look at the release checklist before standup?".into(),
            permalink: Some("https://app.slack.com/archives/D0BV4LADFGB/p1757000000000100".into()),
            received_at: "2026-09-13T09:14:00Z".into(),
        }
    }

    fn card() -> ConnectorCard {
        ConnectorCard {
            item_key: item().key(),
            headline: "Nina asked you to review the release checklist".into(),
            blocks: vec![CardBlock::Message {
                author: "Nina Alvarez".into(),
                text: "can you take a look at the release checklist before standup?".into(),
                timestamp: None,
            }],
            suggested_replies: vec!["On it — looking now.".into()],
            harness_rendered: true,
        }
    }

    #[test]
    fn slack_connected_in_mcp_list_is_an_available_family() {
        // Exactly the line `claude mcp list` prints for a live connector.
        let resolved = resolve_availability(&health(&[("claude.ai Slack", Some(true))]));
        let slack = slack(&resolved);
        assert!(slack.available);
        assert_eq!(slack.server.as_deref(), Some("claude.ai Slack"));
        assert_eq!(slack.harness.as_deref(), Some("claude"));
        assert_eq!(slack.reason, None);
    }

    #[test]
    fn needs_authentication_is_unavailable_with_auth_required() {
        let resolved = resolve_availability(&health(&[("claude.ai Slack", Some(false))]));
        let slack = slack(&resolved);
        assert!(!slack.available);
        assert_eq!(slack.reason, Some(UnavailableReason::AuthRequired));
        // The remediation has to point at the harness: Bridge holds no token to
        // refresh, so a "sign in here" affordance would be a lie.
        assert!(slack.reason.as_ref().unwrap().explanation(ConnectorFamily::Slack).contains("harness"));
    }

    #[test]
    fn a_family_without_a_resolver_is_never_offered() {
        // Even with a connected server, a family with no registry inbox row stays off.
        let resolved = resolve_availability(&health(&[("claude.ai Linear", Some(true))]));
        let linear = resolved.iter().find(|entry| entry.family == ConnectorFamily::Linear).unwrap();
        assert!(!linear.available);
        assert_eq!(linear.reason, Some(UnavailableReason::NoResolver));
        // And it says *why* in the registry's own words, because "not yet
        // written" and "served better elsewhere" are different situations.
        let explanation = linear.reason.as_ref().unwrap().explanation(ConnectorFamily::Linear);
        assert!(explanation.contains("not written"), "{explanation}");
    }

    #[test]
    fn a_connector_run_is_sent_to_the_harness_that_owns_the_connection() {
        // The premise of the feature: the credential lives in one harness's own
        // MCP configuration. Losing track of which one sends the run somewhere
        // that cannot see the account.
        let resolved = resolve_availability(&[
            on("codex", &[("acme-slack", Some(true))]),
            on("claude", &[]),
        ]);
        let slack = slack(&resolved);
        assert!(slack.available);
        assert_eq!(slack.harness.as_deref(), Some("codex"));
    }

    #[test]
    fn a_connected_harness_wins_over_one_that_is_signed_out() {
        let resolved = resolve_availability(&[
            on("claude", &[("claude.ai Slack", Some(false))]),
            on("codex", &[("acme-slack", Some(true))]),
        ]);
        let slack = slack(&resolved);
        assert!(slack.available);
        assert_eq!(slack.harness.as_deref(), Some("codex"), "the live one is the usable one");
    }

    #[test]
    fn resolution_is_stable_when_two_harnesses_are_equally_good() {
        // Iteration order must not decide which account a reply goes to.
        let first = resolve_availability(&[
            on("codex", &[("acme-slack", Some(true))]),
            on("claude", &[("claude.ai Slack", Some(true))]),
        ]);
        let second = resolve_availability(&[
            on("claude", &[("claude.ai Slack", Some(true))]),
            on("codex", &[("acme-slack", Some(true))]),
        ]);
        assert_eq!(slack(&first).harness, slack(&second).harness);
    }

    // ── The registry contract ───────────────────────────────────────────────
    // These are the tests that make the feature extensible rather than merely
    // extensible-looking: they fail when a new family is added without the rows
    // that make it actually work.

    /// The source files that make up this feature, minus the registry itself.
    ///
    /// Read as text on purpose. The point is not what these modules *do* — the
    /// other tests cover that — but that none of them has quietly grown a
    /// second place where one family is special. That is how a feature stops
    /// being extensible: not in one big decision, but in six small `if slack`s
    /// added by six people in a hurry.
    fn feature_sources() -> Vec<(&'static str, String)> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        ["connector_runs.rs", "connector_runs_live.rs", "connector_inbox.rs"]
            .into_iter()
            .map(|name| {
                let body = std::fs::read_to_string(root.join(name))
                    .unwrap_or_else(|error| panic!("{name} is unreadable: {error}"));
                // Everything from `mod tests` down is fixtures and assertions,
                // which name families legitimately and constantly.
                let production = body
                    .split_once("#[cfg(test)]")
                    .map(|(before, _)| before.to_owned())
                    .unwrap_or(body);
                (name, production)
            })
            .collect()
    }

    #[test]
    fn no_module_outside_the_registry_singles_out_one_family() {
        for (name, source) in feature_sources() {
            for family in ConnectorFamily::ALL {
                let variant = format!("ConnectorFamily::{family:?}");
                assert!(
                    !source.contains(&variant),
                    "{name} names {variant} in production code — that behaviour belongs in \
                     `connector_surface::profile` so every family gets it. See \
                     docs/connector-families.md.",
                );
            }
            // The string form is the same mistake wearing a different hat.
            for family in ConnectorFamily::ALL {
                let quoted = format!("\"{}\"", family.as_str());
                assert!(
                    !source.contains(&quoted),
                    "{name} hardcodes the family string {quoted} in production code",
                );
            }
        }
    }

    #[test]
    fn every_family_has_a_registry_row_that_describes_itself() {
        for family in ConnectorFamily::ALL {
            let entry = profile(family);
            assert_eq!(entry.family, family, "a row is filed under the wrong family");
            assert!(!entry.display_name.trim().is_empty(), "{family:?} has no display name");
            // Exactly one of the two must be present: a family either has an
            // inbox or owes the user a sentence about why it does not.
            assert_eq!(
                entry.inbox.is_some(),
                entry.no_inbox_reason.is_none(),
                "{family:?} must have either an inbox or a stated reason it has none",
            );
        }
    }

    #[test]
    fn every_inbox_row_is_fully_populated() {
        // A half-filled row produces prompts with empty words in them, which is
        // the failure mode this test exists to make loud.
        for family in ConnectorFamily::ALL.into_iter().filter(|family| family.has_inbox_support()) {
            let inbox = family.inbox().unwrap();
            for (label, value) in [
                ("attention_items", inbox.attention_items),
                ("direct_label", inbox.direct_label),
                ("mention_label", inbox.mention_label),
                ("thread_label", inbox.thread_label),
                ("container_noun", inbox.container_noun),
                ("reply_verb", inbox.reply_verb),
            ] {
                assert!(!value.trim().is_empty(), "{family:?}.{label} is empty");
            }
            if let Some(reaction) = inbox.reaction {
                assert!(!reaction.noun.trim().is_empty(), "{family:?} reaction noun is empty");
                assert!(!reaction.default_token.trim().is_empty(), "{family:?} reaction token is empty");
            }
            for kind in [ItemKind::DirectMessage, ItemKind::Mention, ItemKind::ThreadReply] {
                assert!(!family.kind_label(kind).trim().is_empty());
            }
        }
    }

    #[test]
    fn a_family_names_its_own_item_kinds() {
        assert_eq!(ConnectorFamily::Slack.kind_label(ItemKind::DirectMessage), "direct message");
        assert_eq!(ConnectorFamily::Slack.kind_label(ItemKind::Mention), "mention");
        // A family with no inbox still answers rather than panicking.
        assert!(!ConnectorFamily::Gmail.kind_label(ItemKind::Mention).is_empty());
    }

    #[test]
    fn github_is_excluded_because_it_has_a_native_surface() {
        let resolved = resolve_availability(&health(&[("claude.ai GitHub", Some(true))]));
        let github = resolved.iter().find(|entry| entry.family == ConnectorFamily::GitHub).unwrap();
        assert!(!github.available, "github goes through the gh-backed surface, not a model turn");
    }

    #[test]
    fn an_absent_server_is_not_configured_rather_than_signed_out() {
        let resolved = resolve_availability(&health(&[]));
        assert_eq!(slack(&resolved).reason, Some(UnavailableReason::NotConfigured));
    }

    #[test]
    fn a_connected_server_wins_over_a_signed_out_duplicate() {
        // Two servers for one family is normal: a claude.ai connector and a
        // plugin-provided one. A stale signed-out entry must not mask the live one.
        let resolved = resolve_availability(&health(&[
            ("plugin:acme:slack", Some(false)),
            ("claude.ai Slack", Some(true)),
        ]));
        let slack = slack(&resolved);
        assert!(slack.available);
        assert_eq!(slack.server.as_deref(), Some("claude.ai Slack"));
    }

    #[test]
    fn server_matching_is_on_word_boundaries() {
        assert_eq!(ConnectorFamily::from_server_name("claude.ai Slack"), Some(ConnectorFamily::Slack));
        assert_eq!(ConnectorFamily::from_server_name("plugin:acme:slack"), Some(ConnectorFamily::Slack));
        assert_eq!(ConnectorFamily::from_server_name("slackware-docs"), None);
    }

    #[test]
    fn an_item_key_is_stable_and_identifies_one_message() {
        assert_eq!(item().key(), "slack:D0BV4LADFGB:1757000000.000100");
        let mut other = item();
        other.message_ts = "1757000000.000200".into();
        assert_ne!(item().key(), other.key(), "two messages in one channel are two items");
        // Re-reading the same message must produce the same key, or "announce
        // once" degrades into "announce once per poll".
        assert_eq!(item().key(), item().key());
    }

    #[test]
    fn a_valid_card_survives_validation() {
        assert!(card().validate(&item().key()).is_ok());
    }

    #[test]
    fn a_card_about_another_item_is_rejected() {
        let mut drifted = card();
        drifted.item_key = "slack:C999:1.0".into();
        assert!(matches!(
            drifted.validate(&item().key()),
            Err(CardRejection::WrongItem { .. })
        ));
    }

    #[test]
    fn an_oversized_card_is_rejected_rather_than_truncated() {
        // Truncating would let a long message quietly reshape the pane; refusing
        // sends it down the fallback path, which is bounded by construction.
        let mut huge = card();
        huge.blocks = vec![CardBlock::Context { text: "x".repeat(MAX_BLOCK_TEXT + 1) }];
        assert!(matches!(huge.validate(&item().key()), Err(CardRejection::TooLong { .. })));

        let mut many = card();
        many.blocks = (0..MAX_CARD_BLOCKS + 1)
            .map(|index| CardBlock::Context { text: format!("block {index}") })
            .collect();
        assert!(matches!(many.validate(&item().key()), Err(CardRejection::TooManyBlocks { .. })));
    }

    #[test]
    fn an_empty_card_is_rejected() {
        let mut empty = card();
        empty.blocks.clear();
        assert!(matches!(empty.validate(&item().key()), Err(CardRejection::NoBlocks)));

        let mut headless = card();
        headless.headline = "   ".into();
        assert!(matches!(headless.validate(&item().key()), Err(CardRejection::EmptyField { .. })));
    }

    #[test]
    fn suggested_replies_are_bounded_in_count_and_length() {
        let mut chatty = card();
        chatty.suggested_replies = vec!["a".into(); MAX_SUGGESTED_REPLIES + 1];
        assert!(matches!(chatty.validate(&item().key()), Err(CardRejection::TooManyReplies { .. })));

        let mut essay = card();
        essay.suggested_replies = vec!["a".repeat(MAX_REPLY_TEXT + 1)];
        assert!(matches!(essay.validate(&item().key()), Err(CardRejection::TooLong { .. })));
    }

    #[test]
    fn the_fallback_card_is_built_only_from_structured_fields() {
        let fallback = ConnectorCard::fallback(&item());
        assert_eq!(fallback.item_key, item().key());
        assert!(!fallback.harness_rendered);
        assert!(fallback.suggested_replies.is_empty(), "Bridge does not invent replies");
        assert!(fallback.headline.contains("Nina Alvarez"));
        assert!(fallback.validate(&item().key()).is_ok(), "the fallback must itself be valid");
    }

    #[test]
    fn the_fallback_bounds_a_pathological_message_body() {
        let mut giant = item();
        giant.text = "x".repeat(MAX_BLOCK_TEXT * 3);
        let fallback = ConnectorCard::fallback(&giant);
        assert!(fallback.clone().validate(&giant.key()).is_ok());
        match &fallback.blocks[0] {
            CardBlock::Message { text, .. } => assert_eq!(text.chars().count(), MAX_BLOCK_TEXT),
            other => panic!("expected the message block, got {other:?}"),
        }
    }

    #[test]
    fn the_fallback_headline_distinguishes_the_three_item_kinds() {
        let mut mention = item();
        mention.kind = ItemKind::Mention;
        mention.channel_label = "#eng-alerts".into();
        assert!(ConnectorCard::fallback(&mention).headline.contains("#eng-alerts"));

        let mut reply = item();
        reply.kind = ItemKind::ThreadReply;
        assert!(ConnectorCard::fallback(&reply).headline.contains("replied"));
        assert!(ConnectorCard::fallback(&item()).headline.contains("direct message"));
    }
}
