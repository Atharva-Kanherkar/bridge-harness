//! The durable inbox: what has arrived, what has been rendered, what is resolved.
//!
//! This module exists for one invariant — **a message is announced exactly once,
//! ever**. Polling ingress re-reads the same recent window every cycle, so without
//! a durable ledger every poll would re-announce the whole window, and a restart
//! would re-announce it again. Announcement is therefore a *write*: an item is new
//! if and only if inserting its key succeeded.
//!
//! What is stored is the structured envelope plus the message body, because the
//! pane has to show the message and the fallback card is built from it. What is
//! never stored is a credential or a token — Bridge has none to store.

use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;

use crate::connector_surface::{ConnectorCard, InboxItem, ItemKind};
use crate::work_connectors::ConnectorFamily;
use crate::BridgeError;

/// How far back ingress looks. Bounded so a first run on a busy workspace does
/// not announce a month of history as if it had all just happened.
pub const INGRESS_LOOKBACK_MINUTES: i64 = 60;

/// Where an item is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemState {
    /// Announced; no card yet. The toast is already up, showing Bridge's own text.
    Pending,
    /// A card is attached — harness-rendered or the fallback.
    Rendered,
    /// Dealt with: replied, reacted, or dismissed. Never re-announced, never re-rendered.
    Resolved,
}

impl ItemState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Rendered => "rendered",
            Self::Resolved => "resolved",
        }
    }

    fn parse(value: &str) -> Self {
        match value {
            "rendered" => Self::Rendered,
            "resolved" => Self::Resolved,
            _ => Self::Pending,
        }
    }
}

/// How an item stopped needing attention. Recorded because "I replied" and "I
/// dismissed it" are different facts about the same message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolution {
    Replied,
    Reacted,
    Dismissed,
}

impl Resolution {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Replied => "replied",
            Self::Reacted => "reacted",
            Self::Dismissed => "dismissed",
        }
    }
}

/// One item as the pane reads it.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredItem {
    pub item: InboxItem,
    pub state: ItemState,
    pub card: Option<ConnectorCard>,
    /// Why the harness's card was refused, when it was. Surfaced in the pane as a
    /// quiet note rather than hidden: a card that silently degraded is a card
    /// nobody can debug.
    pub render_rejection: Option<String>,
    pub resolution: Option<String>,
}

/// Insert the items ingress just reported, and return only the genuinely new ones.
///
/// The dedup is the `INSERT OR IGNORE` itself, not a prior `SELECT`: two polls
/// racing on the same message must produce one announcement, and only the unique
/// index can promise that. The returned list is exactly what may be announced.
pub fn record_arrivals(
    db: &Connection,
    items: &[InboxItem],
) -> Result<Vec<InboxItem>, BridgeError> {
    let mut fresh = Vec::new();
    for item in items {
        let inserted = db.execute(
            "INSERT OR IGNORE INTO connector_inbox_items
                 (item_key,family,channel_id,channel_label,message_ts,author,kind,body,permalink,received_at,state)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,'pending')",
            params![
                item.key(),
                item.family.as_str(),
                item.channel_id,
                item.channel_label,
                item.message_ts,
                item.author,
                kind_str(item.kind),
                item.text,
                item.permalink,
                item.received_at,
            ],
        )?;
        if inserted == 1 {
            fresh.push(item.clone());
        }
    }
    Ok(fresh)
}

/// Attach a card to an item. A resolved item is left alone — a render run that
/// lands after the user already replied must not reopen it.
pub fn attach_card(
    db: &Connection,
    item_key: &str,
    card: &ConnectorCard,
    rejection: Option<&str>,
) -> Result<bool, BridgeError> {
    let updated = db.execute(
        "UPDATE connector_inbox_items
            SET card=?2, render_rejection=?3, state='rendered'
          WHERE item_key=?1 AND state!='resolved'",
        params![
            item_key,
            serde_json::to_string(card).map_err(|error| BridgeError::Invalid(error.to_string()))?,
            rejection
        ],
    )?;
    Ok(updated == 1)
}

/// Mark an item dealt with. Returns false when it was already resolved, which is
/// what makes a double-send refusable at the storage layer rather than only in
/// the UI.
pub fn resolve(
    db: &Connection,
    item_key: &str,
    resolution: Resolution,
    at: &str,
) -> Result<bool, BridgeError> {
    let updated = db.execute(
        "UPDATE connector_inbox_items
            SET state='resolved', resolution=?2, resolved_at=?3
          WHERE item_key=?1 AND state!='resolved'",
        params![item_key, resolution.as_str(), at],
    )?;
    Ok(updated == 1)
}

pub fn load(db: &Connection, item_key: &str) -> Result<Option<StoredItem>, BridgeError> {
    db.query_row(
        "SELECT family,channel_id,channel_label,message_ts,author,kind,body,permalink,received_at,
                state,card,render_rejection,resolution
           FROM connector_inbox_items WHERE item_key=?1",
        params![item_key],
        row_to_item,
    )
    .optional()
    .map_err(BridgeError::from)
}

/// The pane's list: unresolved first and newest first, bounded.
pub fn list(db: &Connection, limit: usize) -> Result<Vec<StoredItem>, BridgeError> {
    let mut statement = db.prepare(
        "SELECT family,channel_id,channel_label,message_ts,author,kind,body,permalink,received_at,
                state,card,render_rejection,resolution
           FROM connector_inbox_items
          ORDER BY (state='resolved') ASC, received_at DESC, item_key DESC
          LIMIT ?1",
    )?;
    let rows = statement.query_map(params![limit as i64], row_to_item)?;
    let mut items = Vec::new();
    for row in rows {
        items.push(row?);
    }
    Ok(items)
}

pub fn unread_count(db: &Connection) -> Result<i64, BridgeError> {
    db.query_row(
        "SELECT COUNT(*) FROM connector_inbox_items WHERE state!='resolved'",
        [],
        |row| row.get(0),
    )
    .map_err(BridgeError::from)
}

fn row_to_item(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredItem> {
    let family: String = row.get(0)?;
    let kind: String = row.get(5)?;
    let card: Option<String> = row.get(10)?;
    Ok(StoredItem {
        item: InboxItem {
            family: ConnectorFamily::parse(&family).unwrap_or(ConnectorFamily::Slack),
            channel_id: row.get(1)?,
            channel_label: row.get(2)?,
            message_ts: row.get(3)?,
            author: row.get(4)?,
            kind: parse_kind(&kind),
            text: row.get(6)?,
            permalink: row.get(7)?,
            received_at: row.get(8)?,
        },
        state: ItemState::parse(&row.get::<_, String>(9)?),
        // A card that no longer parses is a card this build cannot draw. Dropping
        // it degrades to the fallback rather than failing the whole list read.
        card: card.and_then(|raw| serde_json::from_str(&raw).ok()),
        render_rejection: row.get(11)?,
        resolution: row.get(12)?,
    })
}

fn kind_str(kind: ItemKind) -> &'static str {
    match kind {
        ItemKind::DirectMessage => "direct_message",
        ItemKind::Mention => "mention",
        ItemKind::ThreadReply => "thread_reply",
    }
}

fn parse_kind(value: &str) -> ItemKind {
    match value {
        "mention" => ItemKind::Mention,
        "thread_reply" => ItemKind::ThreadReply,
        _ => ItemKind::DirectMessage,
    }
}

// ── Poll bookkeeping ─────────────────────────────────────────────────────────

/// What the last ingress cycle did. The pane shows this instead of pretending an
/// inbox that could not be read is an inbox that is empty — the distinction
/// matters most exactly when something is wrong.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PollStatus {
    pub family: ConnectorFamily,
    pub last_attempt_at: Option<String>,
    pub last_success_at: Option<String>,
    pub degraded: Option<String>,
}

pub fn record_poll_success(
    db: &Connection,
    family: ConnectorFamily,
    at: &str,
) -> Result<(), BridgeError> {
    db.execute(
        "INSERT INTO connector_poll_state(family,last_attempt_at,last_success_at,degraded)
         VALUES(?1,?2,?2,NULL)
         ON CONFLICT(family) DO UPDATE SET last_attempt_at=?2,last_success_at=?2,degraded=NULL",
        params![family.as_str(), at],
    )?;
    Ok(())
}

/// Record a failed cycle. Deliberately does not touch `last_success_at` or any
/// item row: a failed read tells you nothing about the inbox, so it must not be
/// allowed to look like an empty one.
pub fn record_poll_failure(
    db: &Connection,
    family: ConnectorFamily,
    at: &str,
    detail: &str,
) -> Result<(), BridgeError> {
    let detail: String = detail.chars().take(300).collect();
    db.execute(
        "INSERT INTO connector_poll_state(family,last_attempt_at,last_success_at,degraded)
         VALUES(?1,?2,NULL,?3)
         ON CONFLICT(family) DO UPDATE SET last_attempt_at=?2,degraded=?3",
        params![family.as_str(), at, detail],
    )?;
    Ok(())
}

pub fn poll_status(db: &Connection, family: ConnectorFamily) -> Result<PollStatus, BridgeError> {
    db.query_row(
        "SELECT last_attempt_at,last_success_at,degraded FROM connector_poll_state WHERE family=?1",
        params![family.as_str()],
        |row| {
            Ok(PollStatus {
                family,
                last_attempt_at: row.get(0)?,
                last_success_at: row.get(1)?,
                degraded: row.get(2)?,
            })
        },
    )
    .optional()
    .map_err(BridgeError::from)
    .map(|status| {
        status.unwrap_or(PollStatus {
            family,
            last_attempt_at: None,
            last_success_at: None,
            degraded: None,
        })
    })
}

/// Parse an ingress run's JSON into items. Strict: the run is asked for exactly
/// this shape, and an item missing an identity field is dropped rather than
/// guessed at, because a guessed key breaks announce-once.
pub fn parse_ingress(
    value: &Value,
    family: ConnectorFamily,
    received_at: &str,
) -> Vec<InboxItem> {
    let Some(entries) = value.get("items").and_then(Value::as_array) else {
        return Vec::new();
    };
    entries
        .iter()
        .filter_map(|entry| {
            let text = |field: &str| {
                entry.get(field).and_then(Value::as_str).map(str::trim).filter(|value| !value.is_empty())
            };
            let channel_id = text("channelId")?.to_owned();
            let message_ts = text("messageTs")?.to_owned();
            let author = text("author")?.to_owned();
            Some(InboxItem {
                family,
                channel_label: text("channelLabel").unwrap_or(&channel_id).to_owned(),
                channel_id,
                message_ts,
                author,
                kind: match text("kind") {
                    Some("mention") => ItemKind::Mention,
                    Some("thread_reply") => ItemKind::ThreadReply,
                    _ => ItemKind::DirectMessage,
                },
                // A body is allowed to be empty (a file share, a reaction-only
                // post); an identity field is not.
                text: entry.get("text").and_then(Value::as_str).unwrap_or_default().to_owned(),
                permalink: text("permalink").map(str::to_owned),
                received_at: text("receivedAt").unwrap_or(received_at).to_owned(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connector_surface::CardBlock;
    use crate::store;
    use serde_json::json;

    fn db() -> Connection {
        store::open(std::path::Path::new(":memory:")).unwrap()
    }

    fn item(ts: &str) -> InboxItem {
        InboxItem {
            family: ConnectorFamily::Slack,
            channel_id: "D0BV4LADFGB".into(),
            channel_label: "Nina Alvarez".into(),
            message_ts: ts.into(),
            author: "Nina Alvarez".into(),
            kind: ItemKind::DirectMessage,
            text: "release checklist before standup?".into(),
            permalink: None,
            received_at: "2026-09-13T09:14:00Z".into(),
        }
    }

    fn card(key: &str) -> ConnectorCard {
        ConnectorCard {
            item_key: key.into(),
            headline: "Nina asked about the release checklist".into(),
            blocks: vec![CardBlock::Context { text: "1 earlier message".into() }],
            suggested_replies: vec!["On it.".into()],
            harness_rendered: true,
        }
    }

    #[test]
    fn a_new_item_announces_once() {
        let db = db();
        let fresh = record_arrivals(&db, &[item("1.1")]).unwrap();
        assert_eq!(fresh.len(), 1);
        assert_eq!(unread_count(&db).unwrap(), 1);
    }

    #[test]
    fn a_seen_key_is_never_reannounced() {
        let db = db();
        record_arrivals(&db, &[item("1.1")]).unwrap();
        // Ingress re-reads an overlapping window every cycle; the second sight of
        // the same message is not news.
        let second = record_arrivals(&db, &[item("1.1")]).unwrap();
        assert!(second.is_empty());
        assert_eq!(unread_count(&db).unwrap(), 1, "and it is not stored twice");
    }

    #[test]
    fn a_batch_announces_only_the_unseen_members() {
        let db = db();
        record_arrivals(&db, &[item("1.1")]).unwrap();
        let fresh = record_arrivals(&db, &[item("1.1"), item("1.2"), item("1.3")]).unwrap();
        let announced: Vec<_> = fresh.iter().map(|entry| entry.message_ts.clone()).collect();
        assert_eq!(announced, vec!["1.2", "1.3"]);
    }

    #[test]
    fn the_seen_ledger_survives_a_restart() {
        let scratch = tempfile::tempdir().unwrap();
        let path = scratch.path().join("bridge.db");
        {
            let db = store::open(&path).unwrap();
            record_arrivals(&db, &[item("1.1")]).unwrap();
        }
        // A restart that re-announced yesterday's messages would make the whole
        // surface untrustworthy, so this is the load-bearing case.
        let db = store::open(&path).unwrap();
        assert!(record_arrivals(&db, &[item("1.1")]).unwrap().is_empty());
    }

    #[test]
    fn resolving_an_item_marks_it_read_and_is_idempotent() {
        let db = db();
        record_arrivals(&db, &[item("1.1")]).unwrap();
        let key = item("1.1").key();
        assert!(resolve(&db, &key, Resolution::Replied, "2026-09-13T09:20:00Z").unwrap());
        assert_eq!(unread_count(&db).unwrap(), 0);
        // The second attempt is what a double-send looks like from storage.
        assert!(!resolve(&db, &key, Resolution::Replied, "2026-09-13T09:21:00Z").unwrap());
        assert_eq!(load(&db, &key).unwrap().unwrap().state, ItemState::Resolved);
    }

    #[test]
    fn a_resolved_item_is_never_reannounced() {
        let db = db();
        record_arrivals(&db, &[item("1.1")]).unwrap();
        resolve(&db, &item("1.1").key(), Resolution::Dismissed, "2026-09-13T09:20:00Z").unwrap();
        assert!(record_arrivals(&db, &[item("1.1")]).unwrap().is_empty());
    }

    #[test]
    fn attaching_a_card_moves_the_item_to_rendered() {
        let db = db();
        record_arrivals(&db, &[item("1.1")]).unwrap();
        let key = item("1.1").key();
        assert!(attach_card(&db, &key, &card(&key), None).unwrap());
        let stored = load(&db, &key).unwrap().unwrap();
        assert_eq!(stored.state, ItemState::Rendered);
        assert_eq!(stored.card.unwrap().headline, "Nina asked about the release checklist");
    }

    #[test]
    fn a_card_landing_after_resolution_does_not_reopen_the_item() {
        let db = db();
        record_arrivals(&db, &[item("1.1")]).unwrap();
        let key = item("1.1").key();
        resolve(&db, &key, Resolution::Replied, "2026-09-13T09:20:00Z").unwrap();
        // The render run was already in flight when the user replied.
        assert!(!attach_card(&db, &key, &card(&key), None).unwrap());
        assert_eq!(load(&db, &key).unwrap().unwrap().state, ItemState::Resolved);
    }

    #[test]
    fn a_rejection_is_recorded_alongside_the_fallback_card() {
        let db = db();
        record_arrivals(&db, &[item("1.1")]).unwrap();
        let key = item("1.1").key();
        attach_card(&db, &key, &ConnectorCard::fallback(&item("1.1")), Some("the card was empty")).unwrap();
        let stored = load(&db, &key).unwrap().unwrap();
        assert_eq!(stored.render_rejection.as_deref(), Some("the card was empty"));
        assert!(!stored.card.unwrap().harness_rendered);
    }

    #[test]
    fn a_failed_poll_preserves_the_ledger_and_reports_degraded() {
        let db = db();
        record_arrivals(&db, &[item("1.1")]).unwrap();
        record_poll_success(&db, ConnectorFamily::Slack, "2026-09-13T09:00:00Z").unwrap();
        record_poll_failure(&db, ConnectorFamily::Slack, "2026-09-13T09:05:00Z", "provider timed out").unwrap();
        let status = poll_status(&db, ConnectorFamily::Slack).unwrap();
        assert_eq!(status.degraded.as_deref(), Some("provider timed out"));
        // Still the last time the inbox was actually known-good, and the item is
        // untouched: a failed read must never read as an empty inbox.
        assert_eq!(status.last_success_at.as_deref(), Some("2026-09-13T09:00:00Z"));
        assert_eq!(unread_count(&db).unwrap(), 1);
    }

    #[test]
    fn a_success_clears_a_previous_degradation() {
        let db = db();
        record_poll_failure(&db, ConnectorFamily::Slack, "2026-09-13T09:05:00Z", "timed out").unwrap();
        record_poll_success(&db, ConnectorFamily::Slack, "2026-09-13T09:10:00Z").unwrap();
        assert_eq!(poll_status(&db, ConnectorFamily::Slack).unwrap().degraded, None);
    }

    #[test]
    fn an_unpolled_family_reports_no_history_rather_than_failing() {
        let status = poll_status(&db(), ConnectorFamily::Slack).unwrap();
        assert_eq!(status.last_attempt_at, None);
        assert_eq!(status.degraded, None);
    }

    #[test]
    fn the_list_puts_unresolved_items_first_then_newest_first() {
        let db = db();
        let mut older = item("1.1");
        older.received_at = "2026-09-13T08:00:00Z".into();
        let mut newer = item("1.2");
        newer.received_at = "2026-09-13T09:00:00Z".into();
        record_arrivals(&db, &[older.clone(), newer.clone()]).unwrap();
        resolve(&db, &newer.key(), Resolution::Dismissed, "2026-09-13T09:30:00Z").unwrap();
        let listed = list(&db, 10).unwrap();
        assert_eq!(listed[0].item.message_ts, "1.1", "the unresolved one leads");
        assert_eq!(listed[1].item.message_ts, "1.2");
    }

    #[test]
    fn ingress_parsing_drops_entries_without_an_identity() {
        let payload = json!({"items": [
            {"channelId": "C1", "messageTs": "1.1", "author": "nina", "kind": "mention", "text": "ping"},
            {"channelId": "C1", "author": "nina", "text": "no timestamp"},
            {"messageTs": "1.2", "author": "nina", "text": "no channel"},
        ]});
        let parsed = parse_ingress(&payload, ConnectorFamily::Slack, "2026-09-13T09:00:00Z");
        // A guessed key would break announce-once, so a partial entry is dropped.
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].kind, ItemKind::Mention);
        assert_eq!(parsed[0].channel_label, "C1", "the id stands in for a missing label");
    }

    #[test]
    fn ingress_parsing_tolerates_an_empty_body_but_not_a_missing_envelope() {
        let payload = json!({"items": [
            {"channelId": "C1", "messageTs": "1.1", "author": "nina"},
        ]});
        assert_eq!(parse_ingress(&payload, ConnectorFamily::Slack, "now")[0].text, "");
        assert!(parse_ingress(&json!({"nope": []}), ConnectorFamily::Slack, "now").is_empty());
        assert!(parse_ingress(&json!("not an object"), ConnectorFamily::Slack, "now").is_empty());
    }

    #[test]
    fn a_stored_item_round_trips_every_field() {
        let db = db();
        let mut original = item("1.1");
        original.permalink = Some("https://app.slack.com/archives/D0/p1".into());
        original.kind = ItemKind::ThreadReply;
        record_arrivals(&db, &[original.clone()]).unwrap();
        assert_eq!(load(&db, &original.key()).unwrap().unwrap().item, original);
    }
}
