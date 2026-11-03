//! Pages mirrored to Markdown files, both ways.
//!
//! After every change the service hands the page here; it is written to
//! `<root>/<slug>-<shortid>.md` with a temp file and a rename, so a reader
//! never sees half a file. The file's name is chosen once and kept, so an
//! editor holding it open is not pulled out from under.
//!
//! An edit made in another editor is found by [`import_changes`] (every few
//! seconds, and before a list) because the file's hash no longer matches the
//! hash Bridge wrote. It is applied as a save typed against the version the
//! file was written from, so it merges with anything that changed since. Until
//! it is imported the projector refuses to overwrite that file.

use std::path::{Path, PathBuf};

use rusqlite::params;

use super::markdown::{self, Frontmatter};
use super::service::{body_hash, EditPage, NotesService, UpdateNote, FILE_IMPORT, KINDS, STATES};
use crate::BridgeError;

/// What the service hands the projector after a committed change.
pub struct MirrorInput<'a> {
    pub note: &'a bridge_protocol::messages::Note,
    pub body: &'a str,
    pub deleted: bool,
    /// The file and hash Bridge last wrote for this page, if any.
    pub previous_path: Option<&'a str>,
    pub previous_hash: Option<&'a str>,
    /// A database page's `{scope, schema, views}` JSON.
    pub database: Option<&'a str>,
    /// A row's values, by property name, in schema order.
    pub properties: &'a [(String, String)],
    /// Every property name the row's schema owns.
    pub owned: &'a [String],
    /// A renamed or deleted property's old name, and, when the property is
    /// still live, the value the document now writes for it.
    pub retired: &'a [(String, Option<String>)],
    /// Cleared cells whose stale line has not left the file yet: the property
    /// name and the rendered value the line must match to be dropped.
    pub drop: &'a [(String, Option<String>)],
}

/// What the projector did: the file and the hash of what it wrote.
pub struct Mirrored {
    pub path: Option<String>,
    pub hash: Option<String>,
}

fn slug(text: &str) -> String {
    let mut out = String::new();
    for character in text.chars().flat_map(char::to_lowercase) {
        if character.is_alphanumeric() {
            out.push(character);
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
        if out.chars().count() >= 48 {
            break;
        }
    }
    let out = out.trim_end_matches('-').to_owned();
    if out.is_empty() {
        "note".into()
    } else {
        out
    }
}

/// `<slug>-<last 8 of the id>.md`. The id's tail is its random part; its head
/// is a timestamp shared by notes made in the same moment.
pub fn file_name(note: &bridge_protocol::messages::Note, body: &str) -> String {
    let basis = note.title.clone().unwrap_or_else(|| body.lines().find(|line| !line.trim().is_empty()).unwrap_or("note").to_owned());
    let tail: String = note.id.chars().filter(char::is_ascii_hexdigit).collect::<Vec<_>>().iter().rev().take(8).rev().collect();
    format!("{}-{}.md", slug(&basis), tail)
}

fn frontmatter(note: &bridge_protocol::messages::Note, unknown: Vec<String>, database: Option<&str>, properties: &[(String, String)]) -> Frontmatter {
    Frontmatter {
        id: Some(note.id.clone()),
        title: note.title.clone(),
        kind: Some(note.kind.clone()),
        tags: note.tags.clone(),
        project: note.workspace_id.clone(),
        state: Some(note.state.clone()),
        parent: note.parent_id.clone(),
        created: Some(note.created_at.clone()),
        updated: Some(note.updated_at.clone()),
        database: database.map(str::to_owned),
        properties: properties.to_vec(),
        unknown,
    }
}

/// The key of a `key: value` frontmatter line.
fn line_key(line: &str) -> &str {
    line.split_once(':').map(|(key, _)| key.trim()).unwrap_or("")
}

fn write_atomically(path: &Path, contents: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temp = path.with_extension(format!("md.tmp-{}", uuid::Uuid::new_v4().simple()));
    std::fs::write(&temp, contents)?;
    std::fs::rename(&temp, path)
}

fn file_hash(contents: &str) -> String {
    body_hash(contents)
}

/// Mirror one page. Returns `None` when nothing should be recorded (an outside
/// edit is waiting to be imported, or the write failed).
pub fn mirror(root: &Path, input: &MirrorInput<'_>) -> Option<Mirrored> {
    let previous = input.previous_path.map(PathBuf::from);
    if let (Some(path), Some(hash)) = (&previous, input.previous_hash) {
        if let Ok(existing) = std::fs::read_to_string(path) {
            if file_hash(&existing) != hash {
                // Someone edited the file outside Bridge; import it first.
                return None;
            }
        }
    }
    if input.deleted {
        if let Some(path) = previous.filter(|path| path.exists()) {
            let trash = root.join(".trash");
            let _ = std::fs::create_dir_all(&trash);
            let name = path.file_name().map(|name| name.to_owned()).unwrap_or_default();
            let _ = std::fs::rename(&path, trash.join(name));
        }
        return Some(Mirrored { path: None, hash: None });
    }
    let path = previous.unwrap_or_else(|| root.join(file_name(input.note, input.body)));
    let unknown = std::fs::read_to_string(&path)
        .map(|text| markdown::parse(&text).0.unknown)
        .unwrap_or_default()
        .into_iter()
        // A renamed column's old line is not a foreign key; it is the same
        // value under the name the schema no longer uses. A line the document
        // now owns is written from the document, so the file's copy goes.
        .filter(|line| {
            let key = line_key(line).to_lowercase();
            let raw = || line.split_once(':').map(|(_, value)| value.trim());
            if let Some((_, stale)) = input.drop.iter().find(|(name, _)| name.to_lowercase() == key) {
                // Drop the line only while it still says the value the person
                // cleared; anything else is a new edit waiting to import.
                return match (stale, raw()) {
                    (Some(stale), Some(raw)) => raw != stale.trim(),
                    _ => true,
                };
            }
            if input.owned.iter().any(|name| name.to_lowercase() == key) {
                // A live property owns its line unless the document has no
                // value: then the line is the only copy of a value that
                // predates the property (or could not be parsed for it).
                return !input.properties.iter().any(|(name, _)| name.to_lowercase() == key);
            }
            match input.retired.iter().find(|(name, _)| name.to_lowercase() == key) {
                // Drop a stale copy only when the document writes the same
                // value under the property's current name. Anything else is a
                // foreign key, and every foreign key stays.
                Some((_, Some(value))) => line.split_once(':').is_none_or(|(_, raw)| raw.trim() != value.trim()),
                Some((_, None)) => true,
                None => true,
            }
        })
        .collect();
    let contents = markdown::render(&frontmatter(input.note, unknown, input.database, input.properties), input.body);
    match write_atomically(&path, &contents) {
        Ok(()) => Some(Mirrored { path: Some(path.to_string_lossy().into_owned()), hash: Some(file_hash(&contents)) }),
        Err(error) => {
            eprintln!("[notes] could not write {}: {error}", path.display());
            None
        }
    }
}

fn apply_import(
    notes: &NotesService,
    id: &str,
    page: &bridge_protocol::messages::NotePage,
    front: Frontmatter,
    body: String,
    projected_seq: i64,
) -> Result<bool, BridgeError> {
    let mut touched = false;
    if body != page.markdown {
        let result = notes.edit(
            EditPage { id: id.to_owned(), base_seq: Some(projected_seq), text: Some(body), allow_deleting_children: true, ..Default::default() },
            FILE_IMPORT,
        )?;
        touched |= result.warnings.is_empty();
    }
    // The file was written from the page at `projected_seq`; only what the
    // file changed relative to that version is an edit. Comparing with the
    // latest page would treat stale frontmatter as new and undo concurrent
    // property changes.
    let base = notes.with_db(|db| Ok(super::doc::load_page(db, id, Some(projected_seq))?.0))?;
    let base_props = base.props();
    let mut update = UpdateNote { id: id.to_owned(), ..Default::default() };
    let mut wants = false;
    if front.title != base_props.title {
        update.title = Some(front.title.clone().unwrap_or_default());
        wants = true;
    }
    if let Some(kind) = front.kind.filter(|kind| KINDS.contains(&kind.as_str()) && Some(kind) != base_props.kind.as_ref()) {
        update.kind = Some(kind);
        wants = true;
    }
    if let Some(state) = front.state.filter(|state| STATES.contains(&state.as_str()) && Some(state) != base_props.state.as_ref()) {
        update.state = Some(state);
        wants = true;
    }
    if front.tags != base_props.tags {
        update.tags = Some(front.tags);
        wants = true;
    }
    if wants {
        notes.update(update, FILE_IMPORT)?;
        touched = true;
    }
    // The project link, through the same validated update. An id that names
    // no project is left out rather than failing the rest of the import.
    if front.project != base_props.workspace_id {
        let change = match &front.project {
            Some(workspace) => UpdateNote { id: id.to_owned(), workspace_id: Some(workspace.clone()), ..Default::default() },
            None => UpdateNote { id: id.to_owned(), clear_workspace: true, ..Default::default() },
        };
        match notes.update(change, FILE_IMPORT) {
            Ok(_) => touched = true,
            Err(error) => eprintln!("[notes] kept the project of {id}: {error}"),
        }
    }
    // A row's values are frontmatter keys named after its database's
    // properties. A key that names no property, or a value that does not
    // parse, stays in `Frontmatter.unknown` and is written back verbatim.
    if let Some(parent) = page.note.parent_id.as_deref() {
        let values = notes.with_db(|db| {
            let Some(database) = super::database::read(db, parent)? else { return Ok(Vec::new()) };
            // Compared against the file's own base, like every other property,
            // so a value the person changed in Bridge is not reverted here.
            let current = base.values();
            let stale = super::doc::load_page(db, id, None)?.0.cleared();
            let ordered = database.ordered();
            let mut patch: Vec<(String, Option<serde_json::Value>)> = Vec::new();
            for line in &front.unknown {
                let Some((key, raw)) = line.split_once(':') else { continue };
                let key = key.trim();
                let Some((prop, def)) = ordered.iter().find(|(_, def)| def.name.to_lowercase() == key.to_lowercase()) else { continue };
                let parsed = super::database::parse_value(db, &page.note.space_id, def, raw).filter(|value| super::database::validate_value(def, value).is_ok());
                let Some(value) = parsed else {
                    // The line stays in the file; nothing is written.
                    eprintln!("[notes] kept {key} of {id} verbatim: it is not a valid {}", def.kind.as_str());
                    continue;
                };
                // A line that still says the value a clear removed is the stale
                // copy; the projection drops it, and importing it would undo the
                // clear.
                if stale.get(*prop) == Some(&value) {
                    continue;
                }
                if current.get(*prop) != Some(&value) {
                    patch.push(((*prop).clone(), Some(value)));
                }
            }
            // A property the page had a value for and the file no longer names
            // is a clear. A line that predates the property has no base value,
            // so it stays foreign instead of being deleted.
            let present: std::collections::BTreeSet<String> =
                front.unknown.iter().filter_map(|line| line.split_once(':').map(|(key, _)| key.trim().to_lowercase())).collect();
            for (prop, def) in &ordered {
                if current.contains_key(*prop) && !present.contains(&def.name.to_lowercase()) {
                    patch.push(((*prop).clone(), None));
                }
            }
            Ok(patch)
        })?;
        if !values.is_empty() {
            notes.set_values(id, &values, FILE_IMPORT)?;
            touched = true;
        }
    }
    Ok(touched)
}

/// Mirror every page whose newest committed version was never written to
/// its file. Returns how many pages it mirrored.
///
/// A mirror write can fail (a blocked folder, a full disk) and a crash can
/// land between a commit and its projection; either leaves `projected_seq`
/// behind `seq`. Runs when the store opens and on every maintenance pass.
pub fn reconcile(notes: &NotesService) -> Result<usize, BridgeError> {
    let behind: Vec<String> = notes.with_db(|db| {
        let mut statement = db.prepare(
            "SELECT id FROM pages
             WHERE (projected_seq IS NULL OR projected_seq < seq)
               AND (deleted_at IS NULL OR path IS NOT NULL)",
        )?;
        let ids = statement.query_map([], |row| row.get(0))?.collect::<Result<_, _>>()?;
        Ok(ids)
    })?;
    let mut mirrored = 0;
    for id in behind {
        let before = notes.with_db(|db| Ok(db.query_row("SELECT projected_seq FROM pages WHERE id=?1", params![id], |row| row.get::<_, Option<i64>>(0))?))?;
        notes.project(&id)?;
        let after = notes.with_db(|db| Ok(db.query_row("SELECT projected_seq FROM pages WHERE id=?1", params![id], |row| row.get::<_, Option<i64>>(0))?))?;
        mirrored += usize::from(after != before);
    }
    Ok(mirrored)
}

/// Every mirrored file under `root`, by the page id in its frontmatter.
pub fn find_files(root: &Path) -> std::collections::HashMap<String, (PathBuf, String)> {
    let mut found = std::collections::HashMap::new();
    let Ok(entries) = std::fs::read_dir(root) else { return found };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("md") {
            continue;
        }
        let Ok(contents) = std::fs::read_to_string(&path) else { continue };
        if let Some(id) = markdown::parse(&contents).0.id {
            found.insert(id, (path, file_hash(&contents)));
        }
    }
    found
}

/// Apply edits made to mirrored files outside Bridge. Returns how many pages
/// changed.
pub fn import_changes(notes: &NotesService) -> Result<usize, BridgeError> {
    // One importer at a time, and the candidates (hash, base) are read after
    // taking it: two importers holding the same old base would each merge the
    // same outside edit as a fresh change.
    let _import = notes.import_guard();
    let candidates: Vec<(String, String, String, i64)> = notes.with_db(|db| {
        let mut statement = db.prepare(
            "SELECT id, path, content_hash, projected_seq FROM pages
             WHERE deleted_at IS NULL AND path IS NOT NULL AND content_hash IS NOT NULL",
        )?;
        let rows = statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get::<_, Option<i64>>(3)?.unwrap_or(0))))?
            .collect::<Result<_, _>>()?;
        Ok(rows)
    })?;
    let mut changed = 0;
    for (id, path, hash, projected_seq) in candidates {
        let Ok(contents) = std::fs::read_to_string(&path) else { continue };
        if file_hash(&contents) == hash {
            continue;
        }
        let (front, body) = markdown::parse(&contents);
        let Ok(page) = notes.read(&id) else { continue };
        // Bridge has now seen this file; the merged result may overwrite it.
        let set_hash = |value: &str| notes.with_db(|db| Ok(db.execute("UPDATE pages SET content_hash=?2 WHERE id=?1", params![id, value])?));
        set_hash(&file_hash(&contents))?;
        let file_body = body.clone();
        let applied = apply_import(notes, &id, &page, front, body, projected_seq);
        let touched = match applied {
            Ok(touched) => touched,
            Err(error) => {
                set_hash(&hash)?;
                return Err(error);
            }
        };
        if !touched && notes.read(&id).is_ok_and(|now| now.markdown == file_body) {
            // Nothing to write: the file already says what the page says, so
            // it now stands for the page's current version. Without this the
            // next outside edit merges against an older base and re-inserts
            // text the page already has.
            notes.with_db(|db| Ok(db.execute("UPDATE pages SET projected_seq=seq WHERE id=?1", params![id])?))?;
        }
        changed += usize::from(touched);
    }
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notes::service::tests::service;
    use crate::notes::service::{CreatePage, PERSON};
    use bridge_protocol::messages::NoteAppend;

    #[test]
    fn writes_atomically_with_frontmatter() {
        let (dir, notes) = service();
        let note = notes.create(CreatePage { title: Some("Launch Plan: v1!".into()), content: Some("ship it, teh typo stays".into()), tags: vec!["launch".into()], ..Default::default() }, PERSON).unwrap();
        let path = PathBuf::from(note.path.clone().expect("mirrored"));
        assert!(path.starts_with(dir.path().join("notes")));
        assert!(path.file_name().unwrap().to_string_lossy().starts_with("launch-plan-v1-"));
        let text = std::fs::read_to_string(&path).unwrap();
        let (front, body) = markdown::parse(&text);
        assert_eq!(body, "ship it, teh typo stays");
        assert_eq!(front.id.as_deref(), Some(note.id.as_str()));
        assert_eq!(front.title.as_deref(), Some("Launch Plan: v1!"));
        assert_eq!(front.tags, vec!["launch"]);
        assert_eq!(front.project, None);
        let leftovers: Vec<_> = std::fs::read_dir(path.parent().unwrap()).unwrap().flatten().filter(|entry| entry.file_name().to_string_lossy().contains(".tmp-")).collect();
        assert!(leftovers.is_empty());

        // A later change keeps the same file and its unknown keys.
        std::fs::write(&path, text.replace("---\nship", "cssclass: wide\n---\nship")).unwrap();
        crate::notes::projector::import_changes(&notes).unwrap();
        notes.update(UpdateNote { id: note.id.clone(), title: Some("Renamed".into()), ..Default::default() }, PERSON).unwrap();
        let after = notes.read(&note.id).unwrap().note;
        assert_eq!(after.path.as_deref(), Some(path.to_string_lossy().as_ref()));
        let rewritten = std::fs::read_to_string(&path).unwrap();
        assert!(rewritten.contains("title: \"Renamed\""));
        assert!(rewritten.contains("cssclass: wide\n---\n"));

        notes.delete(&note.id, PERSON).unwrap();
        assert!(!path.exists());
        assert!(dir.path().join("notes/.trash").join(path.file_name().unwrap()).exists());
    }

    #[test]
    fn importer_applies_outside_edits_as_ops() {
        let (_dir, notes) = service();
        let note = notes.create(CreatePage { title: Some("Ideas".into()), content: Some("line one\n".into()), ..Default::default() }, PERSON).unwrap();
        let path = PathBuf::from(note.path.clone().unwrap());
        let original = std::fs::read_to_string(&path).unwrap();
        // Edited in another editor: body and a tag.
        std::fs::write(&path, original.replace("line one\n", "line one\nline two from vim\n").replace("tags: []", "tags: [\"vim\"]")).unwrap();

        assert_eq!(import_changes(&notes).unwrap(), 1);
        let page = notes.read(&note.id).unwrap();
        assert_eq!(page.markdown, "line one\nline two from vim\n");
        assert_eq!(page.note.tags, vec!["vim"]);
        assert_eq!(page.note.last_actor, FILE_IMPORT);
        assert_eq!(import_changes(&notes).unwrap(), 0, "an imported file is not imported again");
    }

    #[test]
    fn concurrent_imports_apply_an_outside_edit_once() {
        for _ in 0..5 {
            let (_dir, notes) = service();
            let notes = std::sync::Arc::new(notes);
            let note = notes.create(CreatePage { content: Some("start\n".into()), ..Default::default() }, PERSON).unwrap();
            let path = PathBuf::from(note.path.clone().unwrap());
            let original = std::fs::read_to_string(&path).unwrap();
            std::fs::write(&path, original.replace("start\n", "start\nfrom editor\n")).unwrap();
            let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
            let workers: Vec<_> = (0..8)
                .map(|_| {
                    let (notes, barrier) = (notes.clone(), barrier.clone());
                    std::thread::spawn(move || {
                        barrier.wait();
                        import_changes(&notes).unwrap();
                    })
                })
                .collect();
            for worker in workers {
                worker.join().unwrap();
            }
            assert_eq!(notes.read(&note.id).unwrap().markdown, "start\nfrom editor\n");
        }
    }

    #[test]
    fn a_body_only_file_edit_keeps_concurrent_property_changes() {
        let (_dir, notes) = service();
        let note = notes.create(CreatePage { title: Some("Original".into()), content: Some("body\n".into()), tags: vec!["a".into()], ..Default::default() }, PERSON).unwrap();
        let path = PathBuf::from(note.path.clone().unwrap());
        let original = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, original.replace("body\n", "body edited in vim\n")).unwrap();
        // Meanwhile an agent changes every property; the file still has the old ones.
        notes.update(UpdateNote { id: note.id.clone(), title: Some("Agent title".into()), state: Some("done".into()), kind: Some("idea".into()), tags: Some(vec!["b".into()]), ..Default::default() }, "agent:claude:s1").unwrap();
        import_changes(&notes).unwrap();
        let page = notes.read(&note.id).unwrap();
        assert_eq!(page.markdown, "body edited in vim\n");
        assert_eq!((page.note.title.as_deref(), page.note.state.as_str(), page.note.kind.as_str()), (Some("Agent title"), "done", "idea"));
        assert_eq!(page.note.tags, vec!["b"]);
        // A property the file really changes is still imported.
        let current = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, current.replace("title: \"Agent title\"", "title: \"Renamed in vim\"")).unwrap();
        import_changes(&notes).unwrap();
        let page = notes.read(&note.id).unwrap();
        assert_eq!((page.note.title.as_deref(), page.note.state.as_str()), (Some("Renamed in vim"), "done"));
    }

    #[test]
    fn project_frontmatter_is_imported() {
        let (_dir, notes) = service();
        let note = notes.capture("assign me\n", PERSON).unwrap();
        let path = PathBuf::from(note.path.clone().unwrap());
        let edit_file = |from: &str, to: &str| {
            let text = std::fs::read_to_string(&path).unwrap();
            assert!(text.contains(from), "{text}");
            std::fs::write(&path, text.replace(from, to)).unwrap();
        };
        edit_file("project: null", "project: \"ws-known\"");
        import_changes(&notes).unwrap();
        assert_eq!(notes.read(&note.id).unwrap().note.workspace_id.as_deref(), Some("ws-known"));
        edit_file("project: \"ws-known\"", "project: null");
        import_changes(&notes).unwrap();
        assert_eq!(notes.read(&note.id).unwrap().note.workspace_id, None);
        // An unknown project is not assigned, and the rest of the edit still lands.
        edit_file("project: null", "project: \"ws-missing\"");
        edit_file("assign me\n", "assign me, edited\n");
        import_changes(&notes).unwrap();
        let page = notes.read(&note.id).unwrap();
        assert_eq!((page.note.workspace_id, page.markdown.as_str()), (None, "assign me, edited\n"));
    }

    #[test]
    fn accepting_a_matching_body_advances_the_import_base() {
        let (_dir, notes) = service();
        let note = notes.create(CreatePage { content: Some("top\n".into()), ..Default::default() }, PERSON).unwrap();
        let path = PathBuf::from(note.path.clone().unwrap());
        let original = std::fs::read_to_string(&path).unwrap();
        // The person types the same line an agent is about to append.
        std::fs::write(&path, original.replace("top\n", "top\nagent line\n")).unwrap();
        notes.edit(EditPage { id: note.id.clone(), append: Some(NoteAppend { content: "agent line\n".into(), position: "end".into() }), ..Default::default() }, "agent:claude:s1").unwrap();
        import_changes(&notes).unwrap();
        assert_eq!(notes.read(&note.id).unwrap().markdown, "top\nagent line\n");
        // A later outside edit must merge against what the accepted file represents.
        let accepted = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, accepted.replace("top\nagent line\n", "top\nagent line\nexternal line\n")).unwrap();
        import_changes(&notes).unwrap();
        assert_eq!(notes.read(&note.id).unwrap().markdown, "top\nagent line\nexternal line\n");
    }

    #[test]
    fn a_failed_projection_is_retried_by_maintenance() {
        let dir = tempfile::tempdir().unwrap();
        let blocked = dir.path().join("notes");
        let check: crate::notes::service::ProjectCheck = std::sync::Arc::new(|_: &str| false);
        // A file where the notes folder should be: every mirror write fails.
        std::fs::write(&blocked, "not a folder").unwrap();
        let notes = NotesService::open(dir.path(), check.clone()).unwrap();
        let first = notes.capture("written while blocked\n", PERSON).unwrap();
        assert!(first.path.is_none(), "the capture itself succeeds");
        std::fs::remove_file(&blocked).unwrap();
        drop(notes);
        // Reopening repairs it.
        let notes = NotesService::open(dir.path(), check).unwrap();
        let path = notes.read(&first.id).unwrap().note.path.expect("mirrored on open");
        assert_eq!(markdown::parse(&std::fs::read_to_string(&path).unwrap()).1, "written while blocked\n");
        // So does a maintenance pass, without a restart.
        std::fs::remove_dir_all(&blocked).unwrap();
        std::fs::write(&blocked, "not a folder").unwrap();
        let second = notes.capture("also blocked\n", PERSON).unwrap();
        assert!(second.path.is_none());
        std::fs::remove_file(&blocked).unwrap();
        reconcile(&notes).unwrap();
        assert!(notes.read(&second.id).unwrap().note.path.is_some());
    }

    #[test]
    fn importer_skips_its_own_writes() {
        let (_dir, notes) = service();
        let note = notes.capture("first", PERSON).unwrap();
        for word in ["second", "third"] {
            notes.edit(EditPage { id: note.id.clone(), append: Some(NoteAppend { content: word.into(), position: "end".into() }), ..Default::default() }, "agent:claude:s1").unwrap();
        }
        assert_eq!(import_changes(&notes).unwrap(), 0);
        assert_eq!(notes.read(&note.id).unwrap().note.last_actor, "agent:claude:s1");
    }

    #[test]
    fn a_row_file_writes_every_value_by_name_and_imports_it_back() {
        use crate::notes::service::{NewOption, NewProperty, SchemaOp};
        let (_dir, notes) = service();
        let library = notes.create(CreatePage { title: Some("Library".into()), kind: Some("database".into()), ..Default::default() }, PERSON).unwrap();
        let chapter = notes.create(CreatePage { title: Some("Designing Data-Intensive Applications".into()), parent_id: Some(library.id.clone()), ..Default::default() }, PERSON).unwrap();
        let list = notes.create(CreatePage { title: Some("Reading list".into()), kind: Some("database".into()), ..Default::default() }, PERSON).unwrap();
        let added = notes
            .update_schema(
                &list.id,
                &[
                    SchemaOp::Add(NewProperty { name: "Rating".into(), kind: "number".into(), ..Default::default() }),
                    SchemaOp::Add(NewProperty {
                        name: "Status".into(),
                        kind: "status".into(),
                        options: vec![NewOption { name: "Reading".into(), ..Default::default() }, NewOption { name: "Done".into(), group: Some("done".into()), ..Default::default() }],
                        ..Default::default()
                    }),
                    SchemaOp::Add(NewProperty { name: "Due".into(), kind: "date".into(), ..Default::default() }),
                    SchemaOp::Add(NewProperty { name: "Finished".into(), kind: "checkbox".into(), ..Default::default() }),
                    SchemaOp::Add(NewProperty { name: "Link".into(), kind: "url".into(), ..Default::default() }),
                    SchemaOp::Add(NewProperty { name: "Related".into(), kind: "relation".into(), relation: Some(library.id.clone()), ..Default::default() }),
                    SchemaOp::Add(NewProperty { name: "Note".into(), kind: "text".into(), ..Default::default() }),
                    SchemaOp::Add(NewProperty { name: "Labels".into(), kind: "multiSelect".into(), options: vec![NewOption { name: "Fiction".into(), ..Default::default() }, NewOption { name: "Tech".into(), ..Default::default() }], ..Default::default() }),
                ],
                PERSON,
            )
            .unwrap();
        let prop = |name: &str| added.added.iter().find(|(_, added)| added == name).unwrap().0.clone();
        let schema = notes.database(&list.id).unwrap();
        let reading = schema.schema[&prop("Status")].options[0].id.clone();
        let (fiction, tech) = (schema.schema[&prop("Labels")].options[0].id.clone(), schema.schema[&prop("Labels")].options[1].id.clone());
        let row = notes.create(CreatePage { title: Some("DDIA".into()), parent_id: Some(list.id.clone()), ..Default::default() }, PERSON).unwrap();
        let values = vec![
            (prop("Rating"), Some(serde_json::json!(5))),
            (prop("Status"), Some(serde_json::json!(reading))),
            (prop("Due"), Some(serde_json::json!({"start": "2026-11-10"}))),
            (prop("Finished"), Some(serde_json::json!(true))),
            (prop("Link"), Some(serde_json::json!("https://bridge.dev"))),
            (prop("Related"), Some(serde_json::json!([chapter.id.clone()]))),
            (prop("Note"), Some(serde_json::json!("the best book"))),
            (prop("Labels"), Some(serde_json::json!([fiction, tech]))),
        ];
        notes.set_values(&row.id, &values, PERSON).unwrap();
        let path = PathBuf::from(notes.read(&row.id).unwrap().note.path.unwrap());
        let file = std::fs::read_to_string(&path).unwrap();
        // One line per property, by name, in schema order, after the known keys.
        let order: Vec<usize> = ["Rating: 5", "Status: \"Reading\"", "Due: \"2026-11-10\"", "Finished: true", "Link: \"https://bridge.dev\"", "Related: [\"[[Designing Data-Intensive Applications]]\"]", "Note: \"the best book\"", "Labels: [\"Fiction\",\"Tech\"]"]
            .iter()
            .map(|line| file.find(line).unwrap_or_else(|| panic!("missing {line} in {file}")))
            .collect();
        assert!(order.windows(2).all(|pair| pair[0] < pair[1]), "{file}");
        assert!(file.find("updated:").unwrap() < order[0]);

        // An edit in the file lands in the same doc state as set_values.
        let before = notes.read(&row.id).unwrap().seq;
        std::fs::write(&path, file.replace("Rating: 5", "Rating: 3")).unwrap();
        assert_eq!(import_changes(&notes).unwrap(), 1);
        let page = notes.read(&row.id).unwrap();
        assert_eq!(page.seq, before + 1);
        let stored = notes.with_db(|db| Ok(crate::notes::doc::load_page(db, &row.id, None)?.0.values())).unwrap();
        assert_eq!(stored[&prop("Rating")], serde_json::json!(3));
        assert_eq!(page.note.last_actor, FILE_IMPORT);
        assert!(std::fs::read_to_string(&path).unwrap().contains("Rating: 3"));

        // A value that does not parse is left in the file and changes nothing.
        let accepted = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, accepted.replace("Rating: 3", "Rating: three")).unwrap();
        import_changes(&notes).unwrap();
        assert_eq!(notes.read(&row.id).unwrap().seq, page.seq);
        assert!(std::fs::read_to_string(&path).unwrap().contains("Rating: three"));

        // A rename rewrites the key in the file, off the request path, and
        // writes no row doc.
        let seq = notes.read(&row.id).unwrap().seq;
        notes.update_schema(&list.id, &[SchemaOp::Rename { prop: prop("Rating"), name: "Score".into() }], PERSON).unwrap();
        crate::notes::projector::reconcile(&notes).unwrap();
        let renamed = std::fs::read_to_string(&path).unwrap();
        assert!(renamed.contains("Score: 3"), "{renamed}");
        assert!(!renamed.contains("Rating: 3"), "the value moved to the new name: {renamed}");
        assert!(renamed.contains("Rating: three"), "a refused edit stays as a foreign key: {renamed}");
        assert_eq!(notes.read(&row.id).unwrap().seq, seq, "a rename touches no row doc");

        // Clearing a cell removes its line; a foreign key survives.
        std::fs::write(&path, renamed.replace("\n---\n", "\naliases: [\"ddia\"]\n---\n")).unwrap();
        import_changes(&notes).unwrap();
        notes.set_values(&row.id, &[(prop("Rating"), None)], PERSON).unwrap();
        let cleared = std::fs::read_to_string(&path).unwrap();
        assert!(!cleared.contains("Score:"), "{cleared}");
        assert!(cleared.contains("aliases: [\"ddia\"]"), "{cleared}");

        // The database page's own file carries its schema and views.
        let database = std::fs::read_to_string(notes.read(&list.id).unwrap().note.path.unwrap()).unwrap();
        assert!(database.contains("kind: \"database\""));
        assert!(database.contains("database: {\"scope\":\"children\""), "{database}");
        assert!(database.contains("\"name\":\"Rating\"") || database.contains("\"name\":\"Score\""), "{database}");
    }

    #[test]
    fn a_foreign_database_key_on_a_plain_page_survives() {
        let (_dir, notes) = service();
        let page = notes.create(CreatePage { title: Some("Plain".into()), content: Some("body\n".into()), ..Default::default() }, PERSON).unwrap();
        let path = PathBuf::from(page.path.clone().unwrap());
        let file = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, file.replace("\n---\n", "\naliases: [\"a\"]\ndatabase: \"obsidian-thing\"\ncssclass: wide\n---\n")).unwrap();
        import_changes(&notes).unwrap();
        notes.update(UpdateNote { id: page.id.clone(), title: Some("Renamed".into()), ..Default::default() }, PERSON).unwrap();
        let after = std::fs::read_to_string(&path).unwrap();
        assert!(after.contains("database: \"obsidian-thing\""), "a foreign key is not Bridge's record: {after}");
        assert!(after.contains("title: \"Renamed\""), "{after}");
        let (aliases, database, cssclass) = (after.find("aliases:").unwrap(), after.find("database:").unwrap(), after.find("cssclass:").unwrap());
        assert!(aliases < database && database < cssclass, "a foreign key keeps its place: {after}");
    }

    #[test]
    fn a_item_deleted_from_the_file_clears_the_cell() {
        let (_dir, notes) = service();
        let list = notes.create(CreatePage { title: Some("Reading list".into()), kind: Some("database".into()), ..Default::default() }, PERSON).unwrap();
        let added = notes.update_schema(&list.id, &[crate::notes::service::SchemaOp::Add(crate::notes::service::NewProperty { name: "Rating".into(), kind: "number".into(), ..Default::default() })], PERSON).unwrap();
        let rating = added.added[0].0.clone();
        let row = notes.create(CreatePage { title: Some("DDIA".into()), parent_id: Some(list.id.clone()), ..Default::default() }, PERSON).unwrap();
        notes.set_values(&row.id, &[(rating.clone(), Some(serde_json::json!(5)))], PERSON).unwrap();
        let path = PathBuf::from(notes.read(&row.id).unwrap().note.path.unwrap());
        let file = std::fs::read_to_string(&path).unwrap();
        assert!(file.contains("Rating: 5"), "{file}");
        std::fs::write(&path, file.replace("Rating: 5\n", "")).unwrap();
        import_changes(&notes).unwrap();
        let stored = notes.with_db(|db| Ok(crate::notes::doc::load_page(db, &row.id, None)?.0.values())).unwrap();
        assert!(!stored.contains_key(&rating), "deleting the line clears the cell");
        assert!(!std::fs::read_to_string(&path).unwrap().contains("Rating:"), "and the doc does not write it back");
    }

    #[test]
    fn a_foreign_key_the_new_property_cannot_parse_stays_in_the_file() {
        let (_dir, notes) = service();
        let list = notes.create(CreatePage { title: Some("Reading list".into()), kind: Some("database".into()), ..Default::default() }, PERSON).unwrap();
        let row = notes.create(CreatePage { title: Some("DDIA".into()), parent_id: Some(list.id.clone()), ..Default::default() }, PERSON).unwrap();
        let path = PathBuf::from(notes.read(&row.id).unwrap().note.path.unwrap());
        let file = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, file.replace("\n---\n", "\nPriority: \"urgent\"\n---\n")).unwrap();
        import_changes(&notes).unwrap();
        let added = notes.update_schema(&list.id, &[crate::notes::service::SchemaOp::Add(crate::notes::service::NewProperty { name: "Priority".into(), kind: "number".into(), ..Default::default() })], PERSON).unwrap();
        let stored = notes.with_db(|db| Ok(crate::notes::doc::load_page(db, &row.id, None)?.0.values())).unwrap();
        assert!(!stored.contains_key(&added.added[0].0), "a number column takes no text");
        notes.update(crate::notes::service::UpdateNote { id: row.id.clone(), title: Some("Renamed".into()), ..Default::default() }, PERSON).unwrap();
        let after = std::fs::read_to_string(&path).unwrap();
        assert!(after.contains("Priority: \"urgent\""), "the person's only copy stays: {after}");
    }

    #[test]
    fn a_retired_name_does_not_delete_a_later_foreign_key() {
        let (_dir, notes) = service();
        let list = notes.create(CreatePage { title: Some("Reading list".into()), kind: Some("database".into()), ..Default::default() }, PERSON).unwrap();
        let added = notes.update_schema(&list.id, &[crate::notes::service::SchemaOp::Add(crate::notes::service::NewProperty { name: "Rating".into(), kind: "number".into(), ..Default::default() })], PERSON).unwrap();
        let rating = added.added[0].0.clone();
        let row = notes.create(CreatePage { title: Some("DDIA".into()), parent_id: Some(list.id.clone()), ..Default::default() }, PERSON).unwrap();
        notes.set_values(&row.id, &[(rating.clone(), Some(serde_json::json!(5)))], PERSON).unwrap();
        let path = PathBuf::from(notes.read(&row.id).unwrap().note.path.unwrap());
        notes.update_schema(&list.id, &[crate::notes::service::SchemaOp::Rename { prop: rating, name: "Score".into() }], PERSON).unwrap();
        crate::notes::projector::reconcile(&notes).unwrap();
        assert!(!std::fs::read_to_string(&path).unwrap().contains("Rating:"), "the stale copy goes");
        // Later, the person writes a key that happens to reuse the old name.
        let file = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, file.replace("\n---\n", "\nrating: \"from obsidian\"\n---\n")).unwrap();
        import_changes(&notes).unwrap();
        notes.update(crate::notes::service::UpdateNote { id: row.id.clone(), title: Some("Renamed".into()), ..Default::default() }, PERSON).unwrap();
        let after = std::fs::read_to_string(&path).unwrap();
        assert!(after.contains("rating: \"from obsidian\""), "a retired name is not a permanent delete list: {after}");
    }

    #[test]
    fn an_outside_edit_does_not_revert_a_value_set_in_bridge() {
        let (_dir, notes) = service();
        let list = notes.create(CreatePage { title: Some("Reading list".into()), kind: Some("database".into()), ..Default::default() }, PERSON).unwrap();
        let added = notes.update_schema(&list.id, &[crate::notes::service::SchemaOp::Add(crate::notes::service::NewProperty { name: "Rating".into(), kind: "number".into(), ..Default::default() })], PERSON).unwrap();
        let rating = added.added[0].0.clone();
        let row = notes.create(CreatePage { title: Some("DDIA".into()), parent_id: Some(list.id.clone()), ..Default::default() }, PERSON).unwrap();
        notes.set_values(&row.id, &[(rating.clone(), Some(serde_json::json!(3)))], PERSON).unwrap();
        let path = PathBuf::from(notes.read(&row.id).unwrap().note.path.unwrap());
        // Someone edits the body in another editor; the file is now dirty.
        let file = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, file.replace("\n---\n", "\n---\n\nedited in vim\n")).unwrap();
        // The person rates it 4 in Bridge while that edit waits to be imported.
        notes.set_values(&row.id, &[(rating.clone(), Some(serde_json::json!(4)))], PERSON).unwrap();
        import_changes(&notes).unwrap();
        let stored = notes.with_db(|db| Ok(crate::notes::doc::load_page(db, &row.id, None)?.0.values())).unwrap();
        assert_eq!(stored[&rating], serde_json::json!(4), "the import must not revert a value set in Bridge");
        assert!(notes.read(&row.id).unwrap().markdown.contains("edited in vim"));
        assert!(std::fs::read_to_string(&path).unwrap().contains("Rating: 4"));
    }

    #[test]
    fn an_outside_edit_merges_with_a_write_made_before_import() {        let (_dir, notes) = service();
        let note = notes.create(CreatePage { content: Some("top\n".into()), ..Default::default() }, PERSON).unwrap();
        let path = PathBuf::from(note.path.clone().unwrap());
        let original = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, original.replace("top\n", "top, edited in vim\n")).unwrap();
        // An agent appends before the importer has run: the file must not be clobbered.
        notes.edit(EditPage { id: note.id.clone(), append: Some(NoteAppend { content: "agent line\n".into(), position: "end".into() }), ..Default::default() }, "agent:claude:s1").unwrap();
        assert!(std::fs::read_to_string(&path).unwrap().contains("top, edited in vim"));
        import_changes(&notes).unwrap();
        assert_eq!(notes.read(&note.id).unwrap().markdown, "top, edited in vim\nagent line\n");
        assert!(std::fs::read_to_string(&path).unwrap().ends_with("top, edited in vim\nagent line\n"));
    }
}
