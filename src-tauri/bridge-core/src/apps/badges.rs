//! Badge counts: what each app is waiting on the person for.
//!
//! Each count uses the same rule the frontend used before core owned it, so a
//! badge does not change meaning when it moves. An app that is off has no
//! badge, and a count that cannot be read is no badge rather than a failed
//! app list.

use std::sync::{Mutex, PoisonError};

use bridge_protocol::messages::{AppBadge, AppSummary, TrailState};
use rusqlite::Connection;

use super::builtin::{builtins, BadgeSource, BuiltinApp};
use super::registry;
use crate::trails::timeline::NoteSource;
use crate::BridgeError;

fn count(db: &Connection, sql: &str) -> Result<u32, BridgeError> {
    let value: i64 = db.query_row(sql, [], |row| row.get(0))?;
    Ok(u32::try_from(value).unwrap_or(u32::MAX))
}

/// Runs waiting for approval, and finished runs the person has not read
/// (`src/components/workflows/notify.ts` `needsYouCount`).
pub fn workflow_runs(db: &Connection) -> Result<u32, BridgeError> {
    count(
        db,
        "SELECT COUNT(*) FROM workflow_runs
         WHERE status='waiting_approval' OR (unread=1 AND status NOT IN ('running','queued'))",
    )
}

/// Proposals waiting for review in the account scope. Uncapped: the list
/// read the frontend used stopped at 50.
pub fn memory_proposals(db: &Connection) -> Result<u32, BridgeError> {
    let value: i64 = db.query_row(
        "SELECT COUNT(*) FROM memory_records WHERE scope_key=?1 AND status='proposed'",
        [crate::memory_ledger::account_memory_scope()],
        |row| row.get(0),
    )?;
    Ok(u32::try_from(value).unwrap_or(u32::MAX))
}

/// Trails that are not archived and need the person (`src/trails.ts`
/// `needsYouCount`).
pub fn trails_needing_you(db: &Connection, notes: &dyn NoteSource) -> Result<u32, BridgeError> {
    let mut waiting = 0u32;
    for row in crate::trails::store::list(db, false)? {
        if crate::trails::timeline::summary(db, notes, &row)?.state == TrailState::NeedsYou {
            waiting = waiting.saturating_add(1);
        }
    }
    Ok(waiting)
}

/// The trails count, kept until the next `trails/changed`.
///
/// `apps/changed` follows every memory and workflow-run hint, and each one
/// makes every client reread the app list. Deriving every trail summary on
/// each of those reads would turn a workflow run's status hints into
/// whole-trail derivations under the database lock. The frontend refreshed
/// trails only on `trails/changed`; the cached count keeps that freshness.
#[derive(Debug, Default)]
pub struct BadgeCache {
    /// `(trails generation it was counted at, count)`.
    trails: Mutex<Option<(u64, u32)>>,
}

impl BadgeCache {
    /// The cached count, when it was taken at `generation` or later.
    pub fn trails(&self, generation: u64) -> Option<u32> {
        match *self.trails.lock().unwrap_or_else(PoisonError::into_inner) {
            Some((counted_at, count)) if counted_at >= generation => Some(count),
            _ => None,
        }
    }

    fn store_trails(&self, generation: u64, count: u32) {
        let mut slot = self.trails.lock().unwrap_or_else(PoisonError::into_inner);
        if slot.is_none_or(|(counted_at, _)| counted_at <= generation) {
            *slot = Some((generation, count));
        }
    }
}

/// What a badge read may reuse: the cache and the trails generation the
/// caller saw before reading notes.
#[derive(Clone, Copy)]
pub struct Cached<'a> {
    pub cache: &'a BadgeCache,
    pub trails_generation: u64,
}

fn badge(db: &Connection, notes: &dyn NoteSource, app: &BuiltinApp, cached: Option<Cached<'_>>) -> Option<AppBadge> {
    let counted = match app.badge {
        BadgeSource::None => return None,
        BadgeSource::WorkflowRuns => workflow_runs(db),
        BadgeSource::MemoryProposals => memory_proposals(db),
        BadgeSource::TrailsNeedingYou => match cached {
            Some(Cached { cache, trails_generation }) => match cache.trails(trails_generation) {
                Some(count) => Ok(count),
                None => trails_needing_you(db, notes).inspect(|count| cache.store_trails(trails_generation, *count)),
            },
            None => trails_needing_you(db, notes),
        },
    };
    match counted {
        Ok(count) => Some(AppBadge { count }),
        Err(error) => {
            eprintln!("apps: the {} badge could not be read: {error}", app.manifest.id);
            None
        }
    }
}

/// Every app with its state on this Mac and its badge. `notes` is read
/// before the caller takes the database lock (see `NoteSnapshot`); with a
/// cache hit for trails it is not consulted.
pub fn summaries(db: &Connection, notes: &dyn NoteSource, cached: Option<Cached<'_>>) -> Result<Vec<AppSummary>, BridgeError> {
    let off = registry::disabled_ids(db)?;
    let pins = registry::pins(db)?;
    Ok(builtins()
        .iter()
        .map(|app| {
            let id = &app.manifest.id;
            let enabled = !off.contains(id);
            AppSummary {
                manifest: app.manifest.clone(),
                enabled,
                can_disable: app.can_disable,
                pinned: pins.iter().position(|pin| pin == id).map(|index| index as u32),
                badge: if enabled { badge(db, notes, app, cached) } else { None },
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trails::timeline::NoteSnapshot;
    use std::path::Path;

    fn database() -> Connection {
        crate::store::open(Path::new(":memory:")).unwrap()
    }

    fn run(db: &Connection, id: &str, status: &str, unread: bool) {
        db.execute(
            "INSERT INTO workflows(id, name, current_revision_id, created_at, updated_at) VALUES(?1,'W','r','t','t')",
            [id],
        )
        .unwrap();
        db.execute(
            "INSERT INTO workflow_runs(id, workflow_id, revision_id, trigger_kind, occasion, idempotency_key, status, unread, created_at)
             VALUES(?1,?1,'r','manual','o',?1,?2,?3,'t')",
            rusqlite::params![id, status, unread as i64],
        )
        .unwrap();
    }

    fn badge_of(summaries: &[AppSummary], id: &str) -> Option<u32> {
        summaries.iter().find(|summary| summary.manifest.id == id).unwrap().badge.map(|badge| badge.count)
    }

    #[test]
    fn workflow_badge_matches_the_frontend_predicate() {
        let db = database();
        run(&db, "a", "waiting_approval", false);
        run(&db, "b", "succeeded", true);
        run(&db, "c", "failed", true);
        run(&db, "d", "succeeded", false);
        run(&db, "e", "running", true);
        run(&db, "f", "queued", true);
        run(&db, "g", "waiting_quota", true);
        assert_eq!(workflow_runs(&db).unwrap(), 4, "a, b, c and g");
    }

    #[test]
    fn memory_badge_counts_proposals_without_a_cap() {
        let db = database();
        let scope = crate::memory_ledger::account_memory_scope();
        for index in 0..60 {
            db.execute(
                &format!(
                    "INSERT INTO memory_records(id, scope_key, kind, body, provenance, status, created_at, updated_at)
                     VALUES('m{index}', '{scope}', 'fact', 'fact {index}', 'extracted', 'proposed', 't', 't')"
                ),
                [],
            )
            .unwrap();
        }
        assert_eq!(memory_proposals(&db).unwrap(), 60);
    }

    #[test]
    fn trails_badge_counts_needs_you() {
        let db = database();
        assert_eq!(trails_needing_you(&db, &NoteSnapshot::default()).unwrap(), 0);
        let trail = crate::trails::store::create(&db, "Ship apps", None, "t").unwrap();
        crate::trails::timeline::tests::add_session(&db, "chat", "claude", "Ship apps", Some("t"));
        crate::trails::store::add_member(&db, &trail.id, &crate::trails::store::tests::session("chat"), bridge_protocol::messages::TrailAddedBy::Person, "t").unwrap();
        crate::trails::store::create(&db, "Quiet", None, "t").unwrap();
        assert_eq!(trails_needing_you(&db, &NoteSnapshot::default()).unwrap(), 0);
        db.execute(
            "INSERT INTO attention_events(id, kind, severity, session_id, title, created_at) VALUES('a1','needs_you','actionable','chat','Approve the deploy','t')",
            [],
        )
        .unwrap();
        assert_eq!(trails_needing_you(&db, &NoteSnapshot::default()).unwrap(), 1);
        crate::trails::store::set_archived(&db, &trail.id, true, "t").unwrap();
        assert_eq!(trails_needing_you(&db, &NoteSnapshot::default()).unwrap(), 0, "archived trails do not count");
    }

    #[test]
    fn off_apps_have_no_badge() {
        let db = database();
        let notes = NoteSnapshot::default();
        let summaries = summaries(&db, &notes, None).unwrap();
        assert_eq!(summaries.len(), 12);
        assert_eq!(badge_of(&summaries, "memory"), Some(0), "a zero count is still a badge");
        assert_eq!(badge_of(&summaries, "notes"), None, "notes has no badge source");
        registry::set_enabled(&db, "memory", false, "t").unwrap();
        let summaries = super::summaries(&db, &notes, None).unwrap();
        assert_eq!(badge_of(&summaries, "memory"), None);
        assert!(!summaries.iter().find(|summary| summary.manifest.id == "memory").unwrap().enabled);
    }

    #[test]
    fn a_failing_source_is_no_badge_not_a_failed_list() {
        let db = database();
        db.execute_batch("DROP TABLE workflow_runs").unwrap();
        let summaries = summaries(&db, &NoteSnapshot::default(), None).unwrap();
        assert_eq!(badge_of(&summaries, "workflows"), None);
        assert_eq!(badge_of(&summaries, "memory"), Some(0));
    }
}
