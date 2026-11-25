//! The one harness and model every memory job runs on: extraction,
//! consolidation, and the automatic approval that happens inside an extraction
//! run.
//!
//! Resolution tries, in order, the user's pin, the application new-chat
//! default, and what a new chat starts on when that default is unset. The
//! first choice whose harness can hold briefing authority wins, because every
//! memory turn is tool-free and only such an adapter can enforce that. Every
//! choice passed over is reported with the adapter's own reason, so the Memory
//! page can say why it is not running on what the user expected.
//!
//! The chat a job reads never decides what it runs on. That is what lets a
//! Codex or OpenCode conversation feed memory at all.

use crate::{adapters::AdapterRegistry, BridgeError};
use bridge_protocol::messages::{
    MemoryModelChoice, MemoryModelSettings, MemoryModelSource, ResolvedMemoryModel,
    SkippedMemoryModel, ACCOUNT_MEMORY_SCOPE,
};
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension, Transaction};

const MAX_HARNESS_CHARS: usize = 128;
const MAX_MODEL_CHARS: usize = 512;

/// Creates the table and seeds the pin from the legacy per-job pins: the
/// extraction pin if both halves were set, else the consolidation pin. The seed
/// only fills an absent row, and clearing the pin stores a row of nulls, so a
/// cleared pin is never brought back by a later start.
pub(crate) fn install(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS memory_model_settings (
            scope_key TEXT PRIMARY KEY,
            harness TEXT,
            model TEXT,
            updated_at TEXT NOT NULL
        );",
    )?;
    let legacy = |table: &str| -> Result<Option<(String, String)>, BridgeError> {
        let sql = format!(
            "SELECT harness, model FROM {table}
             WHERE scope_key=?1 AND trim(coalesce(harness,''))<>'' AND trim(coalesce(model,''))<>''"
        );
        Ok(transaction
            .query_row(&sql, params![ACCOUNT_MEMORY_SCOPE], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .optional()?)
    };
    let seed = match legacy("memory_extraction_settings")? {
        Some(pin) => Some(pin),
        None => legacy("memory_consolidation_settings")?,
    };
    if let Some((harness, model)) = seed {
        transaction.execute(
            "INSERT OR IGNORE INTO memory_model_settings(scope_key, harness, model, updated_at)
             VALUES(?1,?2,?3,?4)",
            params![ACCOUNT_MEMORY_SCOPE, harness.trim(), model.trim(), Utc::now().to_rfc3339()],
        )?;
    }
    Ok(())
}

/// The user's pin, if both halves are set.
pub fn pinned(db: &Connection) -> Result<Option<MemoryModelChoice>, BridgeError> {
    let row: Option<(Option<String>, Option<String>)> = db
        .query_row(
            "SELECT harness, model FROM memory_model_settings WHERE scope_key=?1",
            params![ACCOUNT_MEMORY_SCOPE],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    Ok(match row {
        Some((Some(harness), Some(model))) => Some(MemoryModelChoice { harness, model }),
        _ => None,
    })
}

/// Pins both a harness and a model, or clears the pin when given neither.
///
/// A harness that cannot run a tool-free turn is refused here, while the user
/// is looking at the setting, rather than at every run.
pub fn set(db: &Connection, harness: Option<&str>, model: Option<&str>) -> Result<(), BridgeError> {
    let harness = harness.map(str::trim).filter(|value| !value.is_empty());
    let model = model.map(str::trim).filter(|value| !value.is_empty());
    if harness.is_some() != model.is_some() {
        return Err(BridgeError::Invalid(
            "Pin both a harness and a model, or neither to use your new-chat model.".into(),
        ));
    }
    for (value, max, name) in [(harness, MAX_HARNESS_CHARS, "harness"), (model, MAX_MODEL_CHARS, "model id")] {
        if value.is_some_and(|text| text.len() > max || text.chars().any(char::is_control)) {
            return Err(BridgeError::Invalid(format!("Invalid {name}")));
        }
    }
    if let Some(harness) = harness {
        if let Err(unsupported) = crate::briefing_policy::adapter_may_brief(harness) {
            return Err(BridgeError::Invalid(format!(
                "{harness} cannot run memory jobs, which must be tool-free: {}",
                unsupported.reason()
            )));
        }
    }
    db.execute(
        "INSERT INTO memory_model_settings(scope_key, harness, model, updated_at)
         VALUES(?1,?2,?3,?4)
         ON CONFLICT(scope_key) DO UPDATE SET harness=?2, model=?3, updated_at=?4",
        params![ACCOUNT_MEMORY_SCOPE, harness, model, Utc::now().to_rfc3339()],
    )?;
    Ok(())
}

/// The memory model, with the standard choice taken from the same code path a
/// new chat uses when its default model is unset.
pub fn resolve(db: &Connection, registry: &AdapterRegistry) -> Result<MemoryModelSettings, BridgeError> {
    resolve_with(db, || {
        crate::sessions::resolve_orchestrator_selection(db, registry)
            .map(|selection| (selection.adapter_id, selection.model))
            .map_err(|error| error.to_string())
    })
}

/// [`resolve`] with the standard choice supplied by the caller, which is only
/// asked for when neither the pin nor the new-chat default can serve.
pub fn resolve_with(
    db: &Connection,
    standard: impl FnOnce() -> Result<(String, Option<String>), String>,
) -> Result<MemoryModelSettings, BridgeError> {
    let pinned = pinned(db)?;
    let mut skipped = Vec::new();
    let mut consider = |harness: String, model: Option<String>, source: MemoryModelSource| {
        let model = model.map(|value| value.trim().to_owned()).filter(|value| !value.is_empty());
        match crate::briefing_policy::adapter_may_brief(&harness) {
            Ok(_) => Some(ResolvedMemoryModel { harness, model, source }),
            Err(unsupported) => {
                skipped.push(SkippedMemoryModel {
                    harness,
                    model,
                    source,
                    reason: unsupported.reason(),
                });
                None
            }
        }
    };
    let mut resolved = pinned
        .as_ref()
        .and_then(|pin| consider(pin.harness.clone(), Some(pin.model.clone()), MemoryModelSource::User));
    if resolved.is_none() {
        if let Some(choice) = crate::chat_defaults::application_model(db)? {
            resolved = consider(choice.harness, choice.model, MemoryModelSource::ChatDefault);
        }
    }
    if resolved.is_none() {
        // A host with no adapter that can resolve a standard model has no
        // standard choice to skip; the resolution is simply empty.
        if let Ok((harness, model)) = standard() {
            resolved = consider(harness, model, MemoryModelSource::Standard);
        }
    }
    Ok(MemoryModelSettings { pinned, resolved, skipped })
}

/// The harness and model a memory job should start, or `None` when nothing
/// can run one.
pub fn profile(settings: &MemoryModelSettings) -> Option<(String, Option<String>)> {
    settings
        .resolved
        .as_ref()
        .map(|resolved| (resolved.harness.clone(), resolved.model.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store;
    use bridge_protocol::messages::{ChatDefaults, ChatDefaultsRecord, ChatModelChoice};

    fn db() -> (tempfile::TempDir, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let db = store::open(&dir.path().join("bridge.db")).unwrap();
        (dir, db)
    }

    pub(crate) fn set_new_chat_model(db: &Connection, harness: &str, model: Option<&str>) {
        let record = ChatDefaultsRecord {
            revision: 1,
            defaults: ChatDefaults {
                model: Some(ChatModelChoice {
                    harness: harness.into(),
                    model: model.map(Into::into),
                    account: Some("work".into()),
                }),
                ..ChatDefaults::default()
            },
        };
        crate::chat_defaults::write(db, "chat_defaults_v1", "application", &record).unwrap();
    }

    fn no_standard() -> Result<(String, Option<String>), String> {
        Err("no adapter".into())
    }

    #[test]
    fn the_pin_wins_over_the_new_chat_default() {
        let (_dir, db) = db();
        set_new_chat_model(&db, "claude", Some("claude-opus-5-5"));
        set(&db, Some("claude"), Some("claude-haiku-5-5")).unwrap();
        let settings = resolve_with(&db, || panic!("the standard choice is not needed")).unwrap();
        let resolved = settings.resolved.unwrap();
        assert_eq!(resolved.source, MemoryModelSource::User);
        assert_eq!(resolved.model.as_deref(), Some("claude-haiku-5-5"));
        assert!(settings.skipped.is_empty());
    }

    #[test]
    fn no_pin_uses_the_new_chat_default_and_ignores_its_account() {
        let (_dir, db) = db();
        set_new_chat_model(&db, "claude", Some("claude-opus-5-5"));
        let settings = resolve_with(&db, no_standard).unwrap();
        assert!(settings.pinned.is_none());
        assert_eq!(
            settings.resolved,
            Some(ResolvedMemoryModel {
                harness: "claude".into(),
                model: Some("claude-opus-5-5".into()),
                source: MemoryModelSource::ChatDefault,
            })
        );
    }

    #[test]
    fn a_default_that_cannot_run_tool_free_is_skipped_for_the_standard_choice() {
        let (_dir, db) = db();
        set_new_chat_model(&db, "codex", Some("gpt-5.5"));
        let settings = resolve_with(&db, || Ok(("claude".into(), Some("claude-sonnet-5-5".into())))).unwrap();
        let resolved = settings.resolved.unwrap();
        assert_eq!(resolved.source, MemoryModelSource::Standard);
        assert_eq!(resolved.harness, "claude");
        assert_eq!(settings.skipped.len(), 1);
        let skip = &settings.skipped[0];
        assert_eq!((skip.harness.as_str(), skip.source), ("codex", MemoryModelSource::ChatDefault));
        assert!(skip.reason.contains("per-tool authority"), "{}", skip.reason);
    }

    #[test]
    fn nothing_eligible_resolves_to_none_with_every_skip_listed() {
        let (_dir, db) = db();
        // A legacy pin is seeded unchecked, so a stored pin can be one that
        // cannot brief; resolution must skip it rather than trust it.
        db.execute(
            "INSERT INTO memory_model_settings(scope_key, harness, model, updated_at)
             VALUES(?1,'opencode','big-model','now')",
            params![ACCOUNT_MEMORY_SCOPE],
        )
        .unwrap();
        set_new_chat_model(&db, "codex", None);
        let settings = resolve_with(&db, || Ok(("codex".into(), Some("gpt-5.5".into())))).unwrap();
        assert!(settings.resolved.is_none());
        assert!(profile(&settings).is_none());
        let sources: Vec<_> = settings.skipped.iter().map(|skip| skip.source).collect();
        assert_eq!(
            sources,
            [MemoryModelSource::User, MemoryModelSource::ChatDefault, MemoryModelSource::Standard]
        );
        assert!(settings.skipped.iter().all(|skip| !skip.reason.is_empty()));
    }

    #[test]
    fn set_takes_both_halves_or_neither_and_refuses_a_harness_that_cannot_brief() {
        let (_dir, db) = db();
        set(&db, Some("claude"), Some("claude-haiku-5-5")).unwrap();
        let half = set(&db, Some("claude"), None).unwrap_err().to_string();
        assert!(half.contains("both"), "{half}");
        let codex = set(&db, Some("codex"), Some("gpt-5.5")).unwrap_err().to_string();
        assert!(codex.contains("tool-free"), "{codex}");
        assert_eq!(
            pinned(&db).unwrap(),
            Some(MemoryModelChoice { harness: "claude".into(), model: "claude-haiku-5-5".into() }),
            "a refused write leaves the pin as it was"
        );
        set(&db, None, None).unwrap();
        assert_eq!(pinned(&db).unwrap(), None);
    }

    fn legacy_db(extraction: Option<(&str, &str)>, consolidation: Option<(&str, &str)>) -> Connection {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(
            "CREATE TABLE memory_extraction_settings (scope_key TEXT PRIMARY KEY, mode TEXT NOT NULL,
                harness TEXT, model TEXT, updated_at TEXT NOT NULL);
             CREATE TABLE memory_consolidation_settings (scope_key TEXT PRIMARY KEY, mode TEXT NOT NULL,
                harness TEXT, model TEXT, max_records INTEGER NOT NULL, allow_removal INTEGER NOT NULL,
                debounce_seconds INTEGER NOT NULL, updated_at TEXT NOT NULL);
             INSERT INTO memory_extraction_settings VALUES('account:local','propose',NULL,NULL,'now');",
        )
        .unwrap();
        if let Some((harness, model)) = extraction {
            db.execute(
                "UPDATE memory_extraction_settings SET harness=?1, model=?2",
                params![harness, model],
            )
            .unwrap();
        }
        if let Some((harness, model)) = consolidation {
            db.execute(
                "INSERT INTO memory_consolidation_settings VALUES('account:local','propose',?1,?2,200,0,120,'now')",
                params![harness, model],
            )
            .unwrap();
        }
        db
    }

    fn migrate(db: &mut Connection) {
        let transaction = db.transaction().unwrap();
        install(&transaction).unwrap();
        transaction.commit().unwrap();
    }

    #[test]
    fn migration_seeds_the_extraction_pin_before_the_consolidation_pin() {
        let mut db = legacy_db(Some(("claude", "claude-haiku-5-5")), Some(("claude", "claude-sonnet-5-5")));
        migrate(&mut db);
        assert_eq!(pinned(&db).unwrap().unwrap().model, "claude-haiku-5-5");
    }

    #[test]
    fn migration_falls_back_to_the_consolidation_pin_then_to_nothing() {
        let mut db = legacy_db(None, Some(("claude", "claude-sonnet-5-5")));
        migrate(&mut db);
        assert_eq!(pinned(&db).unwrap().unwrap().model, "claude-sonnet-5-5");

        let mut empty = legacy_db(None, None);
        migrate(&mut empty);
        assert_eq!(pinned(&empty).unwrap(), None);
    }

    #[test]
    fn a_cleared_pin_is_not_seeded_again() {
        let mut db = legacy_db(Some(("claude", "claude-haiku-5-5")), None);
        migrate(&mut db);
        set(&db, None, None).unwrap();
        migrate(&mut db);
        assert_eq!(pinned(&db).unwrap(), None);
    }
}
