//! Bridge Notes: local pages that people and agents both write.
//!
//! A page's body ("Your words") is only ever changed by a person or by an agent
//! they asked; model passes (Structure, Turn into tasks) run only on request
//! and write beside the body, never into it. Projects are optional everywhere.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// One page in a list: everything but the full body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Note {
    pub id: String,
    pub space_id: String,
    pub parent_id: Option<String>,
    /// Set on a note split out of another note's structured view.
    pub source_id: Option<String>,
    pub title: Option<String>,
    /// `note`, `idea`, `task`, `question`, or `database`.
    pub kind: String,
    pub tags: Vec<String>,
    /// The linked project, if the person chose one.
    pub workspace_id: Option<String>,
    /// `inbox`, `open`, `doing`, `done`, or `archived`.
    pub state: String,
    /// Chats started from this page.
    pub session_ids: Vec<String>,
    /// For a task: the verbatim span of its parent page it came from.
    pub source_excerpt: Option<String>,
    /// The first few hundred characters of the body.
    pub preview: String,
    /// The Markdown file this page is mirrored to.
    pub path: Option<String>,
    /// The page's latest change number; a save sends it back as `baseSeq`.
    pub seq: i64,
    /// `person`, `file-import`, `agent:<harness>:<sessionId>`, or `model:<pass>:<model>`.
    pub created_by: String,
    pub last_actor: String,
    pub created_at: String,
    pub updated_at: String,
}

/// One item the Structure pass found, quoting the note.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct StructuredItem {
    pub excerpt: String,
    pub title: String,
    pub kind: String,
}

/// The opt-in second view of a note. Never replaces the body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct StructuredView {
    pub title: String,
    pub kind: String,
    pub tags: Vec<String>,
    pub summary: String,
    pub items: Vec<StructuredItem>,
    /// A suggested project; `None` when nothing fit or none was listed.
    pub workspace_id: Option<String>,
    /// The body changed after this view was built.
    pub stale: bool,
    pub harness: Option<String>,
    pub model: Option<String>,
    pub created_at: String,
}

/// One live page linking to a page, with its quoting line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct NoteBacklink {
    pub page_id: String,
    pub title: String,
    pub kind: String,
    pub excerpt: String,
    pub updated_at: String,
}

/// A page with its full body, its structured view and its tasks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct NotePage {
    pub note: Note,
    /// Your words, as Bridge Markdown.
    pub markdown: String,
    pub seq: i64,
    pub structured: Option<StructuredView>,
    pub tasks: Vec<Note>,
    pub backlinks: Vec<NoteBacklink>,
    /// On a database page: its scope, properties and views.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database: Option<NoteDatabase>,
    /// On a row of a database: the database it belongs to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database_id: Option<String>,
    /// On a row: its cells by property id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub values: Option<BTreeMap<String, serde_json::Value>>,
}

/// One choice of a select, multi-select or status property.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct NoteOption {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// Status only: `todo`, `inProgress` or `done`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
}

/// The database a relation property points at.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct NoteRelation {
    pub database_id: String,
}

/// One column of a database.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct NoteProperty {
    pub id: String,
    pub name: String,
    /// `text`, `number`, `select`, `multiSelect`, `date`, `checkbox`, `url`,
    /// `relation` or `status`.
    #[serde(rename = "type")]
    pub kind: String,
    pub order: i64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<NoteOption>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relation: Option<NoteRelation>,
    /// Dates only: `date` or `datetime`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub date_format: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NoteSort {
    /// A property id or a system property (`title`, `kind`, `state`, …).
    pub property: String,
    /// `asc` or `desc`.
    pub direction: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NoteGroupBy {
    pub property: String,
    /// Dates only: `day`, `week` or `month`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub date_bucket: Option<String>,
}

/// How a view shows one property.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NoteViewProperty {
    pub id: String,
    pub visible: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<i64>,
}

/// One saved view of a database.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct NoteDatabaseView {
    pub id: String,
    pub name: String,
    /// `table`, `board`, `calendar` or `list`.
    #[serde(rename = "type")]
    pub kind: String,
    pub order: i64,
    /// The filter tree: `{and: [...]}`, `{or: [...]}` or `{property, op, value?}`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sorts: Vec<NoteSort>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_by: Option<NoteGroupBy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub date_property: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub properties: Vec<NoteViewProperty>,
}

/// A database: its properties in order and its views in order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct NoteDatabase {
    pub id: String,
    /// `children` (its rows are its child pages) or `space` (every page).
    pub scope: String,
    pub properties: Vec<NoteProperty>,
    pub views: Vec<NoteDatabaseView>,
}

/// One row a query matched: the page and its cells by property id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseRow {
    pub note: Note,
    pub values: BTreeMap<String, serde_json::Value>,
}

/// One group of a grouped query, with its first rows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseGroup {
    /// An option id, a fixed value, a date bucket, or `""` for no value.
    pub key: String,
    pub label: String,
    pub count: u32,
    pub rows: Vec<DatabaseRow>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseQueryResult {
    pub rows: Vec<DatabaseRow>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub groups: Option<Vec<DatabaseGroup>>,
    /// Every matching row, across pages.
    pub total: u32,
    /// Absent on the last page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

/// An option in a property being added or an options list being replaced.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NoteOptionInput {
    /// An existing option's id; absent for a new option.
    #[serde(default)]
    pub id: Option<String>,
    pub name: String,
    #[serde(default)]
    pub color: Option<String>,
    /// Status only: `todo`, `inProgress` or `done`.
    #[serde(default)]
    pub group: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NotePropertyInput {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub options: Option<Vec<NoteOptionInput>>,
    /// A relation's target database id.
    #[serde(default)]
    pub relation: Option<String>,
    #[serde(default)]
    pub date_format: Option<String>,
}

/// A view to save. Without `id` it is created; with one it is replaced.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NoteViewInput {
    #[serde(default)]
    pub id: Option<String>,
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub order: Option<i64>,
    #[serde(default)]
    pub filter: Option<serde_json::Value>,
    #[serde(default)]
    pub sorts: Option<Vec<NoteSort>>,
    #[serde(default)]
    pub group_by: Option<NoteGroupBy>,
    #[serde(default)]
    pub date_property: Option<String>,
    #[serde(default)]
    pub properties: Option<Vec<NoteViewProperty>>,
}

/// One change to a database's schema.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "op", rename_all = "camelCase", deny_unknown_fields)]
pub enum NoteSchemaChange {
    Add {
        property: NotePropertyInput,
    },
    Rename {
        #[serde(rename = "propertyId")]
        property_id: String,
        name: String,
    },
    /// Convert values where possible; anything else is cleared.
    Retype {
        #[serde(rename = "propertyId")]
        property_id: String,
        #[serde(rename = "type")]
        kind: String,
    },
    Delete {
        #[serde(rename = "propertyId")]
        property_id: String,
    },
    /// Replace the whole option list: no id is new, a missing id is deleted.
    SetOptions {
        #[serde(rename = "propertyId")]
        property_id: String,
        options: Vec<NoteOptionInput>,
        /// The option ids, in order, the caller read. When given, the write is
        /// refused unless they are exactly what is stored, so a client that
        /// replaces the list can read again and retry instead of deleting
        /// options someone else added.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        base: Option<Vec<String>>,
    },
    /// The schema order, naming every property once.
    Reorder {
        #[serde(rename = "propertyIds")]
        property_ids: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct NoteEditResult {
    pub note: Note,
    pub seq: i64,
    pub warnings: Vec<String>,
}

/// A model pass queued or running.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct NoteJob {
    pub id: String,
    pub page_id: String,
    /// `structure` or `tasks`.
    pub kind: String,
    /// `pending`, `running`, `done`, or `failed`.
    pub status: String,
    pub detail: Option<String>,
    pub harness: Option<String>,
    pub model: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct NotesList {
    pub notes: Vec<Note>,
    /// Model jobs that are pending, running, or failed in the last day.
    pub jobs: Vec<NoteJob>,
    /// The folder pages are mirrored to.
    pub root: String,
    /// The database whose saved views are the Notes list's tabs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_database_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct NoteJobQueued {
    pub job_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct NotesSettings {
    /// Structure every new capture without asking. Off by default.
    pub auto_structure: bool,
    pub root: String,
}

/// A targeted replacement: `old_str` must match the body exactly once.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NoteTextEdit {
    pub old_str: String,
    pub new_str: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NoteAppend {
    pub content: String,
    /// `start` or `end`.
    #[serde(default = "default_append_position")]
    pub position: String,
}

fn default_append_position() -> String {
    "end".into()
}

/// Delivered as `notes/page_changed` after every write.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct NotePageChanged {
    pub page_id: String,
    pub actor: String,
    pub seq: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ListNotesParams {
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub parent_id: Option<String>,
    #[serde(default)]
    pub include_archived: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReadPageParams {
    pub id: String,
}

/// Save Your words exactly as typed. No model is called.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CaptureNoteParams {
    pub body: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreatePageParams {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub parent_id: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    /// Optional project link. Never required.
    #[serde(default)]
    pub workspace_id: Option<String>,
}

/// Exactly one of `text` (a person's save, merged against `base_seq`),
/// `edits` (targeted replacements) or `append`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EditPageParams {
    pub id: String,
    #[serde(default)]
    pub base_seq: Option<i64>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub edits: Option<Vec<NoteTextEdit>>,
    #[serde(default)]
    pub append: Option<NoteAppend>,
    /// Allow the edit to remove child page or database tags. Only after the
    /// person confirmed.
    #[serde(default)]
    pub allow_deleting_children: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateNoteParams {
    pub id: String,
    #[serde(default)]
    pub parent_id: Option<String>,
    #[serde(default)]
    pub clear_parent: bool,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    #[serde(default)]
    pub workspace_id: Option<String>,
    #[serde(default)]
    pub clear_workspace: Option<bool>,
    #[serde(default)]
    pub state: Option<String>,
    /// Record a chat started from this page.
    #[serde(default)]
    pub add_session_id: Option<String>,
    /// Set (a value) or clear (`null`) a row's cells, by property id.
    #[serde(default)]
    pub values: Option<BTreeMap<String, serde_json::Value>>,
}

/// Make a database. With no properties it gets a Status; with no views, one
/// table.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateDatabaseParams {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub parent_id: Option<String>,
    #[serde(default)]
    pub workspace_id: Option<String>,
    #[serde(default)]
    pub properties: Option<Vec<NotePropertyInput>>,
    /// First views. They may name the properties being created, and their
    /// options, by name: the ids are made in the same write.
    #[serde(default)]
    pub views: Option<Vec<NoteViewInput>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateSchemaParams {
    pub id: String,
    pub changes: Vec<NoteSchemaChange>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SaveViewParams {
    pub database_id: String,
    pub view: NoteViewInput,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeleteViewParams {
    pub database_id: String,
    pub view_id: String,
}

/// Filter, sort, group and page a database's rows. A `viewId` supplies each of
/// `filter`, `sorts` and `groupBy` the call leaves out; `null` is the same as
/// leaving a field out. An explicit `sorts: []` or `filter: {and: []}` replaces
/// the view's. To run a view without its grouping, query without `viewId` and
/// pass the view's filter and sorts.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QueryDatabaseParams {
    pub database_id: String,
    #[serde(default)]
    pub view_id: Option<String>,
    #[serde(default)]
    pub filter: Option<serde_json::Value>,
    #[serde(default)]
    pub sorts: Option<Vec<NoteSort>>,
    #[serde(default)]
    pub group_by: Option<NoteGroupBy>,
    #[serde(default)]
    pub group_limit: Option<u32>,
    /// Words to look for, intersected with the filter.
    #[serde(default)]
    pub query: Option<String>,
    /// The caller's local day, `YYYY-MM-DD`, for relative date filters.
    #[serde(default)]
    pub today: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
    #[serde(default)]
    pub cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeleteNoteParams {
    pub id: String,
}

/// Ask for the structured view. Runs only on request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StructureNoteParams {
    pub id: String,
    #[serde(default)]
    pub harness: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
}

/// Turn a page into task pages. Runs only on request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExtractTasksParams {
    pub id: String,
    #[serde(default)]
    pub harness: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SplitItemParams {
    pub id: String,
    pub item_index: u32,
}

/// One stored version of a page, as its history lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct NoteVersion {
    pub seq: i64,
    /// `person`, `agent:<harness>:<session>`, `file-import` or `model:<pass>:<model>`.
    pub actor: String,
    /// When the version was written (RFC 3339, from its clock stamp).
    pub at: String,
    /// False for a change to properties only (title, status, tags, project).
    pub body_changed: bool,
    /// Characters added and removed against the version before.
    pub added: u32,
    pub removed: u32,
    pub title: Option<String>,
}

/// A page's versions, newest first, at most the newest 200.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct NoteHistory {
    pub versions: Vec<NoteVersion>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PageHistoryParams {
    pub id: String,
}

/// A page as it was at one version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct NoteVersionPage {
    pub seq: i64,
    pub actor: String,
    pub at: String,
    pub title: Option<String>,
    pub markdown: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReadVersionParams {
    pub id: String,
    pub seq: i64,
}

/// Write a version's words back as a new change by the person.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RestoreVersionParams {
    pub id: String,
    pub seq: i64,
}

/// Append one part of an image to an upload; without `uploadId` it starts one.
/// Parts keep each request under the daemon's frame cap.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PutAssetPartParams {
    #[serde(default)]
    pub upload_id: Option<String>,
    /// Base64 bytes.
    pub data: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct NoteAssetPart {
    pub upload_id: String,
    /// Bytes received so far.
    pub size: u64,
}

/// Store an upload as a page asset.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FinishAssetParams {
    pub upload_id: String,
    pub mime: String,
}

/// A stored asset; `path` (`assets/<hash>.<ext>`) is what a page links to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct NoteAsset {
    pub path: String,
    pub mime: String,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReadAssetParams {
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct NoteAssetData {
    pub path: String,
    pub mime: String,
    /// Base64 bytes.
    pub data: String,
}

/// Delivered as `notes/pass_progress` while a Structure or Turn into tasks
/// pass runs: the model's reply so far. Nothing is stored until it settles.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct NotePassProgress {
    pub job_id: String,
    pub page_id: String,
    /// `structure` or `tasks`.
    pub kind: String,
    pub text: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SaveNotesSettingsParams {
    #[serde(default)]
    pub auto_structure: Option<bool>,
    #[serde(default)]
    pub root: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notes_params_deserialize_from_only_their_required_fields() {
        let list: ListNotesParams = serde_json::from_str("{}").unwrap();
        assert_eq!((list.tags, list.include_archived), (None, None));
        let create: CreatePageParams = serde_json::from_str("{}").unwrap();
        assert_eq!(create.tags, None);
        let edit: EditPageParams = serde_json::from_str(r#"{"id":"p1"}"#).unwrap();
        assert_eq!((edit.edits, edit.allow_deleting_children), (None, None));
        let update: UpdateNoteParams = serde_json::from_str(r#"{"id":"p1"}"#).unwrap();
        assert_eq!(update.clear_workspace, None);
        let append: NoteAppend = serde_json::from_str(r#"{"content":"x"}"#).unwrap();
        assert_eq!(append.position, "end");
    }
}
