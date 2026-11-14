//! The trail maintenance loop: follow new hard links shortly after the records
//! that carry them move, and tell clients once.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use bridge_protocol::messages::{TrailMemberAdded, TrailsChanged};

use super::auto;
use super::timeline::NoteSnapshot;
use crate::events::{CoreEvent, ReceiveError};
use crate::BridgeCore;

/// Events arriving within this window share one sweep.
pub const DEBOUNCE: Duration = Duration::from_millis(1500);

/// What an event means for trails.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Relevance {
    None,
    /// A new link may exist (a chat was created or forked); sweep, and say
    /// something only if the sweep found one.
    Links,
    /// A member's own state may have moved (checks, a draft, attention):
    /// sweep and always send a refetch hint.
    State,
}

pub fn relevance(event: &CoreEvent) -> Relevance {
    match event {
        CoreEvent::StateChanged => Relevance::Links,
        CoreEvent::GithubSessionPrsChanged { .. }
        | CoreEvent::GithubChecksChanged { .. }
        | CoreEvent::GithubCiFinished { .. }
        | CoreEvent::OutgoingChanged { .. }
        | CoreEvent::NotesPageChanged(_)
        | CoreEvent::AttentionChanged => Relevance::State,
        _ => Relevance::None,
    }
}

/// Sweep every open trail and publish what joined, one notification per
/// trail. With `hint`, also publish a plain refetch hint when there are trails.
pub fn sweep_and_publish(core: &Arc<BridgeCore>, hint: bool) -> Vec<TrailMemberAdded> {
    let any = {
        let db = core.db.lock().unwrap();
        super::store::list(&db, false).map(|trails| !trails.is_empty()).unwrap_or(false)
    };
    if !any {
        return Vec::new();
    }
    let notes = NoteSnapshot::all(core.notes.as_ref());
    let now = chrono::Utc::now().to_rfc3339();
    let added = {
        let db = core.db.lock().unwrap();
        match auto::sweep(&db, &notes, &now) {
            Ok(added) => added,
            Err(error) => {
                eprintln!("trails: sweep failed: {error}");
                Vec::new()
            }
        }
    };
    publish_added(core, &added);
    if hint && added.is_empty() {
        core.events.publish(CoreEvent::TrailsChanged(TrailsChanged { trail_id: None, added: Vec::new() }));
    }
    added
}

pub fn publish_added(core: &Arc<BridgeCore>, added: &[TrailMemberAdded]) {
    let mut by_trail: BTreeMap<&str, Vec<TrailMemberAdded>> = BTreeMap::new();
    for item in added {
        by_trail.entry(item.trail_id.as_str()).or_default().push(item.clone());
    }
    for (trail_id, added) in by_trail {
        core.events.publish(CoreEvent::TrailsChanged(TrailsChanged { trail_id: Some(trail_id.to_owned()), added }));
    }
}

/// Started by both hosts with the other maintenance loops.
pub fn start_trail_maintenance(core: Arc<BridgeCore>) {
    let mut events = core.events.subscribe();
    std::thread::Builder::new()
        .name("bridge-trails".into())
        .spawn(move || {
            sweep_and_publish(&core, false);
            loop {
                let first = match events.blocking_recv() {
                    Ok(event) => relevance(&event),
                    // Missed events may have carried links: sweep anyway.
                    Err(ReceiveError::Lagged(_)) => Relevance::State,
                    Err(ReceiveError::Closed) => break,
                };
                if first == Relevance::None {
                    continue;
                }
                std::thread::sleep(DEBOUNCE);
                let mut strongest = first;
                loop {
                    match events.try_next() {
                        Ok(event) => strongest = strongest.max(relevance(&event)),
                        Err(true) => strongest = Relevance::State,
                        Err(false) => break,
                    }
                }
                sweep_and_publish(&core, strongest == Relevance::State);
            }
        })
        .ok();
}

#[cfg(test)]
mod tests {
    use super::*;
    use bridge_protocol::messages::{TrailAddedBy, TrailMemberKind, TrailMemberRef};

    #[test]
    fn trails_changed_is_not_relevant_to_itself() {
        assert_eq!(relevance(&CoreEvent::TrailsChanged(TrailsChanged { trail_id: None, added: Vec::new() })), Relevance::None);
        assert_eq!(relevance(&CoreEvent::StateChanged), Relevance::Links);
        assert_eq!(relevance(&CoreEvent::OutgoingChanged { session_id: "s".into(), id: "o".into() }), Relevance::State);
    }

    #[test]
    fn a_sweep_publishes_each_trails_additions_once() {
        let dir = tempfile::tempdir().unwrap();
        let core = Arc::new(BridgeCore::for_tests(dir.path()));
        assert!(sweep_and_publish(&core, true).is_empty(), "no trails, nothing to do");
        {
            let db = core.db.lock().unwrap();
            crate::trails::timeline::tests::add_session(&db, "s1", "codex", "Build", None);
            crate::trails::timeline::tests::attach_pr(&db, "s1", "https://github.com/o/r/pull/12", "t", None);
            let trail = crate::trails::store::create(&db, "T", None, "t0").unwrap();
            crate::trails::store::add_member(&db, &trail.id, &TrailMemberRef { kind: TrailMemberKind::Session, reference: "s1".into() }, TrailAddedBy::Person, "t0").unwrap();
        }
        let mut events = core.events.subscribe();
        assert_eq!(sweep_and_publish(&core, true).len(), 1);
        let mut seen = Vec::new();
        while let Ok(event) = events.try_recv() {
            if let CoreEvent::TrailsChanged(changed) = event {
                seen.push(changed);
            }
        }
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].added[0].label, "PR #12 Archive a chat");
        sweep_and_publish(&core, true);
        let mut hints = 0;
        while let Ok(event) = events.try_recv() {
            if let CoreEvent::TrailsChanged(changed) = event {
                assert!(changed.added.is_empty() && changed.trail_id.is_none());
                hints += 1;
            }
        }
        assert_eq!(hints, 1, "a second sweep sends a plain hint, no additions");
    }
}
