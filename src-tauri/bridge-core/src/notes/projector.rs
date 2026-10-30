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

fn frontmatter(note: &bridge_protocol::messages::Note, unknown: Vec<String>) -> Frontmatter {
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
        unknown,
    }
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
    let unknown = std::fs::read_to_string(&path).map(|text| markdown::parse(&text).0.unknown).unwrap_or_default();
    let contents = markdown::render(&frontmatter(input.note, unknown), input.body);
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
    let note = notes.read(id)?.note;
    let mut update = UpdateNote { id: id.to_owned(), ..Default::default() };
    let mut wants = false;
    if front.title != note.title {
        update.title = Some(front.title.clone().unwrap_or_default());
        wants = true;
    }
    if let Some(kind) = front.kind.filter(|kind| KINDS.contains(&kind.as_str()) && *kind != note.kind) {
        update.kind = Some(kind);
        wants = true;
    }
    if let Some(state) = front.state.filter(|state| STATES.contains(&state.as_str()) && *state != note.state) {
        update.state = Some(state);
        wants = true;
    }
    if front.tags != note.tags {
        update.tags = Some(front.tags);
        wants = true;
    }
    if wants {
        notes.update(update, FILE_IMPORT)?;
        touched = true;
    }
    Ok(touched)
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
        let applied = apply_import(notes, &id, &page, front, body, projected_seq);
        let touched = match applied {
            Ok(touched) => touched,
            Err(error) => {
                set_hash(&hash)?;
                return Err(error);
            }
        };
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
    fn an_outside_edit_merges_with_a_write_made_before_import() {
        let (_dir, notes) = service();
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
