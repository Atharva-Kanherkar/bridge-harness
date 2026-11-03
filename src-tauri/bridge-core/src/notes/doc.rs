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
//!
//! Database records are one map entry per property, per view and per value, so
//! edits to different entries merge. A property's options live inside that
//! property's entry: two people changing the same property at once are
//! last-writer-wins, and the later update is the one a replay ends with.

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use similar::{ChangeTag, TextDiff};
use yrs::updates::decoder::Decode;
use yrs::{Any, Doc, GetString, In, Map, Out, ReadTxn, StateVector, Text, Transact, TransactionMut, Update};

use crate::BridgeError;

pub const BODY: &str = "body";
pub const PROPS: &str = "props";
/// Database pages: one map entry per property, per view, and the scope.
pub const DATABASE: &str = "database";
pub const SCHEMA: &str = "schema";
pub const VIEWS: &str = "views";
/// Old property names a rename or a delete retired, mapping the name to the
/// property id. Files still carrying the old line drop it on their next write.
pub const RETIRED: &str = "retired";
/// Row pages: one map entry per property id, holding its JSON value.
pub const VALUES: &str = "values";
/// The one entry of `Y.Map("database")`.
pub const SCOPE: &str = "scope";
/// Updates between snapshots of one page.
pub const SNAPSHOT_EVERY: i64 = 500;

/// A page's properties as the doc holds them. Everything the index needs to
/// rebuild a page lives here, so `pages` can be reconstructed from the log.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Props {
    pub space_id: Option<String>,
    pub title: Option<String>,
    pub kind: Option<String>,
    pub tags: Vec<String>,
    pub workspace_id: Option<String>,
    pub state: Option<String>,
    pub parent_id: Option<String>,
    pub source_id: Option<String>,
    pub session_ids: Vec<String>,
    pub source_excerpt: Option<String>,
    pub created_by: Option<String>,
    pub created_at: Option<String>,
    pub deleted_at: Option<String>,
}

/// Property keys, as stored in `Y.Map("props")`. List values are JSON arrays.
pub mod key {
    pub const SPACE: &str = "spaceId";
    pub const TITLE: &str = "title";
    pub const KIND: &str = "kind";
    pub const TAGS: &str = "tags";
    pub const WORKSPACE: &str = "workspaceId";
    pub const STATE: &str = "state";
    pub const PARENT: &str = "parentId";
    pub const SOURCE: &str = "sourceId";
    pub const SESSIONS: &str = "sessionIds";
    pub const EXCERPT: &str = "sourceExcerpt";
    pub const CREATED_BY: &str = "createdBy";
    pub const CREATED_AT: &str = "createdAt";
    pub const DELETED_AT: &str = "deletedAt";
}

/// A change to properties: each key set to a value, or cleared with `None`.
#[derive(Debug, Clone, Default)]
pub struct PropsPatch(pub Vec<(&'static str, Option<String>)>);

impl PropsPatch {
    pub fn set(mut self, key: &'static str, value: impl Into<String>) -> Self {
        self.0.push((key, Some(value.into())));
        self
    }
    pub fn clear(mut self, key: &'static str) -> Self {
        self.0.push((key, None));
        self
    }
    pub fn maybe(mut self, key: &'static str, value: Option<String>) -> Self {
        self.0.push((key, value));
        self
    }
    /// A list value; an empty list clears the key.
    pub fn list(self, key: &'static str, values: &[String]) -> Self {
        let encoded = (!values.is_empty()).then(|| serde_json::to_string(values).unwrap_or_else(|_| "[]".into()));
        self.maybe(key, encoded)
    }
}

/// A property's type. The wire and file spelling is camelCase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PropertyType {
    Text,
    Number,
    Select,
    MultiSelect,
    Date,
    Checkbox,
    Url,
    Relation,
    Status,
}

impl PropertyType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Number => "number",
            Self::Select => "select",
            Self::MultiSelect => "multiSelect",
            Self::Date => "date",
            Self::Checkbox => "checkbox",
            Self::Url => "url",
            Self::Relation => "relation",
            Self::Status => "status",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        [
            Self::Text,
            Self::Number,
            Self::Select,
            Self::MultiSelect,
            Self::Date,
            Self::Checkbox,
            Self::Url,
            Self::Relation,
            Self::Status,
        ]
        .into_iter()
        .find(|kind| kind.as_str() == raw.trim())
    }

    /// `select`, `multiSelect` and `status` hold options.
    pub fn options(self) -> bool {
        matches!(self, Self::Select | Self::MultiSelect | Self::Status)
    }
}

/// One choice of a `select`, `multiSelect` or `status` property. Ids are
/// UUIDv7, so renaming an option touches no value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OptionDef {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// `status` only: `todo`, `inProgress` or `done`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
}

/// A `relation` property points at another database.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelationDef {
    pub database_id: String,
}

/// One column of a database.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PropertyDef {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: PropertyType,
    pub order: i64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<OptionDef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relation: Option<RelationDef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub date_format: Option<String>,
}

impl PropertyDef {
    pub fn option(&self, id: &str) -> Option<&OptionDef> {
        self.options.iter().find(|option| option.id == id)
    }

    pub fn option_by_name(&self, name: &str) -> Option<&OptionDef> {
        let name = name.trim().to_lowercase();
        self.options.iter().find(|option| option.name.to_lowercase() == name)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ViewType {
    Table,
    Board,
    Calendar,
    List,
}

impl ViewType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Table => "table",
            Self::Board => "board",
            Self::Calendar => "calendar",
            Self::List => "list",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        [Self::Table, Self::Board, Self::Calendar, Self::List].into_iter().find(|kind| kind.as_str() == raw.trim())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SortDef {
    pub property: String,
    /// `asc` or `desc`.
    pub direction: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupByDef {
    pub property: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub date_bucket: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewPropertyDef {
    pub id: String,
    pub visible: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<i64>,
}

/// One saved view: how a database is shown, not what it holds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewDef {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: ViewType,
    pub order: i64,
    /// Stored and validated only as JSON here; the grammar is #888's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sorts: Vec<SortDef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_by: Option<GroupByDef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub date_property: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub properties: Vec<ViewPropertyDef>,
}

/// A change to a page's record maps: each entry set to a JSON string, or
/// removed with `None`. `scope` is `Some(None)` to clear it.
#[derive(Debug, Clone, Default)]
pub struct RecordsPatch {
    pub scope: Option<Option<String>>,
    pub schema: Vec<(String, Option<String>)>,
    pub views: Vec<(String, Option<String>)>,
    pub values: Vec<(String, Option<String>)>,
    /// Retired property names, keyed by the name, holding the property id.
    pub retired: Vec<(String, Option<String>)>,
}

impl RecordsPatch {
    pub fn scope(mut self, scope: &str) -> Self {
        self.scope = Some(Some(scope.to_owned()));
        self
    }

    pub fn put_schema(mut self, id: &str, def: &PropertyDef) -> Self {
        self.schema.push((id.to_owned(), Some(json_string(def))));
        self
    }

    pub fn remove_schema(mut self, id: &str) -> Self {
        self.schema.push((id.to_owned(), None));
        self
    }

    pub fn put_view(mut self, id: &str, view: &ViewDef) -> Self {
        self.views.push((id.to_owned(), Some(json_string(view))));
        self
    }

    pub fn remove_view(mut self, id: &str) -> Self {
        self.views.push((id.to_owned(), None));
        self
    }

    /// Remember an old property name so files stop writing it.
    pub fn retire(mut self, name: &str, prop: &str) -> Self {
        self.retired.push((name.to_owned(), Some(json_string(&prop))));
        self
    }

    /// Forget a retired name once no schema uses it and no file should.
    pub fn restore_name(mut self, name: &str) -> Self {
        self.retired.push((name.to_owned(), None));
        self
    }

    /// Set (`Some`) or clear (`None`) one property's value.
    pub fn put_value(mut self, id: &str, value: Option<&serde_json::Value>) -> Self {
        self.values.push((id.to_owned(), value.map(json_string)));
        self
    }

    pub fn is_empty(&self) -> bool {
        self.scope.is_none() && self.schema.is_empty() && self.views.is_empty() && self.values.is_empty() && self.retired.is_empty()
    }
}

fn json_string(value: &impl Serialize) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "null".into())
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
        let string = |name: &str| match map.get(&txn, name) {
            Some(Out::Any(Any::String(value))) => Some(value.to_string()),
            _ => None,
        };
        let list = |name: &str| string(name).and_then(|json| serde_json::from_str(&json).ok()).unwrap_or_default();
        Props {
            space_id: string(key::SPACE),
            title: string(key::TITLE),
            kind: string(key::KIND),
            tags: list(key::TAGS),
            workspace_id: string(key::WORKSPACE),
            state: string(key::STATE),
            parent_id: string(key::PARENT),
            source_id: string(key::SOURCE),
            session_ids: list(key::SESSIONS),
            source_excerpt: string(key::EXCERPT),
            created_by: string(key::CREATED_BY),
            created_at: string(key::CREATED_AT),
            deleted_at: string(key::DELETED_AT),
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
        self.edit(None, patch)
    }

    /// Change the body (when `body` is given) and properties in one transaction.
    pub fn edit(&self, body: Option<&str>, patch: &PropsPatch) -> Result<Option<Vec<u8>>, BridgeError> {
        let current = body.map(|_| self.body());
        self.change(|txn, text, map| {
            if let (Some(next), Some(current)) = (body, current.as_deref()) {
                if next != current {
                    apply_text_diff(txn, text, current, next);
                }
            }
            apply_props(txn, map, patch);
        })
    }

    /// The database page's scope: `children` or `space`.
    pub fn scope(&self) -> Option<String> {
        let map = self.doc.get_or_insert_map(DATABASE);
        let txn = self.doc.transact();
        match map.get(&txn, SCOPE) {
            Some(Out::Any(Any::String(value))) => Some(value.to_string()),
            _ => None,
        }
    }

    /// One record map's entries as the JSON strings they are stored as,
    /// keyed by property or view id. Unparseable entries are left out.
    fn records(&self, name: &str) -> std::collections::BTreeMap<String, String> {
        let map = self.doc.get_or_insert_map(name);
        let txn = self.doc.transact();
        map.iter(&txn)
            .filter_map(|(key, value)| match value {
                Out::Any(Any::String(value)) => Some((key.to_string(), value.to_string())),
                _ => None,
            })
            .collect()
    }

    pub fn schema(&self) -> std::collections::BTreeMap<String, PropertyDef> {
        self.records(SCHEMA).into_iter().filter_map(|(id, json)| Some((id, serde_json::from_str(&json).ok()?))).collect()
    }

    pub fn views(&self) -> std::collections::BTreeMap<String, ViewDef> {
        self.records(VIEWS).into_iter().filter_map(|(id, json)| Some((id, serde_json::from_str(&json).ok()?))).collect()
    }

    pub fn values(&self) -> std::collections::BTreeMap<String, serde_json::Value> {
        self.records(VALUES).into_iter().filter_map(|(id, json)| Some((id, serde_json::from_str(&json).ok()?))).collect()
    }

    /// The old property names a rename or a delete retired.
    pub fn retired(&self) -> std::collections::BTreeMap<String, String> {
        self.records(RETIRED).into_iter().filter_map(|(name, json)| Some((name, serde_json::from_str(&json).ok()?))).collect()
    }

    /// Change the scope, schema, views or values in one transaction. Maps a
    /// patch does not touch are not created, so a plain page never grows empty
    /// shared types.
    pub fn edit_records(&self, patch: &RecordsPatch) -> Result<Option<Vec<u8>>, BridgeError> {
        if patch.is_empty() {
            return Ok(None);
        }
        let maps = self.record_maps(patch);
        let bytes = {
            let mut txn = self.doc.transact_mut();
            apply_records(&mut txn, &maps, patch);
            txn.encode_update_v1()
        };
        self.some_update(bytes)
    }

    /// Change the body, properties and records in one transaction, for a page
    /// created as a database with its seeded views.
    pub fn edit_page(&self, body: Option<&str>, patch: &PropsPatch, records: &RecordsPatch) -> Result<Option<Vec<u8>>, BridgeError> {
        let text = self.doc.get_or_insert_text(BODY);
        let props = self.doc.get_or_insert_map(PROPS);
        let maps = self.record_maps(records);
        let current = body.map(|_| self.body());
        let bytes = {
            let mut txn = self.doc.transact_mut();
            if let (Some(next), Some(current)) = (body, current.as_deref()) {
                if next != current {
                    apply_text_diff(&mut txn, &text, current, next);
                }
            }
            apply_props(&mut txn, &props, patch);
            apply_records(&mut txn, &maps, records);
            txn.encode_update_v1()
        };
        self.some_update(bytes)
    }

    fn record_maps(&self, patch: &RecordsPatch) -> RecordMaps {
        RecordMaps {
            database: patch.scope.is_some().then(|| self.doc.get_or_insert_map(DATABASE)),
            schema: (!patch.schema.is_empty()).then(|| self.doc.get_or_insert_map(SCHEMA)),
            views: (!patch.views.is_empty()).then(|| self.doc.get_or_insert_map(VIEWS)),
            values: (!patch.values.is_empty()).then(|| self.doc.get_or_insert_map(VALUES)),
            retired: (!patch.retired.is_empty()).then(|| self.doc.get_or_insert_map(RETIRED)),
        }
    }

    fn some_update(&self, bytes: Vec<u8>) -> Result<Option<Vec<u8>>, BridgeError> {
        let empty = Update::decode_v1(&bytes).map_err(yrs_error)?.is_empty();
        Ok((!empty).then_some(bytes))
    }
}

struct RecordMaps {
    database: Option<yrs::MapRef>,
    schema: Option<yrs::MapRef>,
    views: Option<yrs::MapRef>,
    values: Option<yrs::MapRef>,
    retired: Option<yrs::MapRef>,
}

fn apply_records(txn: &mut TransactionMut, maps: &RecordMaps, patch: &RecordsPatch) {
    if let (Some(map), Some(scope)) = (&maps.database, &patch.scope) {
        let was = match map.get(&*txn, SCOPE) {
            Some(Out::Any(Any::String(current))) => Some(current.to_string()),
            _ => None,
        };
        if *scope != was {
            match scope {
                Some(scope) => {
                    map.insert(txn, SCOPE, In::from(scope.clone()));
                }
                None => {
                    map.remove(txn, SCOPE);
                }
            }
        }
    }
    for (map, entries) in [(&maps.schema, &patch.schema), (&maps.views, &patch.views), (&maps.values, &patch.values), (&maps.retired, &patch.retired)] {
        let Some(map) = map else { continue };
        for (key, value) in entries {
            let was = match map.get(&*txn, key.as_str()) {
                Some(Out::Any(Any::String(current))) => Some(current.to_string()),
                _ => None,
            };
            if *value == was {
                continue;
            }
            match value {
                Some(value) => {
                    map.insert(txn, key.as_str(), In::from(value.clone()));
                }
                None => {
                    map.remove(txn, key.as_str());
                }
            }
        }
    }
}

fn apply_props(txn: &mut TransactionMut, map: &yrs::MapRef, patch: &PropsPatch) {
    for (name, value) in &patch.0 {
        let was = match map.get(&*txn, name) {
            Some(Out::Any(Any::String(current))) => Some(current.to_string()),
            _ => None,
        };
        if *value == was {
            continue;
        }
        match value {
            Some(value) => {
                map.insert(txn, *name, In::from(value.clone()));
            }
            None => {
                map.remove(txn, name);
            }
        }
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
        log(&db, "p1", page.set_props(&PropsPatch::default().set(key::TITLE, "Launch").set(key::KIND, "idea").list(key::TAGS, &["launch".into(), "q4".into()]).clear(key::WORKSPACE).set(key::STATE, "inbox")).unwrap());
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
        assert!(live.set_props(&PropsPatch::default().clear(key::TITLE).list(key::TAGS, &[])).unwrap().is_none());
    }

    #[test]
    fn concurrent_schema_edits_to_different_properties_both_survive() {
        let number = PropertyDef { name: "Rating".into(), kind: PropertyType::Number, order: 0, options: vec![], relation: None, date_format: None };
        let status = PropertyDef {
            name: "Status".into(),
            kind: PropertyType::Status,
            order: 1,
            options: vec![OptionDef { id: "reading".into(), name: "Reading".into(), color: None, group: Some("todo".into()) }],
            relation: None,
            date_format: None,
        };
        // Two devices, the same database: one renames a property, the other
        // adds an option to a different one. Both start from the page as it
        // stands, which is what a replay from `page_updates` gives each of them.
        let base = PageDoc::new();
        let base_update = base
            .edit_records(&RecordsPatch::default().scope("children").put_schema("rating", &number).put_schema("status", &status))
            .unwrap()
            .expect("a base");
        let left = PageDoc::new();
        let right = PageDoc::new();
        left.apply(&base_update).unwrap();
        right.apply(&base_update).unwrap();
        let rename = left
            .edit_records(&RecordsPatch::default().put_schema("rating", &PropertyDef { name: "Score".into(), ..number.clone() }))
            .unwrap()
            .expect("a rename");
        let added = right
            .edit_records(&RecordsPatch::default().put_schema(
                "status",
                &PropertyDef {
                    options: vec![status.options[0].clone(), OptionDef { id: "done".into(), name: "Done".into(), color: None, group: Some("done".into()) }],
                    ..status.clone()
                },
            ))
            .unwrap()
            .expect("an option");
        left.apply(&added).unwrap();
        right.apply(&rename).unwrap();
        for page in [&left, &right] {
            let schema = page.schema();
            assert_eq!(schema.len(), 2);
            assert_eq!(schema["rating"].name, "Score", "the rename survives");
            assert_eq!(schema["status"].options.len(), 2, "the new option survives");
            assert_eq!(page.scope().as_deref(), Some("children"));
        }
    }

    #[test]
    fn same_property_edits_are_last_writer_wins() {
        let status = PropertyDef {
            name: "Status".into(),
            kind: PropertyType::Status,
            order: 0,
            options: vec![OptionDef { id: "todo".into(), name: "To do".into(), color: None, group: Some("todo".into()) }],
            relation: None,
            date_format: None,
        };
        let base = PageDoc::new();
        let base_update = base.edit_records(&RecordsPatch::default().put_schema("status", &status)).unwrap().expect("a base");
        let left = PageDoc::new();
        let right = PageDoc::new();
        left.apply(&base_update).unwrap();
        right.apply(&base_update).unwrap();
        let renamed = left.edit_records(&RecordsPatch::default().put_schema("status", &PropertyDef { name: "State".into(), ..status.clone() })).unwrap().unwrap();
        let more = right
            .edit_records(&RecordsPatch::default().put_schema(
                "status",
                &PropertyDef { options: vec![status.options[0].clone(), OptionDef { id: "done".into(), name: "Done".into(), color: None, group: Some("done".into()) }], ..status.clone() },
            ))
            .unwrap()
            .unwrap();
        left.apply(&more).unwrap();
        right.apply(&renamed).unwrap();
        assert_eq!(left.schema(), right.schema(), "two clients converge on one value");
        let def = left.schema().remove("status").unwrap();
        assert!(def.name == "Status" || def.name == "State", "one of the two edits wins the entry: {def:?}");
        assert!(def.options.len() == 1 || def.options.len() == 2, "{def:?}");
    }

    #[test]
    fn records_replay_and_a_plain_page_grows_no_empty_maps() {
        let (_dir, db) = db();
        let page = PageDoc::new();
        log(&db, "p1", page.set_body("words").unwrap());
        assert!(page.edit_records(&RecordsPatch::default()).unwrap().is_none(), "an empty patch writes nothing");
        let mut records = RecordsPatch::default().scope("space");
        records = records.put_value("rating", Some(&serde_json::json!(5)));
        log(&db, "p1", page.edit_records(&records).unwrap());
        assert!(page.edit_records(&records).unwrap().is_none(), "the same values write no update");
        records = records.put_value("rating", None);
        log(&db, "p1", page.edit_records(&records).unwrap());
        let (replayed, seq) = load_page(&db, "p1", None).unwrap();
        assert_eq!(seq, 3);
        assert_eq!(replayed.scope().as_deref(), Some("space"));
        assert!(replayed.values().is_empty());
        assert_eq!(replayed.body(), "words");
    }
}
