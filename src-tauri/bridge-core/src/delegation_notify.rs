//! Opt-out for worker routing notices that cost an orchestrator a model turn.
//!
//! Every notice is classified once in [`NoticeKind::class`]; the level (a
//! global default, optionally overridden per orchestrator session) decides
//! which classes reach the orchestrator. The human transcript card is written
//! by the caller either way, and nothing here touches policy.

use crate::{BridgeCore, BridgeError};
use bridge_protocol::messages::{
    DelegationNotifyLevel, DelegationNotifySettings, DelegationNotifySettingsView,
    SaveDelegationNotifySettingsParams,
};
use rusqlite::{params, Connection, OptionalExtension};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};

const GLOBAL_KIND: &str = "delegation_notify_settings";
const GLOBAL_ID: &str = "global";
const SESSION_KIND: &str = "delegation_notify_level";
/// Identical informational notices for one child inside this window collapse
/// into the first; the next one delivered carries how many were dropped.
const COALESCE_WINDOW: Duration = Duration::from_secs(30);

/// Every routing notice sent to an orchestrator. Adding a variant without a
/// class in [`NoticeKind::class`] does not compile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NoticeKind {
    WorkerUnblocked,
    WorkerBlockedOnApproval,
    WorkerSteeredByUser,
    LaunchApproved,
    LaunchAwaitingApproval,
    WorkerRerouted,
    LaunchDeclined,
    LaunchFailed,
    WorkerStopped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoticeClass {
    /// Nothing to act on ("approval resolved, keep waiting").
    Informational,
    /// Changes what the orchestrator should do next, but is not a final answer.
    Actionable,
    /// A failure or terminal outcome; never suppressed.
    Essential,
}

impl NoticeKind {
    pub const fn class(self) -> NoticeClass {
        match self {
            Self::WorkerUnblocked
            | Self::WorkerBlockedOnApproval
            | Self::WorkerSteeredByUser
            | Self::LaunchApproved => NoticeClass::Informational,
            Self::LaunchAwaitingApproval | Self::WorkerRerouted => NoticeClass::Actionable,
            Self::LaunchDeclined | Self::LaunchFailed | Self::WorkerStopped => NoticeClass::Essential,
        }
    }
}

pub fn level_delivers(level: DelegationNotifyLevel, class: NoticeClass) -> bool {
    match (level, class) {
        (_, NoticeClass::Essential) => true,
        (DelegationNotifyLevel::All, _) => true,
        (DelegationNotifyLevel::Actionable, NoticeClass::Actionable) => true,
        _ => false,
    }
}

pub fn load_global(db: &Connection) -> Result<DelegationNotifySettings, BridgeError> {
    let payload: Option<String> = db
        .query_row(
            "SELECT payload FROM configuration_entries WHERE kind=?1 AND id=?2",
            params![GLOBAL_KIND, GLOBAL_ID],
            |row| row.get(0),
        )
        .optional()?;
    payload
        .map(|payload| serde_json::from_str(&payload).map_err(|error| BridgeError::Invalid(error.to_string())))
        .unwrap_or_else(|| Ok(DelegationNotifySettings::default()))
}

fn load_override(db: &Connection, session_id: &str) -> Result<Option<DelegationNotifyLevel>, BridgeError> {
    let payload: Option<String> = db
        .query_row(
            "SELECT payload FROM configuration_entries WHERE kind=?1 AND id=?2",
            params![SESSION_KIND, session_id],
            |row| row.get(0),
        )
        .optional()?;
    payload
        .map(|payload| serde_json::from_str(&payload).map_err(|error| BridgeError::Invalid(error.to_string())))
        .transpose()
}

pub fn view(db: &Connection, session_id: Option<&str>) -> Result<DelegationNotifySettingsView, BridgeError> {
    let global_level = load_global(db)?.level;
    let session_override = match session_id {
        Some(session_id) => load_override(db, session_id)?,
        None => None,
    };
    Ok(DelegationNotifySettingsView {
        level: session_override.unwrap_or(global_level),
        global_level,
        session_override,
    })
}

pub fn save(db: &Connection, params: &SaveDelegationNotifySettingsParams) -> Result<DelegationNotifySettingsView, BridgeError> {
    match (&params.session_id, params.level) {
        (Some(session_id), Some(level)) => upsert(db, SESSION_KIND, session_id, &json(&level)?)?,
        (Some(session_id), None) => {
            db.execute(
                "DELETE FROM configuration_entries WHERE kind=?1 AND id=?2",
                params![SESSION_KIND, session_id],
            )?;
        }
        (None, Some(level)) => upsert(
            db,
            GLOBAL_KIND,
            GLOBAL_ID,
            &json(&DelegationNotifySettings { level })?,
        )?,
        (None, None) => {
            return Err(BridgeError::Invalid("a global notification level must be set, not cleared".into()))
        }
    }
    view(db, params.session_id.as_deref())
}

fn json<T: serde::Serialize>(value: &T) -> Result<String, BridgeError> {
    serde_json::to_string(value).map_err(|error| BridgeError::Invalid(error.to_string()))
}

fn upsert(db: &Connection, kind: &str, id: &str, payload: &str) -> Result<(), BridgeError> {
    db.execute(
        "INSERT INTO configuration_entries(kind,id,payload,created_at,updated_at)
         VALUES(?1,?2,?3,?4,?4) ON CONFLICT(kind,id) DO UPDATE SET payload=excluded.payload,updated_at=excluded.updated_at",
        params![kind, id, payload, chrono::Utc::now().to_rfc3339()],
    )?;
    Ok(())
}

/// Effective level for an orchestrator. A read failure falls back to `All` so
/// a storage hiccup never hides a notice.
pub fn effective_level(db: &Connection, session_id: &str) -> DelegationNotifyLevel {
    view(db, Some(session_id)).map(|view| view.level).unwrap_or_default()
}

#[derive(Default)]
struct Coalescer {
    last_sent: HashMap<(String, String, NoticeKind, String), Instant>,
    dropped: HashMap<(String, String, NoticeKind, String), u32>,
}

static COALESCER: OnceLock<Mutex<Coalescer>> = OnceLock::new();

/// `Some(count)` when the notice should be sent (count of identical ones
/// dropped since the previous send), `None` when it falls inside the window.
fn coalesce(parent: &str, child: &str, kind: NoticeKind, fingerprint: &str, now: Instant) -> Option<u32> {
    let key = (parent.to_owned(), child.to_owned(), kind, fingerprint.to_owned());
    let mut state = COALESCER.get_or_init(Default::default).lock().unwrap();
    if state.last_sent.get(&key).is_some_and(|sent| now.duration_since(*sent) < COALESCE_WINDOW) {
        *state.dropped.entry(key).or_default() += 1;
        return None;
    }
    state.last_sent.insert(key.clone(), now);
    Some(state.dropped.remove(&key).unwrap_or(0))
}

/// The one place a routing notice becomes an orchestrator model turn. Returns
/// whether the orchestrator was actually told, which is what the transcript
/// event's `orchestratorNotified` records.
pub fn deliver_routing_notice(
    core: &Arc<BridgeCore>,
    parent_session_id: &str,
    child_session_id: Option<&str>,
    kind: NoticeKind,
    notice: &str,
) -> bool {
    let class = kind.class();
    let level = effective_level(&core.db.lock().unwrap(), parent_session_id);
    if !level_delivers(level, class) {
        return false;
    }
    let mut body = notice.to_owned();
    // Scoped by data directory so two cores in one process never share a window.
    let scope = core.database_path.to_string_lossy();
    if class == NoticeClass::Informational {
        let child = child_session_id.unwrap_or_default();
        let fingerprint = serde_json::from_str::<serde_json::Value>(notice)
            .ok()
            .and_then(|value| value.get("outcome").and_then(|outcome| outcome.as_str().map(str::to_owned)))
            .unwrap_or_default();
        match coalesce(&format!("{scope}|{parent_session_id}"), child, kind, &fingerprint, Instant::now()) {
            None => return false,
            Some(0) => {}
            Some(count) => {
                if let Ok(mut value) = serde_json::from_str::<serde_json::Value>(notice) {
                    value["coalescedCount"] = serde_json::json!(count);
                    body = value.to_string();
                }
            }
        }
    }
    core.adapters
        .lock()
        .unwrap()
        .get(parent_session_id)
        .is_some_and(|runtime| runtime.send_turn(&body).is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_KINDS: [NoticeKind; 9] = [
        NoticeKind::WorkerUnblocked,
        NoticeKind::WorkerBlockedOnApproval,
        NoticeKind::WorkerSteeredByUser,
        NoticeKind::LaunchApproved,
        NoticeKind::LaunchAwaitingApproval,
        NoticeKind::WorkerRerouted,
        NoticeKind::LaunchDeclined,
        NoticeKind::LaunchFailed,
        NoticeKind::WorkerStopped,
    ];

    fn db() -> (tempfile::TempDir, Connection) {
        let scratch = tempfile::tempdir().unwrap();
        let db = crate::store::open(&scratch.path().join("test.db")).unwrap();
        (scratch, db)
    }

    #[test]
    fn level_by_class_matrix() {
        use DelegationNotifyLevel as L;
        use NoticeClass as C;
        for class in [C::Informational, C::Actionable, C::Essential] {
            assert!(level_delivers(L::All, class));
            assert_eq!(level_delivers(L::Actionable, class), class != C::Informational);
            assert_eq!(level_delivers(L::ResultsOnly, class), class == C::Essential);
        }
    }

    #[test]
    fn classification_table() {
        assert_eq!(NoticeKind::WorkerUnblocked.class(), NoticeClass::Informational);
        assert_eq!(NoticeKind::LaunchAwaitingApproval.class(), NoticeClass::Actionable);
        for kind in [NoticeKind::LaunchFailed, NoticeKind::LaunchDeclined, NoticeKind::WorkerStopped] {
            assert_eq!(kind.class(), NoticeClass::Essential);
            assert!(level_delivers(DelegationNotifyLevel::ResultsOnly, kind.class()));
        }
        assert_eq!(ALL_KINDS.len(), 9);
    }

    #[test]
    fn global_default_is_all_and_session_override_wins() {
        let (_scratch, db) = db();
        assert_eq!(view(&db, Some("orch")).unwrap().level, DelegationNotifyLevel::All);
        save(&db, &SaveDelegationNotifySettingsParams { session_id: None, level: Some(DelegationNotifyLevel::Actionable) }).unwrap();
        assert_eq!(effective_level(&db, "orch"), DelegationNotifyLevel::Actionable);
        let overridden = save(
            &db,
            &SaveDelegationNotifySettingsParams { session_id: Some("orch".into()), level: Some(DelegationNotifyLevel::All) },
        )
        .unwrap();
        assert_eq!(overridden.level, DelegationNotifyLevel::All);
        assert_eq!(overridden.global_level, DelegationNotifyLevel::Actionable);
        assert_eq!(effective_level(&db, "other"), DelegationNotifyLevel::Actionable);
        let cleared = save(&db, &SaveDelegationNotifySettingsParams { session_id: Some("orch".into()), level: None }).unwrap();
        assert_eq!(cleared.session_override, None);
        assert_eq!(cleared.level, DelegationNotifyLevel::Actionable);
    }

    #[test]
    fn clearing_the_global_level_is_rejected() {
        let (_scratch, db) = db();
        assert!(save(&db, &SaveDelegationNotifySettingsParams { session_id: None, level: None }).is_err());
    }

    #[test]
    fn identical_notices_coalesce_inside_the_window_and_carry_a_count() {
        let t0 = Instant::now();
        let (p, c) = ("coalesce-parent", "coalesce-child");
        assert_eq!(coalesce(p, c, NoticeKind::WorkerUnblocked, "allowed", t0), Some(0));
        assert_eq!(coalesce(p, c, NoticeKind::WorkerUnblocked, "allowed", t0 + Duration::from_secs(5)), None);
        assert_eq!(coalesce(p, c, NoticeKind::WorkerUnblocked, "allowed", t0 + Duration::from_secs(9)), None);
        assert_eq!(coalesce(p, c, NoticeKind::WorkerUnblocked, "denied", t0 + Duration::from_secs(9)), Some(0));
        assert_eq!(coalesce(p, c, NoticeKind::WorkerUnblocked, "allowed", t0 + Duration::from_secs(31)), Some(2));
    }
}
