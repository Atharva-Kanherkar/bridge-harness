//! One page as a `yrs` document, and its change log in `notes.db`.
//!
//! `Y.Text("body")` holds Your words as Bridge Markdown; `Y.Map("props")`
//! holds the page's properties. Every write returns the Yjs update it made,
//! which the service appends to `page_updates` beside the index row. A doc is
//! rebuilt by replaying that log from the newest snapshot at or before the
//! wanted sequence.
//!
//! A person's save carries the full text and the `seq` the editor started
//! from. [`merge_save`] replays the page to that `seq`, applies the text
//! difference there as a fresh client, and returns that update; applied to the
//! current doc it merges with whatever an agent wrote in the meantime instead
//! of overwriting it. Every doc instance gets a random client id, so the
//! rebuilt base and the live doc never share one.

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use similar::{ChangeTag, TextDiff};
use yrs::updates::decoder::Decode;
use yrs::{Any, Doc, GetString, In, Map, Out, ReadTxn, StateVector, Text, Transact, TransactionMut, Update};

use crate::BridgeError;

pub const BODY: &str = "body";
pub const PROPS: &str = "props";
/// Updates between snapshots of one page.
pub const SNAPSHOT_EVERY: i64 = 500;

/// A page's properties as the doc holds them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Props {
    pub title: Option<String>,
    pub kind: Option<String>,
    pub tags: Vec<String>,
    pub workspace_id: Option<String>,
    pub state: Option<String>,
}

/// A change to properties. `None` leaves a key alone; `Some(None)` clears it.
#[derive(Debug, Clone, Default)]
pub struct PropsPatch {
    pub title: Option<Option<String>>,
    pub kind: Option<String>,
    pub tags: Option<Vec<String>>,
    pub workspace_id: Option<Option<String>>,
    pub state: Option<String>,
}

pub struct PageDoc {
    doc: Doc,
}

fn yrs_error(error: impl std::fmt::Display) -> BridgeError {
    BridgeError::Invalid(format!("page document: {error}"))
}

impl Default for PageDoc {
    fn default() -> Self {
        Self { doc: Doc::new() }
    }
}

impl PageDoc {
    pub fn new() -> Self {
        Self::default()
    }

    /// Rebuild from an optional snapshot and the updates that follow it.
    pub fn load<'a>(snapshot: Option<&[u8]>, updates: impl IntoIterator<Item = &'a [u8]>) -> Result<Self, BridgeError> {
        let page = Self::new();
        if let Some(state) = snapshot {
            page.apply(state)?;
        }
        for update in updates {
            page.apply(update)?;
        }
        Ok(page)
    }

    pub fn apply(&self, update: &[u8]) -> Result<(), BridgeError> {
        let update = Update::decode_v1(update).map_err(yrs_error)?;
        self.doc.transact_mut().apply_update(update).map_err(yrs_error)
    }

    pub fn body(&self) -> String {
        let text = self.doc.get_or_insert_text(BODY);
        let txn = self.doc.transact();
        text.get_string(&txn)
    }

    pub fn props(&self) -> Props {
        let map = self.doc.get_or_insert_map(PROPS);
        let txn = self.doc.transact();
        let string = |key: &str| match map.get(&txn, key) {
            Some(Out::Any(Any::String(value))) => Some(value.to_string()),
            _ => None,
        };
        Props {
            title: string("title"),
            kind: string("kind"),
            tags: string("tags").and_then(|json| serde_json::from_str(&json).ok()).unwrap_or_default(),
            workspace_id: string("workspaceId"),
            state: string("state"),
        }
    }

    /// The whole document as one update, for a snapshot.
    pub fn encode_state(&self) -> Vec<u8> {
        self.doc.transact().encode_state_as_update_v1(&StateVector::default())
    }

    /// Run one transaction and return its update, or `None` if it changed nothing.
    fn change(&self, edit: impl FnOnce(&mut TransactionMut, &yrs::TextRef, &yrs::MapRef)) -> Result<Option<Vec<u8>>, BridgeError> {
        let text = self.doc.get_or_insert_text(BODY);
        let map = self.doc.get_or_insert_map(PROPS);
        let bytes = {
            let mut txn = self.doc.transact_mut();
            edit(&mut txn, &text, &map);
            txn.encode_update_v1()
        };
        let empty = Update::decode_v1(&bytes).map_err(yrs_error)?.is_empty();
        Ok((!empty).then_some(bytes))
    }

    /// Replace the body with `next`, as the smallest set of character edits.
    pub fn set_body(&self, next: &str) -> Result<Option<Vec<u8>>, BridgeError> {
        let current = self.body();
        if current == next {
            return Ok(None);
        }
        self.change(|txn, text, _| apply_text_diff(txn, text, &current, next))
    }

    pub fn insert_body(&self, byte_index: usize, chunk: &str) -> Result<Option<Vec<u8>>, BridgeError> {
        if chunk.is_empty() {
            return Ok(None);
        }
        self.change(|txn, text, _| text.insert(txn, byte_index as u32, chunk))
    }

    pub fn set_props(&self, patch: &PropsPatch) -> Result<Option<Vec<u8>>, BridgeError> {
        let current = self.props();
        self.change(|txn, _, map| {
            let mut set = |key: &str, value: Option<String>, was: Option<String>| {
                if value == was {
                    return;
                }
                match value {
                    Some(value) => {
                        map.insert(txn, key, In::from(value));
                    }
                    None => {
                        map.remove(txn, key);
                    }
                }
            };
            if let Some(title) = &patch.title {
                set("title", title.clone(), current.title.clone());
            }
            if let Some(kind) = &patch.kind {
                set("kind", Some(kind.clone()), current.kind.clone());
            }
            if let Some(tags) = &patch.tags {
                let encode = |tags: &[String]| serde_json::to_string(tags).unwrap_or_else(|_| "[]".into());
                let was = (!current.tags.is_empty()).then(|| encode(&current.tags));
                set("tags", (!tags.is_empty()).then(|| encode(tags)), was);
            }
            if let Some(workspace) = &patch.workspace_id {
                set("workspaceId", workspace.clone(), current.workspace_id.clone());
            }
            if let Some(state) = &patch.state {
                set("state", Some(state.clone()), current.state.clone());
            }
        })
    }
}

/// Apply the character-level difference between `old` (the text's current
/// content) and `new`. Offsets are UTF-8 bytes, the doc's offset kind.
fn apply_text_diff(txn: &mut TransactionMut, text: &yrs::TextRef, old: &str, new: &str) {
    let diff = TextDiff::from_chars(old, new);
    let mut cursor = 0u32; // byte position in the text as edited so far
    for change in diff.iter_all_changes() {
        let len = change.value().len() as u32;
        match change.tag() {
            ChangeTag::Equal => cursor += len,
            ChangeTag::Delete => text.remove_range(txn, cursor, len),
            ChangeTag::Insert => {
                text.insert(txn, cursor, change.value());
                cursor += len;
            }
        }
    }
}

/// One row of the change log.
#[derive(Debug, Clone)]
pub struct LoggedUpdate {
    pub seq: i64,
    pub bytes: Vec<u8>,
}

/// Append one update for `page_id` and return its sequence number.
pub fn append_update(
    db: &Connection,
    page_id: &str,
    bytes: &[u8],
    actor: &str,
    hlc: &str,
    device: &str,
) -> Result<i64, BridgeError> {
    let seq: i64 = db.query_row(
        "SELECT COALESCE(MAX(seq), 0) + 1 FROM page_updates WHERE page_id=?1",
        params![page_id],
        |row| row.get(0),
    )?;
    db.execute(
        "INSERT INTO page_updates(page_id, seq, hlc, actor, device_id, update_bytes) VALUES(?1,?2,?3,?4,?5,?6)",
        params![page_id, seq, hlc, actor, device, bytes],
    )?;
    Ok(seq)
}

/// Rebuild a page as of `upto` (inclusive; `None` = the latest).
pub fn load_page(db: &Connection, page_id: &str, upto: Option<i64>) -> Result<(PageDoc, i64), BridgeError> {
    let limit = upto.unwrap_or(i64::MAX);
    let snapshot: Option<(i64, Vec<u8>)> = db
        .query_row(
            "SELECT seq, state FROM page_snapshots WHERE page_id=?1 AND seq<=?2 ORDER BY seq DESC LIMIT 1",
            params![page_id, limit],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let from = snapshot.as_ref().map(|(seq, _)| *seq).unwrap_or(0);
    let mut statement = db.prepare(
        "SELECT seq, update_bytes FROM page_updates WHERE page_id=?1 AND seq>?2 AND seq<=?3 ORDER BY seq",
    )?;
    let updates: Vec<LoggedUpdate> = statement
        .query_map(params![page_id, from, limit], |row| Ok(LoggedUpdate { seq: row.get(0)?, bytes: row.get(1)? }))?
        .collect::<Result<_, _>>()?;
    let last = updates.last().map(|update| update.seq).unwrap_or(from);
    let page = PageDoc::load(snapshot.as_ref().map(|(_, state)| state.as_slice()), updates.iter().map(|update| update.bytes.as_slice()))?;
    Ok((page, last))
}

/// Write a snapshot when `seq` crosses a [`SNAPSHOT_EVERY`] boundary.
pub fn maybe_snapshot(db: &Connection, page_id: &str, seq: i64, page: &PageDoc) -> Result<bool, BridgeError> {
    if seq % SNAPSHOT_EVERY != 0 {
        return Ok(false);
    }
    db.execute(
        "INSERT OR REPLACE INTO page_snapshots(page_id, seq, state) VALUES(?1,?2,?3)",
        params![page_id, seq, page.encode_state()],
    )?;
    Ok(true)
}

/// The update a person's save makes: `text` typed against the page as of
/// `base_seq`. `None` when the text equals the page at that point.
pub fn merge_save(db: &Connection, page_id: &str, base_seq: i64, text: &str) -> Result<Option<Vec<u8>>, BridgeError> {
    let (base, _) = load_page(db, page_id, Some(base_seq))?;
    base.set_body(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notes::store;

    fn db() -> (tempfile::TempDir, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let connection = store::open(&dir.path().join(store::FILE_NAME)).unwrap();
        (dir, connection)
    }

    fn log(db: &Connection, page: &str, bytes: Option<Vec<u8>>) -> i64 {
        append_update(db, page, &bytes.expect("a change"), "person", "0000000000001-000000-dev", "dev").unwrap()
    }

    #[test]
    fn replay_reproduces_body_and_props() {
        let (_dir, db) = db();
        let page = PageDoc::new();
        log(&db, "p1", page.set_body("helo wrld, a typo stays").unwrap());
        log(&db, "p1", page.set_props(&PropsPatch { title: Some(Some("Launch".into())), kind: Some("idea".into()), tags: Some(vec!["launch".into(), "q4".into()]), workspace_id: Some(None), state: Some("inbox".into()) }).unwrap());
        log(&db, "p1", page.insert_body(page.body().len(), "\n- one more line ✨").unwrap());
        let (replayed, seq) = load_page(&db, "p1", None).unwrap();
        assert_eq!(seq, 3);
        assert_eq!(replayed.body(), "helo wrld, a typo stays\n- one more line ✨");
        assert_eq!(replayed.body(), page.body());
        assert_eq!(replayed.props(), page.props());
        assert_eq!(replayed.props().tags, vec!["launch", "q4"]);
        assert_eq!(replayed.props().workspace_id, None);
        log(&db, "p1", page.set_body("café ✨ wörld, naïve edits").unwrap());
        assert_eq!(page.body(), "café ✨ wörld, naïve edits");
        assert_eq!(load_page(&db, "p1", None).unwrap().0.body(), "café ✨ wörld, naïve edits");
        let (at_one, _) = load_page(&db, "p1", Some(1)).unwrap();
        assert_eq!(at_one.body(), "helo wrld, a typo stays");
        assert_eq!(at_one.props(), Props::default());
    }

    #[test]
    fn snapshot_replay_equals_full_replay() {
        let (_dir, db) = db();
        let page = PageDoc::new();
        let mut seq = 0;
        for line in 0..(SNAPSHOT_EVERY + 7) {
            let at = page.body().len();
            seq = log(&db, "p1", page.insert_body(at, &format!("line {line}\n")).unwrap());
            maybe_snapshot(&db, "p1", seq, &page).unwrap();
        }
        let snapshots: i64 = db.query_row("SELECT COUNT(*) FROM page_snapshots", [], |row| row.get(0)).unwrap();
        assert_eq!(snapshots, 1);
        let (from_snapshot, last) = load_page(&db, "p1", None).unwrap();
        assert_eq!(last, seq);
        db.execute("DELETE FROM page_snapshots", []).unwrap();
        let (full, _) = load_page(&db, "p1", None).unwrap();
        assert_eq!(from_snapshot.body(), full.body());
        assert_eq!(full.body(), page.body());
    }

    #[test]
    fn stale_save_merges_with_a_concurrent_append() {
        let (_dir, db) = db();
        let live = PageDoc::new();
        let base_seq = log(&db, "p1", live.set_body("Plan\n- research sync libs\n").unwrap());

        // An agent appends while the person is still typing against base_seq.
        let agent = live.insert_body(live.body().len(), "- agent: compare yrs and automerge\n").unwrap();
        log(&db, "p1", agent);

        // The person's save, typed against the old version, fixes a word at the top.
        let save = merge_save(&db, "p1", base_seq, "Plan for Q4\n- research sync libs\n").unwrap().expect("a change");
        live.apply(&save).unwrap();
        log(&db, "p1", Some(save));

        let (replayed, _) = load_page(&db, "p1", None).unwrap();
        for page in [&live, &replayed] {
            assert_eq!(page.body(), "Plan for Q4\n- research sync libs\n- agent: compare yrs and automerge\n");
        }
    }

    #[test]
    fn unchanged_save_writes_no_update() {
        let (_dir, db) = db();
        let live = PageDoc::new();
        let base_seq = log(&db, "p1", live.set_body("same words").unwrap());
        assert!(merge_save(&db, "p1", base_seq, "same words").unwrap().is_none());
        assert!(live.set_props(&PropsPatch::default()).unwrap().is_none());
        assert!(live.set_props(&PropsPatch { state: None, title: Some(None), ..Default::default() }).unwrap().is_none());
    }
}
