//! The app preferences record: theme, skin, effort picker style and transcript
//! display. The app draws these itself, but the core keeps the record of them
//! so a chat (through `settings_change`), a second window or the web client can
//! change them. Each window caches the values in localStorage only to paint its
//! first frame, and follows `config/app_preferences_changed` after that.

use bridge_protocol::messages::{AppPreferenceValues, AppPreferences};
use rusqlite::{params, Connection, OptionalExtension};

use crate::BridgeError;

const KIND: &str = "app_preferences_v1";
const ID: &str = "application";

pub fn read(db: &Connection) -> Result<AppPreferences, BridgeError> {
    let payload: Option<String> = db
        .query_row("SELECT payload FROM configuration_entries WHERE kind=?1 AND id=?2", params![KIND, ID], |row| row.get(0))
        .optional()?;
    // A record this build cannot read is treated as unset rather than failing
    // the app: every value has a built-in default.
    Ok(payload.and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default())
}

/// Merge the keys `values` sets into the record. A save that changes nothing
/// keeps the revision, so a window echoing what it was just sent is a no-op.
pub fn save(db: &Connection, values: &AppPreferenceValues) -> Result<(AppPreferences, bool), BridgeError> {
    let transaction = db.unchecked_transaction()?;
    let current = read(&transaction)?;
    let mut next = current.values.clone();
    macro_rules! merge {
        ($($field:ident),*) => { $(if values.$field.is_some() { next.$field = values.$field; })* };
    }
    merge!(theme, skin, effort_selector, show_thinking, auto_expand_edits);
    if next == current.values {
        return Ok((current, false));
    }
    let record = AppPreferences {
        revision: current.revision.checked_add(1).ok_or_else(|| BridgeError::Invalid("App preferences revision exhausted".into()))?,
        values: next,
    };
    let payload = serde_json::to_string(&record).map_err(|error| BridgeError::Invalid(error.to_string()))?;
    transaction.execute(
        "INSERT INTO configuration_entries(kind,id,payload,created_at,updated_at) VALUES(?1,?2,?3,datetime('now'),datetime('now')) ON CONFLICT(kind,id) DO UPDATE SET payload=excluded.payload,updated_at=excluded.updated_at",
        params![KIND, ID, payload],
    )?;
    transaction.commit()?;
    Ok((record, true))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bridge_protocol::messages::{ThemePreference, ThemeSkin};

    #[test]
    fn save_merges_present_keys_and_bumps_the_revision_only_on_change() {
        let db = crate::store::open(std::path::Path::new(":memory:")).unwrap();
        assert_eq!(read(&db).unwrap(), AppPreferences::default());

        let (saved, changed) = save(&db, &AppPreferenceValues { theme: Some(ThemePreference::Dark), ..Default::default() }).unwrap();
        assert!(changed);
        assert_eq!(saved.revision, 1);

        let (saved, changed) = save(&db, &AppPreferenceValues { skin: Some(ThemeSkin::Vibrancy), ..Default::default() }).unwrap();
        assert!(changed);
        assert_eq!(saved.revision, 2);
        assert_eq!(saved.values.theme, Some(ThemePreference::Dark), "an absent key keeps its value");
        assert_eq!(saved.values.skin, Some(ThemeSkin::Vibrancy));

        let (again, changed) = save(&db, &AppPreferenceValues { theme: Some(ThemePreference::Dark), ..Default::default() }).unwrap();
        assert!(!changed, "an echo of the current value is not a change");
        assert_eq!(again.revision, 2);
        assert_eq!(read(&db).unwrap(), again);
    }
}
