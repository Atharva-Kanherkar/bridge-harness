//! The production side of notes: the maintenance loop and the live backend.
//!
//! One thread per host drains `model_jobs` (Structure and Turn into tasks,
//! queued only when someone asked) and imports edits made to mirrored files
//! outside Bridge. Model turns run through `title_jobs_live::one_hidden_turn`:
//! tool-less, no `sessions` row, spend recorded on the job.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use rusqlite::params;
use uuid::Uuid;

use super::jobs::{self, ModelBackend, ProjectChoice};
use super::projector;
use super::service::ProjectCheck;
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

/// Started once per host beside the other maintenance loops.
pub fn start_note_maintenance(core: Arc<BridgeCore>) {
    let _ = std::thread::Builder::new().name("notes".into()).spawn(move || {
        let owner = format!("notes-{}", Uuid::new_v4());
        let backend = LiveBackend::new(core.clone());
        let mut last_import = Instant::now() - IMPORT_EVERY;
        loop {
            if last_import.elapsed() >= IMPORT_EVERY {
                if let Err(error) = projector::import_changes(&core.notes) {
                    eprintln!("[notes] import failed: {error}");
                }
                last_import = Instant::now();
            }
            match jobs::drain_once(&core.notes, &backend, &owner) {
                Ok(Some(_)) => continue,
                Ok(None) => {}
                Err(error) => eprintln!("[notes] job failed: {error}"),
            }
            std::thread::sleep(POLL);
        }
    });
}
