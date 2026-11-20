//! The early compaction switch: one global value and an opt-out for each chat.
//!
//! An absent row means off. A chat can only opt out, so no chat row turns the
//! feature on. Only a chat that the rule covers can opt out. A write holds the input lease, so a save and a dispatch of the
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

/// The rule covers the chat: the row exists and no scope check of the rule
/// holds for it. The host decides this, so the menu needs no harness test.
pub fn session_eligible(db: &Connection, session_id: &str) -> Result<bool, BridgeError> {
    let session = crate::early_compaction::attempts::session_row(db, session_id)?;
    Ok(session.is_some_and(|session| crate::early_compaction::rule::in_scope(&session)))
}

pub fn view(db: &Connection, session_id: Option<&str>) -> Result<EarlyCompactionSettingsView, BridgeError> {
    let global_enabled = global_enabled(db)?;
    let (session_opted_out, session_eligible) = match session_id {
        Some(session_id) => (opted_out(db, session_id)?, session_eligible(db, session_id)?),
        None => (false, false),
    };
    Ok(EarlyCompactionSettingsView {
        enabled: global_enabled && !session_opted_out,
        global_enabled,
        session_opted_out,
        session_eligible,
    })
}

/// The value that startup and each write load into memory. A read failure
/// gives off, so a damaged row never turns the feature on.
pub fn stored_switch(db: &Connection) -> bool {
    global_enabled(db).unwrap_or(false)
}

/// Runs `write` under the input lease and the database lock, then loads the
/// stored switch into memory before the lease ends. The lease comes first,
/// the same order as a send. With the switch off, each due mark goes at once.
pub fn with_switch_write<T>(
    core: &BridgeCore,
    write: impl FnOnce(&Connection) -> Result<T, BridgeError>,
) -> Result<T, BridgeError> {
    let _lease = core.input_activity.read().unwrap_or_else(PoisonError::into_inner);
    let db = core.db.lock().unwrap_or_else(PoisonError::into_inner);
    let result = write(&db);
    let enabled = stored_switch(&db);
    core.early_compaction.set_enabled(enabled);
    if !enabled {
        core.early_compaction.clear_marks();
    }
    result
}

pub fn save(core: &BridgeCore, params: &SaveEarlyCompactionSettingsParams) -> Result<EarlyCompactionSettingsView, BridgeError> {
    let change = Change::from_params(params)?;
    with_switch_write(core, |db| {
        match change {
            Change::Global(enabled) => upsert(db, GLOBAL_KIND, GLOBAL_ID, &json(&GlobalSwitch { enabled })?)?,
            Change::OptOut(session_id) => {
                // The removal of an opt-out stays open to each chat id, so an
                // old row can always go.
                if !session_eligible(db, session_id)? {
                    return Err(BridgeError::Invalid(
                        "early compaction does not cover this chat, so it cannot opt out".into(),
                    ));
                }
                upsert(db, OPT_OUT_KIND, session_id, "{}")?
            }
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

    /// One session row. The kind, the parent, the depth and the harness set
    /// whether the rule covers it.
    fn seed(core: &BridgeCore, id: &str, harness: &str, kind: &str, parent: Option<&str>, depth: i64) {
        core.db
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO sessions(id,harness,label,status,metric_source,kind,parent_session_id,depth)
                 VALUES(?1,?2,'Chat','ready','reported',?3,?4,?5)",
                params![id, harness, kind, parent, depth],
            )
            .unwrap();
    }

    fn direct(core: &BridgeCore, id: &str) {
        seed(core, id, "claude", "direct", None, 0);
    }

    /// An eligible Claude chat, an eligible Codex chat and one chat for each
    /// reason that puts a chat outside the scope.
    fn scope(core: &BridgeCore) -> [(&'static str, bool); 6] {
        direct(core, "claude-chat");
        seed(core, "codex-chat", "codex", "direct", None, 0);
        seed(core, "worker-chat", "codex", "worker", Some("claude-chat"), 1);
        seed(core, "deep-chat", "claude", "direct", None, 1);
        seed(core, "opencode-chat", "opencode", "direct", None, 0);
        [
            ("claude-chat", true),
            ("codex-chat", true),
            ("missing-chat", false),
            ("worker-chat", false),
            ("deep-chat", false),
            ("opencode-chat", false),
        ]
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
        assert_eq!(view(&db, None).unwrap(), EarlyCompactionSettingsView::default());
        assert_eq!(view(&db, Some("chat-1")).unwrap(), EarlyCompactionSettingsView::default());
        assert!(!stored_switch(&db));
        drop(db);
        assert!(!core.early_compaction.enabled());
        assert!(rows(&core).is_empty());
    }

    #[test]
    fn the_global_switch_round_trips() {
        let (_scratch, core) = core();
        let on = save(&core, &global(Some(true))).unwrap();
        assert_eq!(on, EarlyCompactionSettingsView { enabled: true, global_enabled: true, session_opted_out: false, session_eligible: false });
        assert_eq!(rows(&core), vec![(GLOBAL_KIND.into(), GLOBAL_ID.into(), r#"{"enabled":true}"#.into())]);
        assert!(view(&core.db.lock().unwrap(), Some("chat-1")).unwrap().enabled);

        let off = save(&core, &global(Some(false))).unwrap();
        assert_eq!(off, EarlyCompactionSettingsView::default());
        assert_eq!(rows(&core), vec![(GLOBAL_KIND.into(), GLOBAL_ID.into(), r#"{"enabled":false}"#.into())]);
    }

    #[test]
    fn a_chat_can_opt_out_and_back() {
        let (_scratch, core) = core();
        direct(&core, "chat-1");
        save(&core, &global(Some(true))).unwrap();

        let out = save(&core, &chat("chat-1", Some(false))).unwrap();
        assert_eq!(out, EarlyCompactionSettingsView { enabled: false, global_enabled: true, session_opted_out: true, session_eligible: true });
        assert!(rows(&core).contains(&(OPT_OUT_KIND.into(), "chat-1".into(), "{}".into())));
        let other = view(&core.db.lock().unwrap(), Some("chat-2")).unwrap();
        assert!(other.enabled, "an opt-out belongs to one chat");
        assert!(core.early_compaction.enabled(), "an opt-out leaves the global switch on");

        let back = save(&core, &chat("chat-1", None)).unwrap();
        assert_eq!(back, EarlyCompactionSettingsView { enabled: true, global_enabled: true, session_opted_out: false, session_eligible: true });
        assert_eq!(rows(&core).len(), 1, "removing the opt-out deletes its row");

        save(&core, &global(Some(false))).unwrap();
        let kept = save(&core, &chat("chat-1", Some(false))).unwrap();
        assert_eq!(kept, EarlyCompactionSettingsView { enabled: false, global_enabled: false, session_opted_out: true, session_eligible: true });
    }

    #[test]
    fn a_chat_cannot_turn_the_feature_on() {
        let (_scratch, core) = core();
        direct(&core, "chat-1");
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
        direct(&core, "chat-1");
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
        direct(&core, "chat-1");
        let core = Arc::new(core);
        save(&core, &global(Some(true))).unwrap();
        save(&core, &chat("chat-1", Some(false))).unwrap();
        assert_eq!(rows(&core).len(), 2);

        crate::api::reset_all_config(&core).unwrap();

        assert!(rows(&core).is_empty(), "a reset deletes both kinds");
        assert!(!core.early_compaction.enabled());
        assert_eq!(
            view(&core.db.lock().unwrap(), Some("chat-1")).unwrap(),
            EarlyCompactionSettingsView { enabled: false, global_enabled: false, session_opted_out: false, session_eligible: true }
        );
    }

    #[test]
    fn get_reports_the_scope_of_a_chat() {
        let (_scratch, core) = core();
        let cases = scope(&core);
        save(&core, &global(Some(true))).unwrap();
        let db = core.db.lock().unwrap();
        assert!(!view(&db, None).unwrap().session_eligible, "a request with no chat names no scope");
        for (session_id, eligible) in cases {
            let view = view(&db, Some(session_id)).unwrap();
            assert_eq!(view.session_eligible, eligible, "{session_id}");
            assert!(view.global_enabled, "{session_id}");
        }
    }

    #[test]
    fn an_opt_out_for_a_chat_outside_the_scope_is_refused() {
        let (_scratch, core) = core();
        let cases = scope(&core);
        save(&core, &global(Some(true))).unwrap();
        for (session_id, eligible) in cases {
            let result = save(&core, &chat(session_id, Some(false)));
            let stored = rows(&core).iter().any(|(kind, id, _)| kind == OPT_OUT_KIND && id == session_id);
            if eligible {
                assert!(result.unwrap().session_opted_out, "{session_id}");
                assert!(stored, "{session_id}");
            } else {
                assert!(matches!(result, Err(BridgeError::Invalid(_))), "{session_id}: {result:?}");
                assert!(!stored, "{session_id} got an opt-out row");
            }
            // The removal of an opt-out stays open to each chat id.
            let removed = save(&core, &chat(session_id, None)).unwrap();
            assert!(!removed.session_opted_out, "{session_id}");
        }
        assert_eq!(rows(&core).len(), 1, "only the global switch is left");
        assert!(core.early_compaction.enabled(), "a refused opt-out leaves the switch on");
    }

    #[test]
    fn a_save_that_turns_the_switch_off_drops_each_due_mark() {
        use crate::early_compaction::{DueMark, Receipt};
        let mark = |session_id: &str| {
            Receipt::Due(DueMark {
                session_id: session_id.into(),
                turn_id: None,
                completed_at: chrono::Utc::now(),
                launch: None,
                harness: "claude".into(),
            })
        };
        let (_scratch, core) = core();
        direct(&core, "chat-3");
        let core = Arc::new(core);
        save(&core, &global(Some(true))).unwrap();
        core.early_compaction.accept(mark("chat-1"));
        core.early_compaction.accept(mark("chat-2"));
        // A save that keeps the switch on keeps the marks.
        save(&core, &chat("chat-3", Some(false))).unwrap();
        assert_eq!(core.early_compaction.marks().len(), 2);
        // Off and on again before the next sweep: no old mark is left.
        save(&core, &global(Some(false))).unwrap();
        assert!(core.early_compaction.marks().is_empty(), "a due mark outlived the switch");
        save(&core, &global(Some(true))).unwrap();
        assert!(core.early_compaction.marks().is_empty());
        // A reset turns the switch off too.
        core.early_compaction.accept(mark("chat-1"));
        crate::api::reset_all_config(&core).unwrap();
        assert!(core.early_compaction.marks().is_empty(), "a due mark outlived the reset");
    }
}
