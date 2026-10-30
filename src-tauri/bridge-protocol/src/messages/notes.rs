//! Bridge Notes: local pages that people and agents both write.
//!
//! A page's body ("Your words") is only ever changed by a person or by an agent
//! they asked; model passes (Structure, Turn into tasks) run only on request
//! and write beside the body, never into it. Projects are optional everywhere.

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
    /// `note`, `idea`, `task`, or `question`.
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
