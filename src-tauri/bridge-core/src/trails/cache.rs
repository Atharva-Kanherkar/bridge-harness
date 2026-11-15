//! Trail reads served again without deriving, and never stale.
//!
//! A [`Stamp`] is what the two databases say about their own writes: the
//! core connection's `total_changes` and `PRAGMA data_version`, and the same
//! for the notes connection. Any committed write, through either connection or
//! another one to the same file, moves it, and reading it costs nothing.
//!
//! The rule that keeps the cache honest: take the stamp before reading
//! anything, and store the result under that stamp. A write during the read
//! leaves the stored stamp older than the next read's, so that read derives
//! again. A hit means nothing at all was written since the stored read began.

use std::collections::HashMap;
use std::sync::Mutex;

use bridge_protocol::messages::{PreviewTrailBriefResult, TrailDetail, TrailSummary};
use rusqlite::Connection;

use crate::BridgeCore;

/// What the databases say about their writes, as of one moment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stamp {
    core: (u64, i64),
    notes: (u64, i64),
}

/// `(total_changes, data_version)` of one connection: the writes it made,
/// and whether any other connection committed since it last looked.
pub fn connection_stamp(db: &Connection) -> (u64, i64) {
    (db.total_changes(), db.query_row("PRAGMA data_version", [], |row| row.get(0)).unwrap_or(-1))
}

pub fn stamp(core: &BridgeCore) -> Stamp {
    // Notes first, then core: callers read notes before taking the core lock,
    // and the stamp must be no newer than anything they go on to read.
    let notes = core.notes.change_stamp();
    let core = connection_stamp(&core.db.lock().unwrap());
    Stamp { core, notes }
}

#[derive(Default)]
struct Inner {
    /// The stamp the last read-time sweep started under.
    swept: Option<Stamp>,
    list: Option<(Stamp, bool, Vec<TrailSummary>)>,
    details: HashMap<String, (Stamp, TrailDetail)>,
    previews: HashMap<String, (Stamp, PreviewTrailBriefResult)>,
}

#[derive(Default)]
pub struct TrailsCache {
    inner: Mutex<Inner>,
}

impl TrailsCache {
    /// Whether a sweep already ran with nothing written since it began.
    pub fn swept_at(&self, now: Stamp) -> bool {
        self.inner.lock().unwrap().swept == Some(now)
    }

    pub fn mark_swept(&self, at: Stamp) {
        self.inner.lock().unwrap().swept = Some(at);
    }

    pub fn list(&self, now: Stamp, include_archived: bool) -> Option<Vec<TrailSummary>> {
        match &self.inner.lock().unwrap().list {
            Some((at, archived, trails)) if *at == now && *archived == include_archived => Some(trails.clone()),
            _ => None,
        }
    }

    pub fn store_list(&self, at: Stamp, include_archived: bool, trails: &[TrailSummary]) {
        self.inner.lock().unwrap().list = Some((at, include_archived, trails.to_vec()));
    }

    pub fn detail(&self, now: Stamp, trail_id: &str) -> Option<TrailDetail> {
        self.inner.lock().unwrap().details.get(trail_id).filter(|(at, _)| *at == now).map(|(_, detail)| detail.clone())
    }

    pub fn store_detail(&self, at: Stamp, trail_id: &str, detail: &TrailDetail) {
        let mut inner = self.inner.lock().unwrap();
        // Entries from an older stamp can never hit again.
        inner.details.retain(|_, (stamp, _)| *stamp == at);
        inner.details.insert(trail_id.to_owned(), (at, detail.clone()));
    }

    pub fn preview(&self, now: Stamp, trail_id: &str) -> Option<PreviewTrailBriefResult> {
        self.inner.lock().unwrap().previews.get(trail_id).filter(|(at, _)| *at == now).map(|(_, preview)| preview.clone())
    }

    pub fn store_preview(&self, at: Stamp, trail_id: &str, preview: &PreviewTrailBriefResult) {
        let mut inner = self.inner.lock().unwrap();
        inner.previews.retain(|_, (stamp, _)| *stamp == at);
        inner.previews.insert(trail_id.to_owned(), (at, preview.clone()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use bridge_protocol::messages::{TrailAddedBy, TrailMemberKind, TrailMemberRef};

    /// A core over a file database, so a second connection can write to it.
    fn core(dir: &std::path::Path) -> Arc<BridgeCore> {
        let mut core = BridgeCore::for_tests(dir);
        core.db = Mutex::new(crate::store::open(&dir.join("bridge.db")).unwrap());
        Arc::new(core)
    }

    fn trail(core: &Arc<BridgeCore>) -> String {
        let db = core.db.lock().unwrap();
        crate::trails::timeline::tests::add_session(&db, "s1", "codex", "Build it", Some("2026-11-10T00:00:00Z"));
        let trail = crate::trails::store::create(&db, "T", None, "t0").unwrap();
        crate::trails::store::add_member(&db, &trail.id, &TrailMemberRef { kind: TrailMemberKind::Session, reference: "s1".into() }, TrailAddedBy::Person, "t0").unwrap();
        trail.id
    }

    fn chat_title(core: &Arc<BridgeCore>, trail: &str) -> String {
        crate::api::read_trail(core, trail, false).unwrap().trail.events.iter().find(|event| event.member.kind == TrailMemberKind::Session).unwrap().title.clone()
    }

    #[test]
    fn an_unchanged_stamp_serves_the_same_detail_without_deriving() {
        let dir = tempfile::tempdir().unwrap();
        let core = core(dir.path());
        let id = trail(&core);
        let first = crate::api::read_trail(&core, &id, false).unwrap().trail;
        assert_eq!(crate::api::read_trail(&core, &id, false).unwrap().trail, first);
        // Plant a marker under the current stamp: a hit returns it untouched,
        // which only happens if the read does not derive.
        let mut marked = first.clone();
        marked.summary.title = "served from the cache".into();
        core.trails_cache.store_detail(stamp(&core), &id, &marked);
        assert_eq!(crate::api::read_trail(&core, &id, false).unwrap().trail.summary.title, "served from the cache");
        // A refresh never serves the cache.
        assert_eq!(crate::api::read_trail(&core, &id, true).unwrap().trail.summary.title, "T");
        let listed = crate::api::list_trails(&core, false).unwrap();
        assert_eq!(crate::api::list_trails(&core, false).unwrap(), listed);
        // The archived flag is part of the key: an archived trail shows only
        // when asked for, cached or not.
        let archived = crate::trails::store::create(&core.db.lock().unwrap(), "Old", None, "t0").unwrap();
        crate::trails::store::set_archived(&core.db.lock().unwrap(), &archived.id, true, "t1").unwrap();
        for _ in 0..2 {
            assert_eq!(crate::api::list_trails(&core, false).unwrap().trails.len(), 1);
            assert_eq!(crate::api::list_trails(&core, true).unwrap().trails.len(), 2);
        }
    }

    #[test]
    fn a_write_through_the_core_invalidates() {
        let dir = tempfile::tempdir().unwrap();
        let core = core(dir.path());
        let id = trail(&core);
        assert_eq!(chat_title(&core, &id), "Build it");
        let listed = crate::api::list_trails(&core, false).unwrap();
        core.db.lock().unwrap().execute("UPDATE sessions SET title='Renamed' WHERE id='s1'", []).unwrap();
        assert_eq!(chat_title(&core, &id), "Renamed");
        assert_ne!(crate::api::list_trails(&core, false).unwrap(), listed);
    }

    #[test]
    fn a_write_through_the_notes_store_invalidates() {
        let dir = tempfile::tempdir().unwrap();
        let core = core(dir.path());
        let id = trail(&core);
        let note = core.notes.create(crate::notes::service::CreatePage { title: Some("Plan".into()), ..Default::default() }, "person").unwrap();
        crate::trails::store::add_member(&core.db.lock().unwrap(), &id, &TrailMemberRef { kind: TrailMemberKind::Note, reference: note.id.clone() }, TrailAddedBy::Person, "t1").unwrap();
        let title = |core: &Arc<BridgeCore>| crate::api::read_trail(core, &id, false).unwrap().trail.events.iter().find(|event| event.member.kind == TrailMemberKind::Note).unwrap().title.clone();
        assert_eq!(title(&core), "Plan");
        core.notes.update(crate::notes::service::UpdateNote { id: note.id.clone(), title: Some("Plan v2".into()), ..Default::default() }, "person").unwrap();
        assert_eq!(title(&core), "Plan v2");
    }

    #[test]
    fn a_write_from_another_connection_invalidates() {
        let dir = tempfile::tempdir().unwrap();
        let core = core(dir.path());
        let id = trail(&core);
        assert_eq!(chat_title(&core, &id), "Build it");
        let before = stamp(&core);
        // Not through the core: its own change count does not move, the
        // file's data version does.
        let other = Connection::open(dir.path().join("bridge.db")).unwrap();
        other.execute("UPDATE sessions SET title='Elsewhere' WHERE id='s1'", []).unwrap();
        drop(other);
        assert_ne!(stamp(&core), before);
        assert_eq!(chat_title(&core, &id), "Elsewhere");
    }

    #[test]
    fn the_read_sweep_runs_again_after_any_write() {
        let dir = tempfile::tempdir().unwrap();
        let core = core(dir.path());
        let id = trail(&core);
        crate::api::list_trails(&core, false).unwrap();
        assert!(core.trails_cache.swept_at(stamp(&core)), "nothing written since the sweep began");
        // A link appears with no event at all; the next read still follows it.
        crate::trails::timeline::tests::attach_pr(&core.db.lock().unwrap(), "s1", "https://github.com/o/r/pull/12", "t2", None);
        assert!(!core.trails_cache.swept_at(stamp(&core)));
        let read = crate::api::read_trail(&core, &id, false).unwrap().trail;
        assert!(read.members.iter().any(|member| member.kind == TrailMemberKind::PullRequest), "{:?}", read.members);
    }
}
