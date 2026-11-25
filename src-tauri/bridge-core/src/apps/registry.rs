//! Per-app state on this Mac: whether each app is on, and the sidebar pins.
//!
//! Built-in manifests are code, so the tables hold departures only: a row in
//! `apps` exists once an app was turned off (or back on), and a missing row
//! means on. An id is known when it is a built-in or a locked personal app
//! (the two never share an id). Rows whose id is unknown (an app a newer
//! build added, one an older build removed, or a deleted personal app) are
//! ignored on read.

use std::collections::BTreeSet;

use bridge_protocol::messages::{AppSlot, MAX_APP_PINS};
use rusqlite::{params, Connection, Transaction};

use super::builtin::{builtin, BuiltinApp};
use crate::BridgeError;

pub fn install_store(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS apps(
            id TEXT PRIMARY KEY,
            source TEXT NOT NULL,
            enabled INTEGER NOT NULL DEFAULT 1,
            updated_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS app_pins(
            app_id TEXT PRIMARY KEY,
            position INTEGER NOT NULL UNIQUE,
            pinned_at TEXT NOT NULL
        );",
    )?;
    Ok(())
}

/// An app this Mac knows by id.
enum Known {
    Builtin(&'static BuiltinApp),
    /// A locked personal app. Its screen is the app surface, and it can be
    /// turned off.
    Personal { name: String },
}

impl Known {
    fn name(&self) -> &str {
        match self {
            Self::Builtin(app) => &app.manifest.name,
            Self::Personal { name } => name,
        }
    }
}

fn personal_exists(db: &Connection, id: &str) -> Result<bool, BridgeError> {
    Ok(super::personal::generation(db, id)?.is_some())
}

fn known(db: &Connection, id: &str) -> Result<Known, BridgeError> {
    if let Some(app) = builtin(id) {
        return Ok(Known::Builtin(app));
    }
    if personal_exists(db, id)? {
        if let Some(version) = super::personal::latest_locked(db, id)? {
            return Ok(Known::Personal { name: version.manifest.name });
        }
    }
    Err(BridgeError::Invalid(format!("Bridge has no app called {id}.")))
}

fn is_known(db: &Connection, id: &str) -> bool {
    builtin(id).is_some() || personal_exists(db, id).unwrap_or(false)
}

/// Ids of known apps that are off.
pub fn disabled_ids(db: &Connection) -> Result<BTreeSet<String>, BridgeError> {
    let mut statement = db.prepare("SELECT id FROM apps WHERE enabled=0")?;
    let ids = statement.query_map([], |row| row.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?;
    Ok(ids.into_iter().filter(|id| is_known(db, id)).collect())
}

pub fn is_enabled(db: &Connection, id: &str) -> Result<bool, BridgeError> {
    let off: Option<i64> = db
        .query_row("SELECT 1 FROM apps WHERE id=?1 AND enabled=0", [id], |row| row.get(0))
        .map(Some)
        .or_else(|error| if error == rusqlite::Error::QueryReturnedNoRows { Ok(None) } else { Err(error) })?;
    Ok(off.is_none())
}

/// Pinned known apps, in sidebar order.
pub fn pins(db: &Connection) -> Result<Vec<String>, BridgeError> {
    let mut statement = db.prepare("SELECT app_id FROM app_pins ORDER BY position")?;
    let ids = statement.query_map([], |row| row.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?;
    Ok(ids.into_iter().filter(|id| is_known(db, id)).collect())
}

/// A deleted personal app's on/off row and pin go with it, in the caller's
/// transaction, so an app later made with the same id starts on and unpinned.
pub fn forget(transaction: &Transaction<'_>, id: &str) -> Result<(), BridgeError> {
    transaction.execute("DELETE FROM apps WHERE id=?1", [id])?;
    if transaction.execute("DELETE FROM app_pins WHERE app_id=?1", [id])? > 0 {
        compact(transaction)?;
    }
    Ok(())
}

/// Rewrite pin positions as 0..n in their current order, so unpinning never
/// leaves a gap.
fn compact(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    let mut statement = transaction.prepare("SELECT app_id FROM app_pins ORDER BY position")?;
    let ids = statement.query_map([], |row| row.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?;
    drop(statement);
    // Two passes, because positions are UNIQUE: move every row out of the
    // way first, then into place.
    transaction.execute("UPDATE app_pins SET position = -1 - position", [])?;
    for (index, id) in ids.iter().enumerate() {
        transaction.execute("UPDATE app_pins SET position=?2 WHERE app_id=?1", params![id, index as i64])?;
    }
    Ok(())
}

/// Pin or unpin an app. Returns whether anything changed. A 4th pin and a pin
/// on an app that is off are refused.
pub fn set_pinned(db: &Connection, id: &str, pinned: bool, now: &str) -> Result<bool, BridgeError> {
    let transaction = db.unchecked_transaction()?;
    let app = known(&transaction, id)?;
    let current = pins(&transaction)?;
    let changed = if pinned {
        if current.iter().any(|pin| pin == id) {
            false
        } else {
            // A pin is a sidebar row that opens the app. An app that only
            // shows up as a chat card has nothing to open, so its pin would
            // hold a slot no client can draw or undo.
            if let Known::Builtin(builtin) = &app {
                if !builtin.manifest.views.iter().any(|view| matches!(view.slot, AppSlot::Screen | AppSlot::Dock)) {
                    return Err(BridgeError::Invalid(format!("{} has no screen to pin to the sidebar.", app.name())));
                }
            }
            if !is_enabled(&transaction, id)? {
                return Err(BridgeError::Invalid(format!("Turn {} on before pinning it.", app.name())));
            }
            if current.len() >= MAX_APP_PINS {
                return Err(BridgeError::Invalid(format!("You can pin {MAX_APP_PINS} apps. Unpin one first.")));
            }
            // Unknown leftovers would hold a position; clear them first.
            transaction.execute("DELETE FROM app_pins WHERE app_id NOT IN (SELECT value FROM json_each(?1))", [serde_json::to_string(&current).unwrap_or_default()])?;
            compact(&transaction)?;
            transaction.execute(
                "INSERT INTO app_pins(app_id, position, pinned_at) VALUES(?1, ?2, ?3)",
                params![id, current.len() as i64, now],
            )?;
            true
        }
    } else {
        let removed = transaction.execute("DELETE FROM app_pins WHERE app_id=?1", [id])? > 0;
        if removed {
            compact(&transaction)?;
        }
        removed
    };
    transaction.commit()?;
    Ok(changed)
}

/// Turn an app on or off. Turning it off also unpins it, in the same
/// transaction; for a personal app it also revokes every write waiting on it
/// and closes its Surface. Returns whether anything changed.
pub fn set_enabled(db: &Connection, id: &str, enabled: bool, now: &str) -> Result<bool, BridgeError> {
    let transaction = db.unchecked_transaction()?;
    let app = known(&transaction, id)?;
    let (source, can_disable) = match &app {
        Known::Builtin(builtin) => ("builtin", builtin.can_disable),
        Known::Personal { .. } => ("personal", true),
    };
    if !enabled && !can_disable {
        return Err(BridgeError::Invalid(format!("The {} cannot be turned off.", app.name())));
    }
    let was = is_enabled(&transaction, id)?;
    let mut changed = was != enabled;
    if enabled {
        // Back to the default: the departure row goes away.
        transaction.execute("DELETE FROM apps WHERE id=?1", [id])?;
    } else {
        transaction.execute(
            "INSERT INTO apps(id, source, enabled, updated_at) VALUES(?1, ?2, 0, ?3)
             ON CONFLICT(id) DO UPDATE SET enabled=0, updated_at=excluded.updated_at",
            params![id, source, now],
        )?;
        if source == "personal" {
            super::open::revoke_app(&transaction, id)?;
        }
        if transaction.execute("DELETE FROM app_pins WHERE app_id=?1", [id])? > 0 {
            compact(&transaction)?;
            changed = true;
        }
    }
    transaction.commit()?;
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn database() -> Connection {
        crate::store::open(Path::new(":memory:")).unwrap()
    }

    fn state(db: &Connection) -> (Vec<String>, BTreeSet<String>) {
        (pins(db).unwrap(), disabled_ids(db).unwrap())
    }

    #[test]
    fn fresh_db_lists_thirteen_apps_on_and_unpinned() {
        let db = database();
        assert_eq!(state(&db), (Vec::new(), BTreeSet::new()));
        for app in super::super::builtins() {
            assert!(is_enabled(&db, &app.manifest.id).unwrap());
        }
    }

    #[test]
    fn fourth_pin_is_refused() {
        let db = database();
        for id in ["notes", "pulls", "usage"] {
            assert!(set_pinned(&db, id, true, "t").unwrap());
        }
        let error = set_pinned(&db, "memory", true, "t").unwrap_err();
        assert_eq!(error.to_string(), BridgeError::Invalid("You can pin 3 apps. Unpin one first.".into()).to_string());
        assert_eq!(pins(&db).unwrap(), ["notes", "pulls", "usage"]);
        assert!(!set_pinned(&db, "pulls", true, "t").unwrap(), "pinning a pinned app is a no-op");
        assert_eq!(pins(&db).unwrap(), ["notes", "pulls", "usage"]);
    }

    #[test]
    fn unpin_closes_the_gap_and_keeps_order() {
        let db = database();
        for id in ["notes", "pulls", "usage"] {
            set_pinned(&db, id, true, "t").unwrap();
        }
        assert!(set_pinned(&db, "pulls", false, "t").unwrap());
        assert!(!set_pinned(&db, "pulls", false, "t").unwrap());
        assert_eq!(pins(&db).unwrap(), ["notes", "usage"]);
        let positions: Vec<i64> = db.prepare("SELECT position FROM app_pins ORDER BY position").unwrap().query_map([], |row| row.get(0)).unwrap().map(Result::unwrap).collect();
        assert_eq!(positions, [0, 1]);
        set_pinned(&db, "memory", true, "t").unwrap();
        assert_eq!(pins(&db).unwrap(), ["notes", "usage", "memory"]);
    }

    #[test]
    fn pins_survive_reopening_the_database() {
        let scratch = tempfile::tempdir().unwrap();
        let path = scratch.path().join("bridge.db");
        {
            let db = crate::store::open(&path).unwrap();
            set_pinned(&db, "usage", true, "t").unwrap();
            set_pinned(&db, "notes", true, "t").unwrap();
            set_enabled(&db, "trails", false, "t").unwrap();
        }
        let db = crate::store::open(&path).unwrap();
        assert_eq!(pins(&db).unwrap(), ["usage", "notes"]);
        assert!(!is_enabled(&db, "trails").unwrap());
    }

    #[test]
    fn pinning_an_off_app_is_refused() {
        let db = database();
        set_enabled(&db, "notes", false, "t").unwrap();
        let error = set_pinned(&db, "notes", true, "t").unwrap_err();
        assert!(error.to_string().contains("Turn Notes on before pinning it."), "{error}");
        assert!(pins(&db).unwrap().is_empty());
    }

    #[test]
    fn an_app_without_a_screen_cannot_be_pinned() {
        let db = database();
        let error = set_pinned(&db, "outgoing", true, "t").unwrap_err();
        assert!(error.to_string().contains("Outgoing has no screen to pin to the sidebar."), "{error}");
        assert!(pins(&db).unwrap().is_empty(), "no hidden pin holds a slot");
        assert!(!set_pinned(&db, "outgoing", false, "t").unwrap(), "unpinning it is a no-op");
    }

    #[test]
    fn turning_off_unpins() {
        let db = database();
        set_pinned(&db, "notes", true, "t").unwrap();
        set_pinned(&db, "pulls", true, "t").unwrap();
        assert!(set_enabled(&db, "notes", false, "t").unwrap());
        assert_eq!(pins(&db).unwrap(), ["pulls"]);
        assert!(!set_enabled(&db, "notes", false, "t").unwrap(), "already off");
        assert!(set_enabled(&db, "notes", true, "t").unwrap());
        assert!(is_enabled(&db, "notes").unwrap());
        assert_eq!(pins(&db).unwrap(), ["pulls"], "turning on does not pin again");
    }

    #[test]
    fn store_cannot_be_turned_off() {
        let db = database();
        let error = set_enabled(&db, "store", false, "t").unwrap_err();
        assert!(error.to_string().contains("The Store cannot be turned off."), "{error}");
        assert!(is_enabled(&db, "store").unwrap());
        assert!(!set_enabled(&db, "store", true, "t").unwrap());
    }

    #[test]
    fn unknown_ids_are_refused_and_stored_unknowns_ignored() {
        let db = database();
        assert!(set_pinned(&db, "linear", true, "t").unwrap_err().to_string().contains("Bridge has no app called linear."));
        assert!(set_enabled(&db, "linear", false, "t").is_err());
        db.execute("INSERT INTO apps(id, source, enabled, updated_at) VALUES('linear','community',0,'t')", []).unwrap();
        db.execute("INSERT INTO app_pins(app_id, position, pinned_at) VALUES('linear',0,'t')", []).unwrap();
        assert_eq!(state(&db), (Vec::new(), BTreeSet::new()));
        for id in ["notes", "pulls", "usage"] {
            set_pinned(&db, id, true, "t").unwrap();
        }
        assert_eq!(pins(&db).unwrap(), ["notes", "pulls", "usage"], "a stale row never holds a pin slot");
    }

    #[test]
    fn only_departures_are_stored() {
        let db = database();
        let rows = |db: &Connection| db.query_row("SELECT COUNT(*) FROM apps", [], |row| row.get::<_, i64>(0)).unwrap();
        assert_eq!(rows(&db), 0);
        set_enabled(&db, "usage", true, "t").unwrap();
        assert_eq!(rows(&db), 0, "on is the default");
        set_enabled(&db, "usage", false, "t").unwrap();
        assert_eq!(rows(&db), 1);
        set_enabled(&db, "usage", true, "t").unwrap();
        assert_eq!(rows(&db), 0, "back to the default leaves nothing behind");
    }
}
