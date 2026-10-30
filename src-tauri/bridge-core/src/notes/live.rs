//! The production side of notes: the maintenance loop and the live backend.
//!
//! Two threads per host: one drains `model_jobs` (Structure and Turn into
//! tasks, queued only when someone asked); the other retries Markdown mirrors
//! and imports edits made to mirrored files outside Bridge. Model turns run through `title_jobs_live::one_hidden_turn`:
//! tool-less, no `sessions` row, spend recorded on the job.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
#[cfg(test)]
use std::time::Instant;

use rusqlite::params;
use uuid::Uuid;

use super::jobs::{self, ModelBackend, ProjectChoice};
use super::projector;
use super::service::{NotesService, ProjectCheck};
use crate::runtime::BridgeCore;
use crate::title_jobs::Generation;

const POLL: Duration = Duration::from_secs(2);
const IMPORT_EVERY: Duration = Duration::from_secs(5);
/// Every note pass shares one scratch directory, as title runs do.
const SCRATCH: &str = "note-runs";

/// Whether a workspace (project) id exists, read from `bridge.db` on its own
/// connection so a notes write never waits on the core database lock.
pub fn project_check(bridge_db: PathBuf) -> ProjectCheck {
    Arc::new(move |workspace: &str| {
        rusqlite::Connection::open(&bridge_db)
            .and_then(|db| db.query_row("SELECT EXISTS(SELECT 1 FROM workspaces WHERE id=?1)", params![workspace], |row| row.get(0)))
            .unwrap_or(false)
    })
}

pub struct LiveBackend {
    core: Arc<BridgeCore>,
    scratch: PathBuf,
}

impl LiveBackend {
    pub fn new(core: Arc<BridgeCore>) -> Self {
        let scratch = core.chat_scratch_dir(SCRATCH);
        Self { core, scratch }
    }
}

impl ModelBackend for LiveBackend {
    fn can_generate(&self, harness: &str) -> bool {
        self.core.adapter_registry.descriptor(harness).is_some_and(|descriptor| descriptor.available)
    }

    fn default_choice(&self) -> Option<(String, Option<String>)> {
        let db = self.core.db.lock().unwrap();
        let view = crate::chat_defaults::view(&db, None).ok()?;
        view.application.defaults.model.map(|choice| (choice.harness, choice.model))
    }

    fn projects(&self) -> Vec<ProjectChoice> {
        let db = self.core.db.lock().unwrap();
        let Ok(mut statement) = db.prepare(
            "SELECT w.id, p.name, w.title FROM workspaces w JOIN projects p ON p.id=w.project_id ORDER BY w.created_at DESC LIMIT 30",
        ) else {
            return Vec::new();
        };
        let rows = statement.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?)));
        rows.map(|rows| {
            rows.flatten()
                .map(|(id, project, title)| ProjectChoice { id, label: if project == title { project } else { format!("{project} / {title}") } })
                .collect()
        })
        .unwrap_or_default()
    }

    fn generate(&self, harness: &str, model: Option<&str>, instructions: &str, prompt: &str) -> Result<Generation, String> {
        crate::title_jobs_live::one_hidden_turn(&self.core.adapter_registry, &self.scratch, harness, model, instructions, prompt)
    }
}

/// Stops the maintenance threads; production never does.
pub struct Maintenance {
    stop: Arc<AtomicBool>,
}

impl Maintenance {
    pub fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

/// The maintenance loops, on two threads: file upkeep (retry mirrors, import
/// outside edits) every `every`, and model jobs. Separate because one model
/// turn can take minutes, and imports must not wait behind it: open pages
/// and the notes tools read what is on disk.
pub(crate) fn spawn_loops(notes: Arc<NotesService>, backend: Arc<dyn ModelBackend + Send + Sync>, every: Duration) -> Maintenance {
    let stop = Arc::new(AtomicBool::new(false));
    let (files, flag) = (notes.clone(), stop.clone());
    let _ = std::thread::Builder::new().name("notes-files".into()).spawn(move || {
        while !flag.load(Ordering::SeqCst) {
            if let Err(error) = projector::reconcile(&files) {
                eprintln!("[notes] mirror retry failed: {error}");
            }
            if let Err(error) = projector::import_changes(&files) {
                eprintln!("[notes] import failed: {error}");
            }
            std::thread::sleep(every);
        }
    });
    let flag = stop.clone();
    let _ = std::thread::Builder::new().name("notes-jobs".into()).spawn(move || {
        let owner = format!("notes-{}", Uuid::new_v4());
        while !flag.load(Ordering::SeqCst) {
            match jobs::drain_once(&notes, backend.as_ref(), &owner) {
                Ok(Some(_)) => continue,
                Ok(None) => {}
                Err(error) => eprintln!("[notes] job failed: {error}"),
            }
            std::thread::sleep(POLL.min(every));
        }
    });
    Maintenance { stop }
}

/// Started once per host beside the other maintenance loops.
pub fn start_note_maintenance(core: Arc<BridgeCore>) {
    super::tool::install(&core);
    if let Err(error) = super::tool::serve_listener() {
        // Chats then launch without notes tools rather than with broken ones.
        eprintln!("[notes] could not start the notes-tool listener: {error}");
    }
    let backend = Arc::new(LiveBackend::new(core.clone()));
    let _ = spawn_loops(core.notes.clone(), backend, IMPORT_EVERY);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notes::jobs::{enqueue, STRUCTURE};
    use crate::notes::service::tests::service;
    use crate::notes::service::PERSON;
    use std::sync::mpsc;
    use std::sync::Mutex;

    /// A model that holds its turn until released.
    struct HeldBackend {
        entered: Mutex<mpsc::Sender<()>>,
        release: Mutex<mpsc::Receiver<()>>,
    }

    impl ModelBackend for HeldBackend {
        fn can_generate(&self, _harness: &str) -> bool {
            true
        }
        fn default_choice(&self) -> Option<(String, Option<String>)> {
            None
        }
        fn projects(&self) -> Vec<ProjectChoice> {
            Vec::new()
        }
        fn generate(&self, _harness: &str, _model: Option<&str>, _instructions: &str, _prompt: &str) -> Result<Generation, String> {
            let _ = self.entered.lock().unwrap().send(());
            let _ = self.release.lock().unwrap().recv_timeout(Duration::from_secs(30));
            Err("released".into())
        }
    }

    #[test]
    fn file_imports_continue_while_a_model_job_is_in_flight() {
        let (_dir, notes) = service();
        let notes = Arc::new(notes);
        let page = notes.capture("first line\n", PERSON).unwrap();
        notes.with_db(|db| enqueue(db, notes.space(), &page.id, STRUCTURE, Some("claude"), None)).unwrap();
        let (entered_tx, entered) = mpsc::channel();
        let (release, release_rx) = mpsc::channel();
        let backend = Arc::new(HeldBackend { entered: Mutex::new(entered_tx), release: Mutex::new(release_rx) });
        let maintenance = spawn_loops(notes.clone(), backend, Duration::from_millis(50));
        entered.recv_timeout(Duration::from_secs(10)).expect("the model job started");

        // While the model turn is held, someone edits the file.
        let path = notes.read(&page.id).unwrap().note.path.unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, text.replace("first line\n", "first line\nedited during the job\n")).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut imported = false;
        while Instant::now() < deadline && !imported {
            imported = notes.read(&page.id).unwrap().markdown.contains("edited during the job");
            std::thread::sleep(Duration::from_millis(25));
        }
        let _ = release.send(());
        maintenance.stop();
        assert!(imported, "the outside edit waited for the model turn");
    }
}
