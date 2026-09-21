//! Menu presentation preferences stored by the central backend.
use crate::BridgeError;
use bridge_protocol::messages::MenuBarSettings;
use rusqlite::{params, Connection, OptionalExtension};

pub fn load(db: &Connection) -> Result<MenuBarSettings, BridgeError> {
    let payload: Option<String> = db
        .query_row(
            "SELECT payload FROM configuration_entries WHERE kind='menu_bar' AND id='default'",
            [],
            |r| r.get(0),
        )
        .optional()?;
    let settings = payload
        .map(|p| {
            serde_json::from_str(&p)
                .map_err(|e| BridgeError::Invalid(format!("Invalid Menu Bar settings: {e}")))
        })
        .transpose()?
        .unwrap_or_default();
    validate(&settings)?;
    Ok(settings)
}

fn validate(settings: &MenuBarSettings) -> Result<(), BridgeError> {
    if settings.pinned_providers.len() > bridge_protocol::messages::MenuBarProvider::ALL.len()
        || settings
            .pinned_providers
            .iter()
            .enumerate()
            .any(|(index, provider)| settings.pinned_providers[..index].contains(provider))
    {
        return Err(BridgeError::Invalid(
            "Choose different favorite providers from the supported providers".into(),
        ));
    }
    if settings.status_layout.len() > 2
        || settings.status_layout.iter().any(|line| line.len() > 12)
        || settings
            .status_layout
            .iter()
            .flatten()
            .filter(|token| matches!(token, bridge_protocol::messages::MenuBarLayoutToken::Icon))
            .count()
            > 1
    {
        return Err(BridgeError::Invalid(
            "Menu layout supports two lines, twelve items per line, and one icon".into(),
        ));
    }
    if settings.schema_version != 1 || ![0, 60, 300, 900, 1800].contains(&settings.refresh_seconds)
    {
        return Err(BridgeError::Invalid(
            "Unsupported Menu Bar settings version or refresh interval".into(),
        ));
    }
    if settings
        .opencode_workspace
        .as_deref()
        .is_some_and(|v| !crate::provider_usage::credentials::valid_workspace(v))
    {
        return Err(BridgeError::Invalid(
            "OpenCode workspace must be a wrk_ workspace ID".into(),
        ));
    }
    Ok(())
}

pub fn save(db: &Connection, settings: &MenuBarSettings) -> Result<MenuBarSettings, BridgeError> {
    validate(settings)?;
    let previous = load(db)?;
    let transaction = db.unchecked_transaction()?;
    if previous.opencode_workspace != settings.opencode_workspace {
        // Never present the previous workspace's quota under a new selection.
        transaction.execute(
            "DELETE FROM configuration_entries WHERE kind='usage_overview' AND id='opencode'",
            [],
        )?;
    }
    transaction.execute("INSERT INTO configuration_entries(kind,id,payload,created_at,updated_at) VALUES('menu_bar','default',?1,?2,?2)
        ON CONFLICT(kind,id) DO UPDATE SET payload=excluded.payload,updated_at=excluded.updated_at",
        params![serde_json::to_string(settings).map_err(|e| BridgeError::Invalid(e.to_string()))?, chrono::Utc::now().to_rfc3339()])?;
    transaction.commit()?;
    load(db)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn favorites_preserve_order_without_enabling_collectors() {
        use bridge_protocol::messages::MenuBarProvider::{Claude, Codex, Cursor, OpenCode};
        let temp = tempfile::tempdir().unwrap();
        let db = crate::store::open(&temp.path().join("favorites.db")).unwrap();
        let mut settings = load(&db).unwrap();
        assert_eq!(settings.pinned_providers, vec![Codex, Claude]);
        assert!(!settings.cursor_enabled && !settings.claude_enabled);
        settings.pinned_providers = vec![Cursor, OpenCode, Claude];
        let saved = save(&db, &settings).unwrap();
        assert_eq!(saved.pinned_providers, vec![Cursor, OpenCode, Claude]);
        assert!(!saved.cursor_enabled && !saved.opencode_enabled);
        settings.pinned_providers = vec![Cursor, Cursor];
        assert!(save(&db, &settings).is_err());
        assert_eq!(load(&db).unwrap(), saved);
        settings.pinned_providers = vec![Codex, Claude, Cursor, OpenCode];
        assert_eq!(save(&db, &settings).unwrap().pinned_providers.len(), 4);
        settings.pinned_providers.clear();
        assert!(save(&db, &settings).unwrap().pinned_providers.is_empty());
    }

    #[test]
    fn persists_settings_and_rejects_invalid_changes_without_overwriting() {
        let temp = tempfile::tempdir().unwrap();
        let db = crate::store::open(&temp.path().join("test.db")).unwrap();
        let mut settings = load(&db).unwrap();
        assert!(settings.show_overview_summary);
        assert!(!settings.separate_provider_icons);
        settings.show_overview_summary = false;
        settings.separate_provider_icons = true;
        assert_eq!(save(&db, &settings).unwrap(), settings);
        assert_eq!(load(&db).unwrap(), settings);
        settings.enabled = false;
        save(&db, &settings).unwrap();
        settings.opencode_workspace = Some("wrk_../wrong".into());
        assert!(save(&db, &settings).is_err());
        settings.opencode_workspace = Some("wrk_example".into());
        assert_eq!(
            save(&db, &settings).unwrap().opencode_workspace.as_deref(),
            Some("wrk_example")
        );
        settings.refresh_seconds = 2;
        assert!(save(&db, &settings).is_err());
        assert!(!load(&db).unwrap().enabled);
        assert_eq!(load(&db).unwrap().refresh_seconds, 300);
    }

    #[test]
    fn persists_layout_and_rejects_overflow_without_overwriting() {
        use bridge_protocol::messages::{MenuBarLayoutToken as Token, MenuBarQuotaDisplayMode};
        let temp = tempfile::tempdir().unwrap();
        let db = crate::store::open(&temp.path().join("layout.db")).unwrap();
        let mut settings = load(&db).unwrap();
        settings.status_layout = vec![vec![Token::Icon, Token::Used], vec![Token::WeeklyUsed]];
        settings.quota_display_mode = MenuBarQuotaDisplayMode::Remaining;
        assert_eq!(save(&db, &settings).unwrap(), settings);
        let saved = settings.clone();
        settings.status_layout.push(vec![Token::Space]);
        assert!(save(&db, &settings).is_err());
        settings.status_layout = vec![vec![Token::Space; 13]];
        assert!(save(&db, &settings).is_err());
        settings.status_layout = vec![vec![Token::Icon], vec![Token::Icon]];
        assert!(save(&db, &settings).is_err());
        assert_eq!(load(&db).unwrap(), saved);
    }
}
