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
            Self::NoResolver => {
                format!("Bridge cannot derive provenance for {name} results yet, so it will not surface them.")
            }
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
    pub available: bool,
    pub reason: Option<UnavailableReason>,
}

impl ConnectorFamily {
    pub fn display_name(self) -> &'static str {
        match self {
            Self::Slack => "Slack",
            Self::Gmail => "Gmail",
            Self::GitHub => "GitHub",
            Self::Linear => "Linear",
            Self::Notion => "Notion",
        }
    }

    /// Whether this slice can render and act on the family. Deliberately narrow:
    /// a family reaches the inbox only once someone writes its card template and
    /// its ingress query, not merely because an MCP server for it exists.
    ///
    /// GitHub is excluded on purpose and permanently — it has a native, `gh`-backed
    /// surface that needs no model turn, and routing it through here would be a
    /// strictly worse version of a thing Bridge already does well.
    pub fn has_inbox_support(self) -> bool {
        matches!(self, Self::Slack)
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

/// Resolve every family's availability from the harness's health map.
///
/// `health` is exactly what `marketplace::claude_sdk_configuration` already
/// parses: server name → `Some(true)` connected, `Some(false)` failed or signed
/// out, `None` no verdict. This function performs no I/O and no model turn —
/// knowing *which* connectors exist must never cost a token.
pub fn resolve_availability(
    health: &std::collections::BTreeMap<String, Option<bool>>,
) -> Vec<ConnectorAvailability> {
    ConnectorFamily::ALL
        .into_iter()
        .map(|family| {
            if !family.has_inbox_support() {
                return ConnectorAvailability {
                    family,
                    server: None,
                    available: false,
                    reason: Some(UnavailableReason::NoResolver),
                };
            }
            // Prefer a connected server when the harness lists several for one
            // family, so a stale signed-out duplicate cannot mask a live one.
            let mut matched: Vec<(&String, &Option<bool>)> = health
                .iter()
                .filter(|(server, _)| ConnectorFamily::from_server_name(server) == Some(family))
                .collect();
            matched.sort_by_key(|(server, verdict)| (**verdict != Some(true), (*server).clone()));
            match matched.first() {
                Some((server, Some(true))) => ConnectorAvailability {
                    family,
                    server: Some((*server).clone()),
                    available: true,
                    reason: None,
                },
                Some((server, Some(false))) => ConnectorAvailability {
                    family,
                    server: Some((*server).clone()),
                    available: false,
                    reason: Some(UnavailableReason::AuthRequired),
                },
                Some((server, None)) => ConnectorAvailability {
                    family,
                    server: Some((*server).clone()),
                    available: false,
                    reason: Some(UnavailableReason::Unreachable),
                },
                None => ConnectorAvailability {
                    family,
                    server: None,
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
    use std::collections::BTreeMap;

    fn health(entries: &[(&str, Option<bool>)]) -> BTreeMap<String, Option<bool>> {
        entries.iter().map(|(name, verdict)| ((*name).to_owned(), *verdict)).collect()
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
        // Even with a connected server, a family this slice cannot resolve stays off.
        let resolved = resolve_availability(&health(&[("claude.ai Linear", Some(true))]));
        let linear = resolved.iter().find(|entry| entry.family == ConnectorFamily::Linear).unwrap();
        assert!(!linear.available);
        assert_eq!(linear.reason, Some(UnavailableReason::NoResolver));
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
