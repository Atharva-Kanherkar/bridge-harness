//! The inbox's own preferences.
//!
//! Deliberately not a corner of `WorkSettings`. Work's settings describe a
//! briefing run — which model, which cadence, which connectors it may read — and
//! the inbox is a different surface with a different cadence that happens to talk
//! to the same connectors. Filing one surface's preference under another's
//! configuration is how a setting ends up changing something nobody expected.
//!
//! Every read here fails soft. An absent row, an unreadable row, and a payload
//! this version cannot parse all mean the same thing operationally — the user has
//! not turned this on — and none of them is worth withholding an inbox over.

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::BridgeError;

/// Where the inbox's configuration lives in `configuration_entries`.
const SETTINGS_KIND: &str = "connectors";
const SETTINGS_ID: &str = "settings";

/// What the user has chosen about how their inbox is read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ConnectorSettings {
    /// Let ingress see items the provider already considers read.
    ///
    /// Off, the inbox asks only for what is unread, which is the right question
    /// for a notification surface and an unanswerable one for anybody who reads
    /// Slack in Slack. On, a mention already opened still arrives once, which is
    /// the only signal a user can produce on demand to check the pipe works.
    pub include_read_mentions: bool,
}

/// Read the stored preferences, falling back to the defaults.
///
/// `serde(default)` on the struct plus this fallback means a payload written by
/// a newer build, an older build, or a corrupted write all degrade to "off"
/// rather than to an error the inbox would have to render.
pub fn read(db: &Connection) -> ConnectorSettings {
    let payload: Option<String> = db
        .query_row(
            "SELECT payload FROM configuration_entries WHERE kind=?1 AND id=?2",
            rusqlite::params![SETTINGS_KIND, SETTINGS_ID],
            |row| row.get(0),
        )
        .optional()
        .ok()
        .flatten();
    payload
        .and_then(|payload| serde_json::from_str(&payload).ok())
        .unwrap_or_default()
}

/// Persist the preferences. An upsert, so a rewrite replaces rather than
/// accumulating rows that later disagree.
pub fn write(db: &Connection, settings: ConnectorSettings) -> Result<ConnectorSettings, BridgeError> {
    let payload = serde_json::to_string(&settings)
        .map_err(|error| BridgeError::Invalid(format!("inbox settings are not storable: {error}")))?;
    let now = chrono::Utc::now().to_rfc3339();
    db.execute(
        "INSERT INTO configuration_entries(kind,id,payload,created_at,updated_at)
         VALUES(?1,?2,?3,?4,?4)
         ON CONFLICT(kind,id) DO UPDATE SET payload=excluded.payload,updated_at=excluded.updated_at",
        rusqlite::params![SETTINGS_KIND, SETTINGS_ID, payload, now],
    )?;
    Ok(settings)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Connection {
        crate::store::open(std::path::Path::new(":memory:")).unwrap()
    }

    #[test]
    fn settings_default_to_off_when_nothing_was_ever_written() {
        assert_eq!(read(&db()), ConnectorSettings { include_read_mentions: false });
    }

    #[test]
    fn settings_round_trip_and_a_rewrite_is_an_upsert() {
        let db = db();
        write(&db, ConnectorSettings { include_read_mentions: true }).unwrap();
        assert!(read(&db).include_read_mentions);

        write(&db, ConnectorSettings { include_read_mentions: false }).unwrap();
        assert!(!read(&db).include_read_mentions);

        let rows: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM configuration_entries WHERE kind=?1 AND id=?2",
                rusqlite::params![SETTINGS_KIND, SETTINGS_ID],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(rows, 1, "a rewrite replaces rather than accumulating");
    }

    #[test]
    fn a_payload_this_version_cannot_read_is_off_rather_than_an_error() {
        // A row written by a future build, or a half-written one. The inbox is
        // useful without this preference, so it must not be the thing that
        // stops the pane rendering.
        let db = db();
        let now = chrono::Utc::now().to_rfc3339();
        db.execute(
            "INSERT INTO configuration_entries(kind,id,payload,created_at,updated_at)
             VALUES(?1,?2,'{not json',?3,?3)",
            rusqlite::params![SETTINGS_KIND, SETTINGS_ID, now],
        )
        .unwrap();

        assert!(!read(&db).include_read_mentions);
    }

    #[test]
    fn an_unknown_field_does_not_discard_the_fields_this_version_knows() {
        // No `deny_unknown_fields` here on purpose: a newer build adding a
        // second preference must not cost this one its value on downgrade.
        let db = db();
        let now = chrono::Utc::now().to_rfc3339();
        db.execute(
            "INSERT INTO configuration_entries(kind,id,payload,created_at,updated_at)
             VALUES(?1,?2,'{\"includeReadMentions\":true,\"somethingNewer\":3}',?3,?3)",
            rusqlite::params![SETTINGS_KIND, SETTINGS_ID, now],
        )
        .unwrap();

        assert!(read(&db).include_read_mentions);
    }
}
