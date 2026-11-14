//! Times the reads the Notes screen makes when it opens.
//!
//! The screen's cold open is three serial core calls: `notes/list_notes` for the
//! rows, `notes/read_page` for the system database (the tabs), then
//! `notes/query_database` for the open tab's rows. This example seeds a store
//! the size of a busy one and prints the median of each call, plus the serial
//! chain the person waits on.
//!
//!   cargo run --release -p bridge-core --example notes_perf -- [pages] [runs]
//!
//! The default 600 pages is about a year of daily capture with a few
//! agent-written pages and tasks. To time a real store instead, pass its
//! `notes.db` directory as the first argument:
//!
//!   cargo run --release -p bridge-core --example notes_perf -- --store <data dir>
//!
//! Nothing is written to a real store: it is only read.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use bridge_core::notes::query::QuerySpec;
use bridge_core::notes::service::{CreatePage, EditPage, ListFilter, NotesService, UpdateNote};
use bridge_protocol::messages::NoteAppend;

const WORDS: [&str; 12] = [
    "bridge", "notes", "capture", "graphite", "paper", "session", "model", "harness", "workspace",
    "transcript", "memory", "skill",
];

fn body_of(index: usize) -> String {
    let mut body = String::new();
    for line in 0..8 {
        body.push_str(&format!("note {index} line {line}: "));
        for word in 0..9 {
            body.push_str(WORDS[(index + line + word) % WORDS.len()]);
            body.push(' ');
        }
        body.push('\n');
    }
    body
}

fn median(samples: &[Duration]) -> Duration {
    let mut sorted = samples.to_vec();
    sorted.sort();
    sorted[sorted.len() / 2]
}

fn p95(samples: &[Duration]) -> Duration {
    let mut sorted = samples.to_vec();
    sorted.sort();
    sorted[((sorted.len() as f64 * 0.95).floor() as usize).min(sorted.len() - 1)]
}

fn fmt(duration: Duration) -> String {
    format!("{:.1}ms", duration.as_secs_f64() * 1000.0)
}

/// A store people actually accumulate: plain pages, pages with children,
/// tasks under pages, tags, and a few agent-written pages, each edited twice
/// after it was made.
fn seed(service: &NotesService, pages: usize) {
    for index in 0..pages {
        let task = index % 6 == 5;
        let parent = if index % 9 == 8 {
            Some(service.create(CreatePage { title: Some(format!("Project {index}")), content: Some(body_of(index)), tags: vec!["project".into()], ..Default::default() }, "person").unwrap().id)
        } else {
            None
        };
        let note = service
            .create(
                CreatePage {
                    title: Some(format!("Page {index}")),
                    content: Some(body_of(index)),
                    tags: if index % 4 == 0 { vec!["idea".into()] } else { Vec::new() },
                    kind: Some(if task { "task".to_string() } else { "note".to_string() }),
                    state: Some(if index % 11 == 0 { "archived".into() } else if task { "doing".into() } else { "open".into() }),
                    parent_id: parent,
                    ..Default::default()
                },
                if index % 7 == 0 { "agent:claude:seed" } else { "person" },
            )
            .unwrap();
        service.update(UpdateNote { id: note.id.clone(), title: Some(format!("Page {index} revised")), ..Default::default() }, "person").unwrap();
        service
            .edit(EditPage { id: note.id, append: Some(NoteAppend { content: "- one more line\n".into(), position: "end".into() }), ..Default::default() }, "person")
            .unwrap();
    }
}

fn main() {
    let mut args = std::env::args().skip(1).peekable();
    let store = match args.peek().map(String::as_str) {
        Some("--store") => {
            args.next();
            Some(PathBuf::from(args.next().expect("--store needs a data dir")))
        }
        _ => None,
    };
    let pages: usize = args.next().and_then(|value| value.parse().ok()).unwrap_or(600);
    let runs: usize = args.next().and_then(|value| value.parse().ok()).unwrap_or(15);

    // A seeded store lives in a scratch directory that goes away with this run.
    let scratch = (store.is_none()).then(|| tempfile::tempdir().unwrap());
    let data_dir: PathBuf = store.clone().unwrap_or_else(|| scratch.as_ref().unwrap().path().to_path_buf());
    let seeded = Instant::now();
    let service = NotesService::open(&data_dir, std::sync::Arc::new(|_: &str| true)).unwrap();
    let pages = match &store {
        Some(_) => service.with_db(|db| Ok(db.query_row("SELECT COUNT(*) FROM pages", [], |row| row.get::<_, i64>(0))?)).unwrap() as usize,
        None => {
            seed(&service, pages);
            pages
        }
    };
    let seeded = seeded.elapsed();

    let system = service.system_database().unwrap();
    let filter = ListFilter { include_archived: true, limit: Some(500), ..Default::default() };
    let all_view = service.database(&system).unwrap().views.iter().min_by_key(|(_, view)| view.order).map(|(id, _)| id.clone());
    let spec = |notes_only: bool| QuerySpec { view_id: all_view.clone(), limit: Some(500), notes_only, ..Default::default() };

    let mut list_times = Vec::new();
    let mut system_times = Vec::new();
    let mut cells_times = Vec::new();
    let mut pages_times = Vec::new();
    let mut before_chain = Vec::new();
    let mut after_chain = Vec::new();
    for _ in 0..runs {
        let started = Instant::now();
        let notes = service.list(&filter).unwrap();
        list_times.push(started.elapsed());
        assert!(notes.len() <= 500);

        let started = Instant::now();
        let read = service.read(&system).unwrap();
        system_times.push(started.elapsed());
        assert!(read.database.is_some());

        let started = Instant::now();
        let result = service.query_database(&system, &spec(false)).unwrap();
        cells_times.push(started.elapsed());
        assert!(!result.rows.is_empty());

        let started = Instant::now();
        let result = service.query_database(&system, &spec(true)).unwrap();
        pages_times.push(started.elapsed());
        assert!(!result.rows.is_empty());

        // The chain the screen waits on, with the tab read as it was ...
        let started = Instant::now();
        let _ = service.list(&filter).unwrap();
        let _ = service.read(&system).unwrap();
        let _ = service.query_database(&system, &spec(false)).unwrap();
        before_chain.push(started.elapsed());

        // ... and as it is.
        let started = Instant::now();
        let _ = service.list(&filter).unwrap();
        let _ = service.read(&system).unwrap();
        let _ = service.query_database(&system, &spec(true)).unwrap();
        after_chain.push(started.elapsed());
    }

    println!("pages: {pages} (seeded in {})", fmt(seeded));
    println!("tab rows: {}", service.query_database(&system, &spec(true)).unwrap().rows.len());
    println!();
    println!("{:<34} {:>9} {:>9}", "read", "median", "p95");
    println!("{:<34} {:>9} {:>9}", "notes/list_notes", fmt(median(&list_times)), fmt(p95(&list_times)));
    println!("{:<34} {:>9} {:>9}", "notes/read_page (system)", fmt(median(&system_times)), fmt(p95(&system_times)));
    println!("{:<34} {:>9} {:>9}", "notes/query_database (cells)", fmt(median(&cells_times)), fmt(p95(&cells_times)));
    println!("{:<34} {:>9} {:>9}", "notes/query_database (pages)", fmt(median(&pages_times)), fmt(p95(&pages_times)));
    println!();
    println!("{:<34} {:>9} {:>9}", "cold open, with cells", fmt(median(&before_chain)), fmt(p95(&before_chain)));
    println!("{:<34} {:>9} {:>9}", "cold open, rows as pages", fmt(median(&after_chain)), fmt(p95(&after_chain)));
}
