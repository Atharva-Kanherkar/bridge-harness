//! The Notes service: the one write path for people, agents and model passes.
//!
//! Every change goes through [`NotesService::write`]: rebuild the page doc,
//! make the change as one Yjs update, append it to `page_updates`, refresh the
//! `pages` index row, all in one transaction; then mirror the page to its
//! Markdown file and announce `notes/page_changed`. Nothing else writes
//! `pages` or `page_updates`, so the index can always be rebuilt from the log.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use bridge_protocol::messages::{Note, NoteAppend, NoteEditResult, NotePage, NotePageChanged, NoteTextEdit, NotesSettings, StructuredView};
use chrono::{TimeZone, Utc};
use rusqlite::{params, Connection, OptionalExtension, Row};
use sha2::{Digest, Sha256};

use super::doc::{self, key, PageDoc, PropsPatch};
use super::ids::{new_id, Hlc};
use super::{jobs, projector, store};
use crate::BridgeError;

pub const KINDS: [&str; 4] = ["note", "idea", "task", "question"];
pub const STATES: [&str; 5] = ["inbox", "open", "doing", "done", "archived"];
pub const PERSON: &str = "person";
pub const FILE_IMPORT: &str = "file-import";
const PREVIEW_CHARS: usize = 280;

/// Called after every committed change.
pub type ChangeListener = Arc<dyn Fn(&NotePageChanged) + Send + Sync>;
/// Answers whether a Bridge workspace (project) id exists.
pub type ProjectCheck = Arc<dyn Fn(&str) -> bool + Send + Sync>;

pub fn agent_actor(harness: &str, session_id: &str) -> String {
    format!("agent:{harness}:{session_id}")
}

pub fn model_actor(pass: &str, model: &str) -> String {
    format!("model:{pass}:{model}")
}

pub(crate) fn invalid(message: impl Into<String>) -> BridgeError {
    BridgeError::Invalid(message.into())
}

pub fn body_hash(body: &str) -> String {
    let digest = Sha256::digest(body.as_bytes());
    digest.iter().take(16).map(|byte| format!("{byte:02x}")).collect()
}

#[derive(Debug, Clone, Default)]
pub struct CreatePage {
    pub title: Option<String>,
    pub content: Option<String>,
    pub parent_id: Option<String>,
    pub kind: Option<String>,
    pub tags: Vec<String>,
    pub workspace_id: Option<String>,
    pub source_id: Option<String>,
    pub source_excerpt: Option<String>,
    pub state: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct EditPage {
    pub id: String,
    pub base_seq: Option<i64>,
    pub text: Option<String>,
    pub edits: Vec<NoteTextEdit>,
    pub append: Option<NoteAppend>,
    pub allow_deleting_children: bool,
}

#[derive(Debug, Clone, Default)]
pub struct UpdateNote {
    pub id: String,
    pub title: Option<String>,
    pub kind: Option<String>,
    pub tags: Option<Vec<String>>,
    pub workspace_id: Option<String>,
    pub clear_workspace: bool,
    pub state: Option<String>,
    pub add_session_id: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ListFilter {
    pub query: Option<String>,
    pub tags: Vec<String>,
    pub kind: Option<String>,
    pub state: Option<String>,
    pub parent_id: Option<String>,
    pub include_archived: bool,
    pub limit: Option<usize>,
}

pub struct NotesService {
    db: Mutex<Connection>,
    clock: Mutex<Hlc>,
    device: String,
    space: String,
    root: Mutex<PathBuf>,
    project_exists: ProjectCheck,
    listener: Mutex<Option<ChangeListener>>,
}

impl NotesService {
    /// Open `notes.db` in `data_dir`, mirroring pages under `data_dir/notes`.
    pub fn open(data_dir: &Path, project_exists: ProjectCheck) -> Result<Self, BridgeError> {
        Self::open_at(&data_dir.join(store::FILE_NAME), data_dir.join("notes"), project_exists)
    }

    pub fn open_at(db_path: &Path, default_root: PathBuf, project_exists: ProjectCheck) -> Result<Self, BridgeError> {
        let connection = store::open(db_path)?;
        let device = store::device_id(&connection)?;
        let space = store::personal_space(&connection)?;
        let newest = store::newest_stamp(&connection, &device)?;
        let root = store::get_setting(&connection, "root")?.map(PathBuf::from).unwrap_or(default_root);
        jobs::recover(&connection)?;
        Ok(Self {
            clock: Mutex::new(Hlc::resume(device.clone(), newest.as_deref())),
            db: Mutex::new(connection),
            device,
            space,
            root: Mutex::new(root),
            project_exists,
            listener: Mutex::new(None),
        })
    }

    pub fn root(&self) -> PathBuf {
        self.root.lock().unwrap().clone()
    }

    pub fn settings(&self) -> Result<NotesSettings, BridgeError> {
        let auto = self.with_db(|db| store::get_setting(db, "auto_structure"))?;
        Ok(NotesSettings { auto_structure: auto.as_deref() == Some("true"), root: self.root().to_string_lossy().into_owned() })
    }

    /// Change settings. A new root applies to pages mirrored from now on;
    /// files already written stay where they are.
    pub fn save_settings(&self, auto_structure: Option<bool>, root: Option<String>) -> Result<NotesSettings, BridgeError> {
        if let Some(root) = root.as_deref().map(str::trim).filter(|root| !root.is_empty()) {
            let path = PathBuf::from(root);
            if !path.is_absolute() {
                return Err(invalid("the notes folder must be an absolute path"));
            }
            std::fs::create_dir_all(&path)?;
            self.with_db(|db| store::set_setting(db, "root", root))?;
            *self.root.lock().unwrap() = path;
        }
        if let Some(auto) = auto_structure {
            self.with_db(|db| store::set_setting(db, "auto_structure", if auto { "true" } else { "false" }))?;
        }
        self.settings()
    }

    /// Queue an opt-in model pass on a page.
    pub fn request_pass(&self, id: &str, kind: &str, harness: Option<&str>, model: Option<&str>) -> Result<String, BridgeError> {
        self.check_live(id)?;
        self.with_db(|db| jobs::enqueue(db, &self.space, id, kind, harness, model))
    }

    /// Make one structured item its own note, quoting where it came from.
    pub fn split_item(&self, id: &str, item_index: usize, actor: &str) -> Result<Note, BridgeError> {
        let page = self.read(id)?;
        let view = page.structured.ok_or_else(|| invalid("this note has no structured view yet"))?;
        let item = view.items.get(item_index).ok_or_else(|| invalid("no such item"))?;
        self.create(
            CreatePage {
                title: Some(item.title.clone()),
                content: Some(item.excerpt.clone()),
                kind: Some(item.kind.clone()),
                source_id: Some(id.to_owned()),
                state: Some("inbox".into()),
                ..Default::default()
            },
            actor,
        )
    }

    pub fn space(&self) -> &str {
        &self.space
    }

    pub fn set_listener(&self, listener: ChangeListener) {
        *self.listener.lock().unwrap() = Some(listener);
    }

    /// Run `read` against the database. For queries and the job queue; page
    /// writes go through [`Self::write`].
    pub fn with_db<T>(&self, read: impl FnOnce(&Connection) -> Result<T, BridgeError>) -> Result<T, BridgeError> {
        read(&self.db.lock().unwrap())
    }

    /// Save Your words exactly as typed. Calls no model.
    pub fn capture(&self, body: &str, actor: &str) -> Result<Note, BridgeError> {
        if body.trim().is_empty() {
            return Err(invalid("a note needs some text"));
        }
        let note = self.create(
            CreatePage { content: Some(body.to_owned()), state: Some("inbox".into()), ..Default::default() },
            actor,
        )?;
        if self.settings()?.auto_structure {
            self.request_pass(&note.id, jobs::STRUCTURE, None, None)?;
        }
        Ok(note)
    }

    pub fn create(&self, page: CreatePage, actor: &str) -> Result<Note, BridgeError> {
        let kind = page.kind.unwrap_or_else(|| "note".into());
        check_kind(&kind)?;
        let state = page.state.unwrap_or_else(|| "open".into());
        check_state(&state)?;
        if let Some(workspace) = &page.workspace_id {
            self.check_project(workspace)?;
        }
        if let Some(parent) = &page.parent_id {
            self.check_live(parent)?;
        }
        let title = page.title.map(|title| title.trim().to_owned()).filter(|title| !title.is_empty());
        let tags = normalize_tags(&page.tags);
        let id = new_id();
        let body = page.content.unwrap_or_default();
        let space = self.space.clone();
        let (note, _, _) = self.write(&id, actor, |doc| {
            let patch = PropsPatch::default()
                .set(key::SPACE, space)
                .maybe(key::TITLE, title)
                .set(key::KIND, kind)
                .list(key::TAGS, &tags)
                .maybe(key::WORKSPACE, page.workspace_id)
                .set(key::STATE, state)
                .maybe(key::PARENT, page.parent_id)
                .maybe(key::SOURCE, page.source_id)
                .maybe(key::EXCERPT, page.source_excerpt)
                .set(key::CREATED_BY, actor)
                .set(key::CREATED_AT, now());
            doc.edit(Some(&body), &patch)
        })?;
        Ok(note)
    }

    pub fn edit(&self, edit: EditPage, actor: &str) -> Result<NoteEditResult, BridgeError> {
        self.check_live(&edit.id)?;
        let modes = [edit.text.is_some(), !edit.edits.is_empty(), edit.append.is_some()];
        if modes.iter().filter(|mode| **mode).count() != 1 {
            return Err(invalid("send exactly one of text, edits or append"));
        }
        let mut warnings = Vec::new();
        let allow = edit.allow_deleting_children;
        let (note, seq, changed) = if let Some(text) = edit.text {
            let base_seq = edit.base_seq;
            self.write_with(&edit.id, actor, |db, live, current_seq| {
                let base = base_seq.unwrap_or(current_seq).min(current_seq);
                check_children_kept(&live.body(), &text, allow)?;
                let update = doc::merge_save(db, &edit.id, base, &text)?;
                if let Some(update) = &update {
                    live.apply(update)?;
                }
                Ok(update)
            })?
        } else if !edit.edits.is_empty() {
            self.write(&edit.id, actor, |live| {
                let current = live.body();
                let next = apply_edits(&current, &edit.edits)?;
                check_children_kept(&current, &next, allow)?;
                live.set_body(&next)
            })?
        } else {
            let append = edit.append.unwrap();
            if append.content.is_empty() {
                return Err(invalid("nothing to append"));
            }
            self.write(&edit.id, actor, |live| {
                let current = live.body();
                match append.position.as_str() {
                    "start" => {
                        let mut chunk = append.content.clone();
                        if !current.is_empty() && !chunk.ends_with('\n') {
                            chunk.push('\n');
                        }
                        live.insert_body(0, &chunk)
                    }
                    "end" => {
                        let mut chunk = String::new();
                        if !current.is_empty() && !current.ends_with('\n') {
                            chunk.push('\n');
                        }
                        chunk.push_str(&append.content);
                        live.insert_body(current.len(), &chunk)
                    }
                    other => Err(invalid(format!("append position must be start or end, not {other}"))),
                }
            })?
        };
        if !changed {
            warnings.push("nothing changed".to_owned());
        }
        Ok(NoteEditResult { note, seq, warnings })
    }

    pub fn update(&self, update: UpdateNote, actor: &str) -> Result<Note, BridgeError> {
        self.check_live(&update.id)?;
        if let Some(kind) = &update.kind {
            check_kind(kind)?;
        }
        if let Some(state) = &update.state {
            check_state(state)?;
        }
        if let Some(workspace) = &update.workspace_id {
            self.check_project(workspace)?;
        }
        let (note, _, _) = self.write(&update.id, actor, |live| {
            let props = live.props();
            let mut patch = PropsPatch::default();
            if let Some(title) = update.title {
                let title = title.trim().to_owned();
                patch = patch.maybe(key::TITLE, (!title.is_empty()).then_some(title));
            }
            if let Some(kind) = update.kind {
                patch = patch.set(key::KIND, kind);
            }
            if let Some(tags) = update.tags {
                patch = patch.list(key::TAGS, &normalize_tags(&tags));
            }
            if update.clear_workspace {
                patch = patch.clear(key::WORKSPACE);
            } else if let Some(workspace) = update.workspace_id {
                patch = patch.set(key::WORKSPACE, workspace);
            }
            if let Some(state) = update.state {
                patch = patch.set(key::STATE, state);
            }
            if let Some(session) = update.add_session_id.filter(|session| !session.trim().is_empty()) {
                let mut sessions = props.session_ids.clone();
                if !sessions.contains(&session) {
                    sessions.push(session);
                }
                patch = patch.list(key::SESSIONS, &sessions);
            }
            live.set_props(&patch)
        })?;
        Ok(note)
    }

    /// Tombstone a page and its tasks.
    pub fn delete(&self, id: &str, actor: &str) -> Result<(), BridgeError> {
        self.check_live(id)?;
        let tasks: Vec<String> = self.with_db(|db| {
            let mut statement = db.prepare("SELECT id FROM pages WHERE parent_id=?1 AND kind='task' AND deleted_at IS NULL")?;
            let ids = statement.query_map(params![id], |row| row.get(0))?.collect::<Result<_, _>>()?;
            Ok(ids)
        })?;
        let stamp = now();
        for page in tasks.iter().map(String::as_str).chain(std::iter::once(id)) {
            self.write(page, actor, |live| live.set_props(&PropsPatch::default().set(key::DELETED_AT, stamp.clone())))?;
        }
        Ok(())
    }

    pub fn read(&self, id: &str) -> Result<NotePage, BridgeError> {
        self.with_db(|db| {
            let (note, body, hash) = db
                .query_row(&format!("SELECT {NOTE_COLUMNS}, body, body_hash FROM pages WHERE id=?1 AND deleted_at IS NULL"), params![id], |row| {
                    Ok((note_from_row(row)?, row.get::<_, String>(19)?, row.get::<_, String>(20)?))
                })
                .optional()?
                .ok_or_else(|| invalid("note not found"))?;
            let structured = read_structured(db, id, &hash)?;
            let mut statement = db.prepare(&format!(
                "SELECT {NOTE_COLUMNS} FROM pages WHERE parent_id=?1 AND kind='task' AND deleted_at IS NULL ORDER BY created_at, id"
            ))?;
            let tasks = statement.query_map(params![id], note_from_row)?.collect::<Result<_, _>>()?;
            Ok(NotePage { seq: note.seq, note, markdown: body, structured, tasks })
        })
    }

    pub fn list(&self, filter: &ListFilter) -> Result<Vec<Note>, BridgeError> {
        self.with_db(|db| {
            let mut sql = format!("SELECT {NOTE_COLUMNS} FROM pages WHERE deleted_at IS NULL AND space_id=?1");
            let mut args: Vec<rusqlite::types::Value> = vec![self.space.clone().into()];
            let mut bind = |sql: &mut String, clause: &str, value: String| {
                args.push(value.into());
                sql.push_str(&clause.replace('?', &format!("?{}", args.len())));
            };
            if !filter.include_archived && filter.state.as_deref() != Some("archived") {
                sql.push_str(" AND state<>'archived'");
            }
            if let Some(query) = filter.query.as_deref().map(str::trim).filter(|query| !query.is_empty()) {
                let pattern = format!("%{}%", query.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_"));
                bind(&mut sql, " AND (title LIKE ? ESCAPE '\\' OR body LIKE ? ESCAPE '\\')", pattern);
            }
            for tag in normalize_tags(&filter.tags) {
                bind(&mut sql, " AND EXISTS (SELECT 1 FROM json_each(pages.tags) WHERE value=?)", tag);
            }
            if let Some(kind) = &filter.kind {
                bind(&mut sql, " AND kind=?", kind.clone());
            }
            if let Some(state) = &filter.state {
                bind(&mut sql, " AND state=?", state.clone());
            }
            if let Some(parent) = &filter.parent_id {
                bind(&mut sql, " AND parent_id=?", parent.clone());
            }
            sql.push_str(&format!(" ORDER BY updated_at DESC, id DESC LIMIT {}", filter.limit.unwrap_or(500).min(2000)));
            let mut statement = db.prepare(&sql)?;
            let notes = statement.query_map(rusqlite::params_from_iter(args.iter()), note_from_row)?.collect::<Result<_, _>>()?;
            Ok(notes)
        })
    }

    /// Drop the index and rebuild every page from `page_updates`.
    pub fn rebuild_index(&self) -> Result<usize, BridgeError> {
        let db = self.db.lock().unwrap();
        let transaction = db.unchecked_transaction()?;
        let paths: Vec<(String, Option<String>, Option<String>, Option<i64>)> = {
            let mut statement = transaction.prepare("SELECT id, path, content_hash, projected_seq FROM pages")?;
            let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)))?.collect::<Result<_, _>>()?;
            rows
        };
        transaction.execute("DELETE FROM pages", [])?;
        let files = projector::find_files(&self.root());
        let pages: Vec<String> = {
            let mut statement = transaction.prepare("SELECT DISTINCT page_id FROM page_updates ORDER BY page_id")?;
            let rows = statement.query_map([], |row| row.get(0))?.collect::<Result<_, _>>()?;
            rows
        };
        for page in &pages {
            let (live, seq) = doc::load_page(&transaction, page, None)?;
            let (actor, hlc): (String, String) = transaction.query_row(
                "SELECT actor, hlc FROM page_updates WHERE page_id=?1 ORDER BY seq DESC LIMIT 1",
                params![page],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;
            upsert_index(&transaction, page, &live, seq, &actor, &stamp_time(&hlc), &self.space)?;
            if let Some((_, path, hash, projected)) = paths.iter().find(|(id, _, _, _)| id == page) {
                transaction.execute("UPDATE pages SET path=?2, content_hash=?3, projected_seq=?4 WHERE id=?1", params![page, path, hash, projected])?;
            } else if let Some((path, hash)) = files.get(page) {
                // The index was lost with its paths; the files still name their pages.
                transaction.execute(
                    "UPDATE pages SET path=?2, content_hash=?3, projected_seq=?4 WHERE id=?1",
                    params![page, path.to_string_lossy(), hash, seq],
                )?;
            }
        }
        transaction.commit()?;
        Ok(pages.len())
    }

    fn check_project(&self, workspace: &str) -> Result<(), BridgeError> {
        if (self.project_exists)(workspace) {
            Ok(())
        } else {
            Err(invalid(format!("no project with id {workspace}")))
        }
    }

    fn check_live(&self, id: &str) -> Result<(), BridgeError> {
        let live: bool = self.with_db(|db| {
            Ok(db.query_row("SELECT EXISTS(SELECT 1 FROM pages WHERE id=?1 AND deleted_at IS NULL)", params![id], |row| row.get(0))?)
        })?;
        if live {
            Ok(())
        } else {
            Err(invalid("note not found"))
        }
    }

    /// The one write path. `change` makes at most one Yjs update on the live doc.
    pub(crate) fn write(
        &self,
        page_id: &str,
        actor: &str,
        change: impl FnOnce(&PageDoc) -> Result<Option<Vec<u8>>, BridgeError>,
    ) -> Result<(Note, i64, bool), BridgeError> {
        self.write_with(page_id, actor, |_, live, _| change(live))
    }

    fn write_with(
        &self,
        page_id: &str,
        actor: &str,
        change: impl FnOnce(&Connection, &PageDoc, i64) -> Result<Option<Vec<u8>>, BridgeError>,
    ) -> Result<(Note, i64, bool), BridgeError> {
        let (note, seq, changed, body) = {
            let db = self.db.lock().unwrap();
            let transaction = db.unchecked_transaction()?;
            let (live, current_seq) = doc::load_page(&transaction, page_id, None)?;
            let update = change(&transaction, &live, current_seq)?;
            let (seq, changed) = match update {
                Some(bytes) => {
                    let hlc = self.clock.lock().unwrap().now();
                    let seq = doc::append_update(&transaction, page_id, &bytes, actor, &hlc, &self.device)?;
                    doc::maybe_snapshot(&transaction, page_id, seq, &live)?;
                    upsert_index(&transaction, page_id, &live, seq, actor, &now(), &self.space)?;
                    (seq, true)
                }
                None => (current_seq, false),
            };
            transaction.commit()?;
            let (note, body, hash, deleted) = db.query_row(
                &format!("SELECT {NOTE_COLUMNS}, body, content_hash FROM pages WHERE id=?1"),
                params![page_id],
                |row| Ok((note_from_row(row)?, row.get::<_, String>(19)?, row.get::<_, Option<String>>(20)?, row.get::<_, Option<String>>(18)?.is_some())),
            )?;
            (note, seq, changed, (body, hash, deleted))
        };
        if !changed {
            return Ok((note, seq, false));
        }
        let (body, hash, deleted) = body;
        let note = self.mirror_page(note, &body, hash.as_deref(), deleted, seq)?;
        if let Some(listener) = self.listener.lock().unwrap().clone() {
            listener(&NotePageChanged { page_id: page_id.to_owned(), actor: actor.to_owned(), seq });
        }
        Ok((note, seq, true))
    }

    fn mirror_page(&self, note: Note, body: &str, previous_hash: Option<&str>, deleted: bool, seq: i64) -> Result<Note, BridgeError> {
        let input = projector::MirrorInput { note: &note, body, deleted, previous_path: note.path.as_deref(), previous_hash };
        let Some(mirrored) = projector::mirror(&self.root(), &input) else {
            return Ok(note);
        };
        self.with_db(|db| {
            db.execute(
                "UPDATE pages SET path=?2, content_hash=?3, projected_seq=?4 WHERE id=?1",
                params![note.id, mirrored.path, mirrored.hash, seq],
            )?;
            Ok(())
        })?;
        Ok(Note { path: mirrored.path, ..note })
    }

    /// The device this service stamps changes with.
    pub fn device(&self) -> &str {
        &self.device
    }
}

pub(crate) const NOTE_COLUMNS: &str = "id, space_id, parent_id, source_id, title, kind, tags, workspace_id, state, session_ids, source_excerpt, \
     substr(body, 1, 280), path, seq, created_by, last_actor, created_at, updated_at, deleted_at";

pub(crate) fn note_from_row(row: &Row<'_>) -> rusqlite::Result<Note> {
    let list = |index: usize| -> rusqlite::Result<Vec<String>> {
        Ok(serde_json::from_str(&row.get::<_, String>(index)?).unwrap_or_default())
    };
    let preview: String = row.get(11)?;
    Ok(Note {
        id: row.get(0)?,
        space_id: row.get(1)?,
        parent_id: row.get(2)?,
        source_id: row.get(3)?,
        title: row.get(4)?,
        kind: row.get(5)?,
        tags: list(6)?,
        workspace_id: row.get(7)?,
        state: row.get(8)?,
        session_ids: list(9)?,
        source_excerpt: row.get(10)?,
        preview: preview.chars().take(PREVIEW_CHARS).collect(),
        path: row.get(12)?,
        seq: row.get(13)?,
        created_by: row.get(14)?,
        last_actor: row.get(15)?,
        created_at: row.get(16)?,
        updated_at: row.get(17)?,
    })
}

fn upsert_index(db: &Connection, page_id: &str, live: &PageDoc, seq: i64, actor: &str, updated_at: &str, default_space: &str) -> Result<(), BridgeError> {
    let props = live.props();
    let body = live.body();
    let created_at = props.created_at.clone().unwrap_or_else(|| updated_at.to_owned());
    db.execute(
        "INSERT INTO pages(id, space_id, parent_id, source_id, title, kind, tags, workspace_id, state, session_ids, source_excerpt,
                           body, body_hash, seq, created_by, last_actor, created_at, updated_at, deleted_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19)
         ON CONFLICT(id) DO UPDATE SET
            space_id=excluded.space_id, parent_id=excluded.parent_id, source_id=excluded.source_id, title=excluded.title,
            kind=excluded.kind, tags=excluded.tags, workspace_id=excluded.workspace_id, state=excluded.state,
            session_ids=excluded.session_ids, source_excerpt=excluded.source_excerpt, body=excluded.body,
            body_hash=excluded.body_hash, seq=excluded.seq, last_actor=excluded.last_actor,
            updated_at=excluded.updated_at, deleted_at=excluded.deleted_at",
        params![
            page_id,
            props.space_id.unwrap_or_else(|| default_space.to_owned()),
            props.parent_id,
            props.source_id,
            props.title,
            props.kind.unwrap_or_else(|| "note".into()),
            serde_json::to_string(&props.tags).unwrap_or_else(|_| "[]".into()),
            props.workspace_id,
            props.state.unwrap_or_else(|| "open".into()),
            serde_json::to_string(&props.session_ids).unwrap_or_else(|_| "[]".into()),
            props.source_excerpt,
            body,
            body_hash(&body),
            seq,
            props.created_by.unwrap_or_else(|| actor.to_owned()),
            actor,
            created_at,
            updated_at,
            props.deleted_at,
        ],
    )?;
    Ok(())
}

fn read_structured(db: &Connection, page_id: &str, current_hash: &str) -> Result<Option<StructuredView>, BridgeError> {
    let row: Option<(String, String)> = db
        .query_row("SELECT body_hash, view FROM structured_views WHERE page_id=?1", params![page_id], |row| Ok((row.get(0)?, row.get(1)?)))
        .optional()?;
    Ok(row.and_then(|(hash, view)| {
        let mut view: StructuredView = serde_json::from_str(&view).ok()?;
        view.stale = hash != current_hash;
        Some(view)
    }))
}

fn now() -> String {
    Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// The wall time a clock stamp was taken at.
fn stamp_time(stamp: &str) -> String {
    let millis = stamp.split('-').next().and_then(|millis| millis.parse::<i64>().ok()).unwrap_or(0);
    Utc.timestamp_millis_opt(millis).single().unwrap_or_else(Utc::now).to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn check_kind(kind: &str) -> Result<(), BridgeError> {
    if KINDS.contains(&kind) {
        Ok(())
    } else {
        Err(invalid(format!("kind must be one of {}", KINDS.join(", "))))
    }
}

fn check_state(state: &str) -> Result<(), BridgeError> {
    if STATES.contains(&state) {
        Ok(())
    } else {
        Err(invalid(format!("state must be one of {}", STATES.join(", "))))
    }
}

/// Lowercase slugs, deduplicated, in order.
pub(crate) fn normalize_tags(tags: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for tag in tags {
        let tag = tag.trim().trim_start_matches('#').to_lowercase();
        if !tag.is_empty() && !out.contains(&tag) {
            out.push(tag);
        }
    }
    out
}

/// Apply targeted replacements in order; each must match exactly once.
fn apply_edits(body: &str, edits: &[NoteTextEdit]) -> Result<String, BridgeError> {
    let mut text = body.to_owned();
    for (index, edit) in edits.iter().enumerate() {
        if edit.old_str.is_empty() {
            return Err(invalid(format!("edit {} has an empty oldStr", index + 1)));
        }
        match text.matches(edit.old_str.as_str()).count() {
            0 => return Err(invalid(format!("edit {}: oldStr was not found; read the page and quote it exactly", index + 1))),
            1 => text = text.replacen(edit.old_str.as_str(), &edit.new_str, 1),
            n => return Err(invalid(format!("edit {}: oldStr matches {n} places; quote more of the surrounding text", index + 1))),
        }
    }
    Ok(text)
}

/// Child page and database tags the edit would drop, unless allowed.
fn check_children_kept(before: &str, after: &str, allow: bool) -> Result<(), BridgeError> {
    if allow {
        return Ok(());
    }
    let pattern = regex::Regex::new(r"<(?:page|database)\b[^>]*>").expect("valid pattern");
    let dropped: Vec<&str> = pattern.find_iter(before).map(|found| found.as_str()).filter(|tag| !after.contains(tag)).collect();
    if dropped.is_empty() {
        Ok(())
    } else {
        Err(invalid(format!(
            "this edit would delete {} child page or database tag(s): {}. Ask the person first, then retry with allowDeletingChildren.",
            dropped.len(),
            dropped.join(", ")
        )))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn service() -> (tempfile::TempDir, NotesService) {
        let dir = tempfile::tempdir().unwrap();
        let service = NotesService::open(dir.path(), Arc::new(|workspace: &str| workspace == "ws-known")).unwrap();
        (dir, service)
    }

    fn edit(id: &str) -> EditPage {
        EditPage { id: id.into(), ..Default::default() }
    }

    #[test]
    fn capture_stores_the_body_byte_for_byte() {
        let (_dir, notes) = service();
        let typed = "  helo wrld!! idea: a notes app,\n\tkeep my typos ✨  \n";
        let note = notes.capture(typed, PERSON).unwrap();
        assert_eq!(note.state, "inbox");
        assert_eq!(note.kind, "note");
        assert_eq!(note.workspace_id, None);
        assert_eq!(note.created_by, PERSON);
        assert_eq!(notes.read(&note.id).unwrap().markdown, typed);
        let jobs: i64 = notes.with_db(|db| Ok(db.query_row("SELECT COUNT(*) FROM model_jobs", [], |row| row.get(0))?)).unwrap();
        assert_eq!(jobs, 0, "capture must not queue a model pass");
    }

    #[test]
    fn capture_refuses_an_empty_body() {
        let (_dir, notes) = service();
        assert!(notes.capture("", PERSON).is_err());
        assert!(notes.capture(" \n\t ", PERSON).is_err());
        assert!(notes.list(&ListFilter::default()).unwrap().is_empty());
    }

    #[test]
    fn create_without_a_project_is_allowed_and_unknown_project_is_refused() {
        let (_dir, notes) = service();
        let free = notes.create(CreatePage { title: Some("No project".into()), ..Default::default() }, PERSON).unwrap();
        assert_eq!(free.workspace_id, None);
        assert_eq!(free.state, "open");
        let linked = notes.create(CreatePage { title: Some("Linked".into()), workspace_id: Some("ws-known".into()), ..Default::default() }, PERSON).unwrap();
        assert_eq!(linked.workspace_id.as_deref(), Some("ws-known"));
        assert!(notes.create(CreatePage { workspace_id: Some("ws-missing".into()), ..Default::default() }, PERSON).is_err());
        assert!(notes.create(CreatePage { parent_id: Some("no-such-page".into()), ..Default::default() }, PERSON).is_err());
        let cleared = notes.update(UpdateNote { id: linked.id.clone(), clear_workspace: true, ..Default::default() }, PERSON).unwrap();
        assert_eq!(cleared.workspace_id, None);
    }

    #[test]
    fn targeted_edit_requires_a_unique_match_and_is_all_or_nothing() {
        let (_dir, notes) = service();
        let page = notes.create(CreatePage { content: Some("- [ ] buy domain\n- [ ] buy domain\n- [ ] write landing copy\n".into()), ..Default::default() }, PERSON).unwrap();
        let ambiguous = notes.edit(EditPage { edits: vec![NoteTextEdit { old_str: "buy domain".into(), new_str: "buy bridge.dev".into() }], ..edit(&page.id) }, "agent:claude:s1");
        assert!(ambiguous.unwrap_err().to_string().contains("matches 2 places"));
        let partly_missing = notes.edit(
            EditPage {
                edits: vec![
                    NoteTextEdit { old_str: "write landing copy".into(), new_str: "write landing copy (draft)".into() },
                    NoteTextEdit { old_str: "not on the page".into(), new_str: "x".into() },
                ],
                ..edit(&page.id)
            },
            "agent:claude:s1",
        );
        assert!(partly_missing.is_err());
        assert_eq!(notes.read(&page.id).unwrap().markdown, "- [ ] buy domain\n- [ ] buy domain\n- [ ] write landing copy\n");
        let done = notes.edit(EditPage { edits: vec![NoteTextEdit { old_str: "- [ ] write landing copy".into(), new_str: "- [x] write landing copy".into() }], ..edit(&page.id) }, "agent:claude:s1").unwrap();
        assert_eq!(notes.read(&page.id).unwrap().markdown, "- [ ] buy domain\n- [ ] buy domain\n- [x] write landing copy\n");
        assert_eq!(done.note.last_actor, "agent:claude:s1");
    }

    #[test]
    fn edit_refuses_to_remove_a_child_page_tag() {
        let (_dir, notes) = service();
        let body = "Research\n\n<page url=\"bridge://p/123\">Sync libraries</page>\n";
        let page = notes.create(CreatePage { content: Some(body.into()), ..Default::default() }, PERSON).unwrap();
        let removal = notes.edit(EditPage { edits: vec![NoteTextEdit { old_str: "<page url=\"bridge://p/123\">Sync libraries</page>\n".into(), new_str: String::new() }], ..edit(&page.id) }, "agent:claude:s1");
        assert!(removal.unwrap_err().to_string().contains("allowDeletingChildren"));
        let save = notes.edit(EditPage { text: Some("Research\n".into()), base_seq: Some(page.seq), ..edit(&page.id) }, PERSON);
        assert!(save.is_err());
        assert_eq!(notes.read(&page.id).unwrap().markdown, body);
        notes.edit(EditPage { text: Some("Research\n".into()), base_seq: Some(page.seq), allow_deleting_children: true, ..edit(&page.id) }, PERSON).unwrap();
        assert_eq!(notes.read(&page.id).unwrap().markdown, "Research\n");
    }

    #[test]
    fn append_never_rewrites_existing_text() {
        let (_dir, notes) = service();
        let page = notes.create(CreatePage { content: Some("first line".into()), ..Default::default() }, PERSON).unwrap();
        notes.edit(EditPage { append: Some(NoteAppend { content: "- added at the end".into(), position: "end".into() }), ..edit(&page.id) }, "agent:codex:s2").unwrap();
        notes.edit(EditPage { append: Some(NoteAppend { content: "# Heading".into(), position: "start".into() }), ..edit(&page.id) }, "agent:codex:s2").unwrap();
        assert_eq!(notes.read(&page.id).unwrap().markdown, "# Heading\nfirst line\n- added at the end");
        assert!(notes.edit(EditPage { append: Some(NoteAppend { content: "x".into(), position: "middle".into() }), ..edit(&page.id) }, PERSON).is_err());
    }

    #[test]
    fn update_props_leaves_the_body_untouched() {
        let (_dir, notes) = service();
        let page = notes.capture("raw words, untouched", PERSON).unwrap();
        let updated = notes
            .update(UpdateNote { id: page.id.clone(), title: Some("  Titled  ".into()), kind: Some("idea".into()), tags: Some(vec!["#Launch".into(), "launch".into(), "Q4".into()]), state: Some("open".into()), ..Default::default() }, PERSON)
            .unwrap();
        assert_eq!(updated.title.as_deref(), Some("Titled"));
        assert_eq!(updated.kind, "idea");
        assert_eq!(updated.tags, vec!["launch", "q4"]);
        assert_eq!(notes.read(&page.id).unwrap().markdown, "raw words, untouched");
        assert!(notes.update(UpdateNote { id: page.id.clone(), kind: Some("essay".into()), ..Default::default() }, PERSON).is_err());
        assert!(notes.update(UpdateNote { id: page.id, state: Some("someday".into()), ..Default::default() }, PERSON).is_err());
    }

    #[test]
    fn add_session_id_is_a_set() {
        let (_dir, notes) = service();
        let page = notes.capture("start a chat from me", PERSON).unwrap();
        for session in ["s1", "s2", "s1"] {
            notes.update(UpdateNote { id: page.id.clone(), add_session_id: Some(session.into()), ..Default::default() }, PERSON).unwrap();
        }
        assert_eq!(notes.read(&page.id).unwrap().note.session_ids, vec!["s1", "s2"]);
    }

    #[test]
    fn delete_is_a_tombstone_and_takes_its_tasks() {
        let (_dir, notes) = service();
        let page = notes.create(CreatePage { title: Some("Launch".into()), content: Some("plan".into()), ..Default::default() }, PERSON).unwrap();
        let task = notes.create(CreatePage { title: Some("Buy domain".into()), kind: Some("task".into()), parent_id: Some(page.id.clone()), ..Default::default() }, PERSON).unwrap();
        let other = notes.capture("keep me", PERSON).unwrap();
        assert_eq!(notes.read(&page.id).unwrap().tasks.len(), 1);
        notes.delete(&page.id, PERSON).unwrap();
        assert!(notes.read(&page.id).is_err());
        assert!(notes.read(&task.id).is_err());
        let listed: Vec<String> = notes.list(&ListFilter::default()).unwrap().into_iter().map(|note| note.id).collect();
        assert_eq!(listed, vec![other.id]);
        let log_rows: i64 = notes.with_db(|db| Ok(db.query_row("SELECT COUNT(*) FROM page_updates WHERE page_id=?1", params![page.id], |row| row.get(0))?)).unwrap();
        assert_eq!(log_rows, 2, "a delete is one more logged change, not an erase");
    }

    #[test]
    fn every_write_publishes_page_changed() {
        let (_dir, notes) = service();
        let seen = Arc::new(Mutex::new(Vec::<NotePageChanged>::new()));
        let sink = seen.clone();
        notes.set_listener(Arc::new(move |change: &NotePageChanged| sink.lock().unwrap().push(change.clone())));
        let page = notes.capture("one", PERSON).unwrap();
        notes.edit(EditPage { append: Some(NoteAppend { content: "two".into(), position: "end".into() }), ..edit(&page.id) }, "agent:claude:s9").unwrap();
        notes.update(UpdateNote { id: page.id.clone(), title: Some("T".into()), ..Default::default() }, PERSON).unwrap();
        notes.update(UpdateNote { id: page.id.clone(), title: Some("T".into()), ..Default::default() }, PERSON).unwrap(); // no change, no event
        let seen = seen.lock().unwrap();
        let summary: Vec<(String, i64)> = seen.iter().map(|change| (change.actor.clone(), change.seq)).collect();
        assert_eq!(summary, vec![(PERSON.into(), 1), ("agent:claude:s9".into(), 2), (PERSON.into(), 3)]);
        assert!(seen.iter().all(|change| change.page_id == page.id));
    }

    #[test]
    fn list_filters_by_text_tag_kind_state_and_parent() {
        let (_dir, notes) = service();
        let launch = notes.create(CreatePage { title: Some("Launch plan".into()), content: Some("ship 100% of it".into()), tags: vec!["launch".into()], ..Default::default() }, PERSON).unwrap();
        let task = notes.create(CreatePage { title: Some("Buy domain".into()), kind: Some("task".into()), parent_id: Some(launch.id.clone()), ..Default::default() }, PERSON).unwrap();
        let idea = notes.capture("an idea about sync_engines", PERSON).unwrap();
        let archived = notes.capture("old stuff", PERSON).unwrap();
        notes.update(UpdateNote { id: archived.id.clone(), state: Some("archived".into()), ..Default::default() }, PERSON).unwrap();
        let ids = |filter: ListFilter| -> Vec<String> { notes.list(&filter).unwrap().into_iter().map(|note| note.id).collect() };
        assert_eq!(ids(ListFilter { query: Some("launch PLAN".into()), ..Default::default() }), vec![launch.id.clone()]);
        assert_eq!(ids(ListFilter { query: Some("100%".into()), ..Default::default() }), vec![launch.id.clone()]);
        assert_eq!(ids(ListFilter { query: Some("sync_e".into()), ..Default::default() }), vec![idea.id.clone()]);
        assert!(ids(ListFilter { query: Some("sync%e".into()), ..Default::default() }).is_empty());
        assert_eq!(ids(ListFilter { tags: vec!["#Launch".into()], ..Default::default() }), vec![launch.id.clone()]);
        assert_eq!(ids(ListFilter { kind: Some("task".into()), ..Default::default() }), vec![task.id.clone()]);
        assert_eq!(ids(ListFilter { parent_id: Some(launch.id.clone()), ..Default::default() }), vec![task.id.clone()]);
        assert_eq!(ids(ListFilter { state: Some("inbox".into()), ..Default::default() }), vec![idea.id.clone()]);
        assert!(!ids(ListFilter::default()).contains(&archived.id));
        assert!(ids(ListFilter { include_archived: true, ..Default::default() }).contains(&archived.id));
        assert_eq!(ids(ListFilter { state: Some("archived".into()), ..Default::default() }), vec![archived.id]);
    }

    #[test]
    fn rebuild_index_from_updates_restores_everything() {
        let (_dir, notes) = service();
        let page = notes.create(CreatePage { title: Some("Launch".into()), content: Some("plan it".into()), tags: vec!["launch".into()], workspace_id: Some("ws-known".into()), ..Default::default() }, PERSON).unwrap();
        notes.create(CreatePage { title: Some("Buy domain".into()), kind: Some("task".into()), parent_id: Some(page.id.clone()), source_excerpt: Some("plan it".into()), ..Default::default() }, "model:tasks:sonnet").unwrap();
        notes.update(UpdateNote { id: page.id.clone(), add_session_id: Some("s1".into()), state: Some("doing".into()), ..Default::default() }, PERSON).unwrap();
        let gone = notes.capture("delete me", PERSON).unwrap();
        notes.delete(&gone.id, PERSON).unwrap();
        let before = notes.list(&ListFilter { include_archived: true, ..Default::default() }).unwrap();
        let page_before = notes.read(&page.id).unwrap();

        notes.with_db(|db| Ok(db.execute("DELETE FROM pages", [])?)).unwrap();
        assert!(notes.list(&ListFilter::default()).unwrap().is_empty());
        assert_eq!(notes.rebuild_index().unwrap(), 3);

        let mut after = notes.list(&ListFilter { include_archived: true, ..Default::default() }).unwrap();
        let mut expected = before.clone();
        let key = |note: &Note| note.id.clone();
        after.sort_by_key(key);
        expected.sort_by_key(key);
        let strip = |notes: Vec<Note>| notes.into_iter().map(|note| Note { updated_at: String::new(), ..note }).collect::<Vec<_>>();
        assert_eq!(strip(after), strip(expected));
        let page_after = notes.read(&page.id).unwrap();
        assert_eq!(page_after.markdown, page_before.markdown);
        assert_eq!(page_after.tasks.len(), 1);
        assert_eq!(page_after.note.session_ids, vec!["s1"]);
        assert!(notes.read(&gone.id).is_err());
    }
}
