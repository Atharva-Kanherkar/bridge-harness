//! The early compaction switch: one global value and an opt-out for each chat.
//!
//! An absent row means off. A chat can only opt out, so no chat row turns the
//! feature on. A write holds the input lease, so a save and a dispatch of the
//! sweep never interleave. Nothing here publishes an event or touches policy.

use crate::{BridgeCore, BridgeError};
use bridge_protocol::messages::{EarlyCompactionSettingsView, SaveEarlyCompactionSettingsParams};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::sync::PoisonError;

pub const GLOBAL_KIND: &str = "early_compaction_settings";
pub const GLOBAL_ID: &str = "global";
pub const OPT_OUT_KIND: &str = "early_compaction_opt_out";

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct GlobalSwitch {
    enabled: bool,
}

enum Change<'a> {
    Global(bool),
    OptOut(&'a str),
    RemoveOptOut(&'a str),
}

impl<'a> Change<'a> {
    fn from_params(params: &'a SaveEarlyCompactionSettingsParams) -> Result<Self, BridgeError> {
        match (params.session_id.as_deref(), params.enabled) {
            (Some(session_id), Some(false)) => Ok(Self::OptOut(session_id)),
            (Some(session_id), None) => Ok(Self::RemoveOptOut(session_id)),
            (Some(_), Some(true)) => Err(BridgeError::Invalid(
                "a chat can only opt out of early compaction, the global switch turns it on".into(),
            )),
            (None, Some(enabled)) => Ok(Self::Global(enabled)),
            (None, None) => Err(BridgeError::Invalid("the global early compaction switch must be set, not cleared".into())),
        }
    }
}

pub fn global_enabled(db: &Connection) -> Result<bool, BridgeError> {
    let payload: Option<String> = db
        .query_row(
            "SELECT payload FROM configuration_entries WHERE kind=?1 AND id=?2",
            params![GLOBAL_KIND, GLOBAL_ID],
            |row| row.get(0),
        )
        .optional()?;
    payload
        .map(|payload| {
            serde_json::from_str::<GlobalSwitch>(&payload)
                .map(|switch| switch.enabled)
                .map_err(|error| BridgeError::Invalid(error.to_string()))
        })
        .unwrap_or(Ok(false))
}

pub fn opted_out(db: &Connection, session_id: &str) -> Result<bool, BridgeError> {
    let row = db
        .query_row(
            "SELECT 1 FROM configuration_entries WHERE kind=?1 AND id=?2",
            params![OPT_OUT_KIND, session_id],
            |_| Ok(()),
        )
        .optional()?;
    Ok(row.is_some())
}

pub fn view(db: &Connection, session_id: Option<&str>) -> Result<EarlyCompactionSettingsView, BridgeError> {
    let global_enabled = global_enabled(db)?;
    let session_opted_out = match session_id {
        Some(session_id) => opted_out(db, session_id)?,
        None => false,
    };
    Ok(EarlyCompactionSettingsView { enabled: global_enabled && !session_opted_out, global_enabled, session_opted_out })
}

/// The value that startup and each write load into memory. A read failure
/// gives off, so a damaged row never turns the feature on.
pub fn stored_switch(db: &Connection) -> bool {
    global_enabled(db).unwrap_or(false)
}

/// Runs `write` under the input lease and the database lock, then loads the
/// stored switch into memory before the lease ends. The lease comes first,
/// the same order as a send.
pub fn with_switch_write<T>(
    core: &BridgeCore,
    write: impl FnOnce(&Connection) -> Result<T, BridgeError>,
) -> Result<T, BridgeError> {
    let _lease = core.input_activity.read().unwrap_or_else(PoisonError::into_inner);
    let db = core.db.lock().unwrap_or_else(PoisonError::into_inner);
    let result = write(&db);
    core.early_compaction.set_enabled(stored_switch(&db));
    result
}

pub fn save(core: &BridgeCore, params: &SaveEarlyCompactionSettingsParams) -> Result<EarlyCompactionSettingsView, BridgeError> {
    let change = Change::from_params(params)?;
    with_switch_write(core, |db| {
        match change {
            Change::Global(enabled) => upsert(db, GLOBAL_KIND, GLOBAL_ID, &json(&GlobalSwitch { enabled })?)?,
            Change::OptOut(session_id) => upsert(db, OPT_OUT_KIND, session_id, "{}")?,
            Change::RemoveOptOut(session_id) => {
                db.execute(
                    "DELETE FROM configuration_entries WHERE kind=?1 AND id=?2",
                    params![OPT_OUT_KIND, session_id],
                )?;
            }
        }
        view(db, params.session_id.as_deref())
    })
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{mpsc, Arc};
    use std::time::Duration;

    fn core() -> (tempfile::TempDir, BridgeCore) {
        let scratch = tempfile::tempdir().unwrap();
        let core = BridgeCore::for_tests(scratch.path());
        (scratch, core)
    }

    fn global(enabled: Option<bool>) -> SaveEarlyCompactionSettingsParams {
        SaveEarlyCompactionSettingsParams { session_id: None, enabled }
    }

    fn chat(session_id: &str, enabled: Option<bool>) -> SaveEarlyCompactionSettingsParams {
        SaveEarlyCompactionSettingsParams { session_id: Some(session_id.into()), enabled }
    }

    fn rows(core: &BridgeCore) -> Vec<(String, String, String)> {
        let db = core.db.lock().unwrap();
        let mut statement = db
            .prepare("SELECT kind,id,payload FROM configuration_entries WHERE kind IN (?1,?2) ORDER BY kind,id")
            .unwrap();
        statement
            .query_map(params![GLOBAL_KIND, OPT_OUT_KIND], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    }

    #[test]
    fn the_switch_is_off_when_absent() {
        let (_scratch, core) = core();
        let db = core.db.lock().unwrap();
        assert_eq!(
            view(&db, None).unwrap(),
            EarlyCompactionSettingsView { enabled: false, global_enabled: false, session_opted_out: false }
        );
        assert_eq!(
            view(&db, Some("chat-1")).unwrap(),
            EarlyCompactionSettingsView { enabled: false, global_enabled: false, session_opted_out: false }
        );
        assert!(!stored_switch(&db));
        drop(db);
        assert!(!core.early_compaction.enabled());
        assert!(rows(&core).is_empty());
    }

    #[test]
    fn the_global_switch_round_trips() {
        let (_scratch, core) = core();
        let on = save(&core, &global(Some(true))).unwrap();
        assert_eq!(on, EarlyCompactionSettingsView { enabled: true, global_enabled: true, session_opted_out: false });
        assert_eq!(rows(&core), vec![(GLOBAL_KIND.into(), GLOBAL_ID.into(), r#"{"enabled":true}"#.into())]);
        assert!(view(&core.db.lock().unwrap(), Some("chat-1")).unwrap().enabled);

        let off = save(&core, &global(Some(false))).unwrap();
        assert_eq!(off, EarlyCompactionSettingsView { enabled: false, global_enabled: false, session_opted_out: false });
        assert_eq!(rows(&core), vec![(GLOBAL_KIND.into(), GLOBAL_ID.into(), r#"{"enabled":false}"#.into())]);
    }

    #[test]
    fn a_chat_can_opt_out_and_back() {
        let (_scratch, core) = core();
        save(&core, &global(Some(true))).unwrap();

        let out = save(&core, &chat("chat-1", Some(false))).unwrap();
        assert_eq!(out, EarlyCompactionSettingsView { enabled: false, global_enabled: true, session_opted_out: true });
        assert!(rows(&core).contains(&(OPT_OUT_KIND.into(), "chat-1".into(), "{}".into())));
        let other = view(&core.db.lock().unwrap(), Some("chat-2")).unwrap();
        assert!(other.enabled, "an opt-out belongs to one chat");
        assert!(core.early_compaction.enabled(), "an opt-out leaves the global switch on");

        let back = save(&core, &chat("chat-1", None)).unwrap();
        assert_eq!(back, EarlyCompactionSettingsView { enabled: true, global_enabled: true, session_opted_out: false });
        assert_eq!(rows(&core).len(), 1, "removing the opt-out deletes its row");

        save(&core, &global(Some(false))).unwrap();
        let kept = save(&core, &chat("chat-1", Some(false))).unwrap();
        assert_eq!(kept, EarlyCompactionSettingsView { enabled: false, global_enabled: false, session_opted_out: true });
    }

    #[test]
    fn a_chat_cannot_turn_the_feature_on() {
        let (_scratch, core) = core();
        let error = save(&core, &chat("chat-1", Some(true))).unwrap_err();
        assert!(matches!(error, BridgeError::Invalid(_)), "{error:?}");
        assert!(rows(&core).is_empty());
        assert!(!core.early_compaction.enabled());

        save(&core, &global(Some(true))).unwrap();
        save(&core, &chat("chat-1", Some(false))).unwrap();
        assert!(matches!(save(&core, &chat("chat-1", Some(true))), Err(BridgeError::Invalid(_))));
        assert!(view(&core.db.lock().unwrap(), Some("chat-1")).unwrap().session_opted_out, "a refused save keeps the opt-out");
    }

    #[test]
    fn a_global_save_without_a_value_is_refused() {
        let (_scratch, core) = core();
        save(&core, &global(Some(true))).unwrap();
        assert!(matches!(save(&core, &global(None)), Err(BridgeError::Invalid(_))));
        assert!(view(&core.db.lock().unwrap(), None).unwrap().global_enabled, "a refused save keeps the switch");
        assert!(core.early_compaction.enabled());
    }

    #[test]
    fn a_save_sets_the_switch_in_memory() {
        let (_scratch, core) = core();
        let core = Arc::new(core);
        assert!(!core.early_compaction.enabled());

        // The save waits while another holder has the lease exclusively.
        let exclusive = core.input_activity.write().unwrap();
        let (done_tx, done_rx) = mpsc::channel();
        let saver = {
            let core = Arc::clone(&core);
            std::thread::spawn(move || {
                let result = save(&core, &global(Some(true)));
                done_tx.send(()).unwrap();
                result
            })
        };
        assert!(done_rx.recv_timeout(Duration::from_millis(200)).is_err(), "the save waits on the input lease");
        assert!(!core.early_compaction.enabled());
        drop(exclusive);
        assert!(saver.join().unwrap().unwrap().enabled);
        assert!(core.early_compaction.enabled());

        save(&core, &chat("chat-1", Some(false))).unwrap();
        assert!(core.early_compaction.enabled(), "the memory flag holds the global switch only");
        save(&core, &global(Some(false))).unwrap();
        assert!(!core.early_compaction.enabled());
    }

    #[test]
    fn a_configuration_reset_turns_it_off() {
        let (_scratch, core) = core();
        let core = Arc::new(core);
        save(&core, &global(Some(true))).unwrap();
        save(&core, &chat("chat-1", Some(false))).unwrap();
        assert_eq!(rows(&core).len(), 2);

        crate::api::reset_all_config(&core).unwrap();

        assert!(rows(&core).is_empty(), "a reset deletes both kinds");
        assert!(!core.early_compaction.enabled());
        assert_eq!(
            view(&core.db.lock().unwrap(), Some("chat-1")).unwrap(),
            EarlyCompactionSettingsView { enabled: false, global_enabled: false, session_opted_out: false }
        );
    }
}
