//! Persisted Bridge prompt-section overrides and append-only revision history.

use crate::{prompt_compiler::PromptCompiler, prompts, BridgeError};
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};

const CONFIG_KIND: &str = "prompt_section";
const MAX_SECTION_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptSectionKey {
    pub target: prompts::PromptTarget,
    pub section_id: String,
}

impl PromptSectionKey {
    pub fn new(
        target: prompts::PromptTarget,
        section_id: impl Into<String>,
    ) -> Result<Self, BridgeError> {
        let key = Self {
            target,
            section_id: section_id.into(),
        };
        validate_key(&key)?;
        Ok(key)
    }

    fn configuration_id(&self) -> String {
        format!("{}:{}", self.target.storage_key(), self.section_id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum PromptSectionState {
    Default,
    Overridden { text: String },
    Deleted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptSectionOperation {
    Override,
    Delete,
    Reset,
    Restore,
}

impl PromptSectionOperation {
    fn as_str(self) -> &'static str {
        match self {
            Self::Override => "override",
            Self::Delete => "delete",
            Self::Reset => "reset",
            Self::Restore => "restore",
        }
    }

    fn parse(value: &str) -> Result<Self, BridgeError> {
        match value {
            "override" => Ok(Self::Override),
            "delete" => Ok(Self::Delete),
            "reset" => Ok(Self::Reset),
            "restore" => Ok(Self::Restore),
            _ => Err(BridgeError::Invalid(format!(
                "invalid stored prompt section operation {value:?}"
            ))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptSectionRevision {
    pub id: i64,
    pub key: PromptSectionKey,
    pub operation: PromptSectionOperation,
    pub state: PromptSectionState,
    pub restored_from_revision_id: Option<i64>,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedPromptSection {
    pub id: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedPromptStack {
    pub target: prompts::PromptTarget,
    pub sections: Vec<ResolvedPromptSection>,
}

pub fn resolve(
    db: &Connection,
    target: prompts::PromptTarget,
    worker_depth: i64,
) -> Result<ResolvedPromptStack, BridgeError> {
    let mut sections = Vec::new();
    for default in prompts::default_sections(target, worker_depth) {
        let key = PromptSectionKey::new(target, default.id)?;
        match current_state(db, &key)? {
            PromptSectionState::Default => sections.push(ResolvedPromptSection {
                id: default.id.into(),
                text: default.text,
            }),
            PromptSectionState::Overridden { text } => sections.push(ResolvedPromptSection {
                id: default.id.into(),
                text,
            }),
            PromptSectionState::Deleted => {}
        }
    }
    Ok(ResolvedPromptStack { target, sections })
}

pub fn current_state(
    db: &Connection,
    key: &PromptSectionKey,
) -> Result<PromptSectionState, BridgeError> {
    validate_key(key)?;
    let payload: Option<String> = db
        .query_row(
            "SELECT payload FROM configuration_entries WHERE kind=?1 AND id=?2",
            params![CONFIG_KIND, key.configuration_id()],
            |row| row.get(0),
        )
        .optional()?;
    let Some(payload) = payload else {
        return Ok(PromptSectionState::Default);
    };
    let state: PromptSectionState = match serde_json::from_str(&payload) {
        Ok(state) => state,
        Err(error) => {
            // A corrupt or forward-incompatible payload should not hard-fail
            // every session launch. Fall back to the compiled default.
            eprintln!(
                "bridge-core: falling back to default prompt section state for {:?}: {}",
                key, error
            );
            return Ok(PromptSectionState::Default);
        }
    };
    if state == PromptSectionState::Default {
        return Err(BridgeError::Invalid(
            "stored prompt section state cannot be default".into(),
        ));
    }
    validate_state(key, &state)?;
    Ok(state)
}

pub fn save_override(
    db: &Connection,
    key: &PromptSectionKey,
    text: impl Into<String>,
) -> Result<PromptSectionRevision, BridgeError> {
    mutate(
        db,
        key,
        PromptSectionState::Overridden { text: text.into() },
        PromptSectionOperation::Override,
        None,
    )
}

pub fn delete_section(
    db: &Connection,
    key: &PromptSectionKey,
) -> Result<PromptSectionRevision, BridgeError> {
    mutate(
        db,
        key,
        PromptSectionState::Deleted,
        PromptSectionOperation::Delete,
        None,
    )
}

pub fn reset_section(
    db: &Connection,
    key: &PromptSectionKey,
) -> Result<PromptSectionRevision, BridgeError> {
    mutate(
        db,
        key,
        PromptSectionState::Default,
        PromptSectionOperation::Reset,
        None,
    )
}

pub fn restore_revision(
    db: &Connection,
    key: &PromptSectionKey,
    revision_id: i64,
) -> Result<PromptSectionRevision, BridgeError> {
    validate_key(key)?;
    let transaction = db.unchecked_transaction()?;
    let state = revision_state(&transaction, key, revision_id)?.ok_or_else(|| {
        BridgeError::Invalid(format!(
            "prompt section revision {revision_id} does not belong to {}:{}",
            key.target.storage_key(),
            key.section_id
        ))
    })?;
    validate_state(key, &state)?;
    apply_active_state(&transaction, key, &state)?;
    let revision = append_revision(
        &transaction,
        key,
        PromptSectionOperation::Restore,
        &state,
        Some(revision_id),
    )?;
    transaction.commit()?;
    Ok(revision)
}

pub fn revisions(
    db: &Connection,
    key: &PromptSectionKey,
) -> Result<Vec<PromptSectionRevision>, BridgeError> {
    validate_key(key)?;
    let mut statement = db.prepare(
        "SELECT id,operation,state,content,restored_from_revision_id,created_at
         FROM prompt_section_revisions
         WHERE target=?1 AND section_id=?2
         ORDER BY id",
    )?;
    let rows = statement
        .query_map(params![key.target.storage_key(), key.section_id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<i64>>(4)?,
                row.get::<_, String>(5)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    rows.into_iter()
        .map(
            |(id, operation, state, content, restored_from_revision_id, created_at)| {
                Ok(PromptSectionRevision {
                    id,
                    key: key.clone(),
                    operation: PromptSectionOperation::parse(&operation)?,
                    state: state_from_columns(&state, content)?,
                    restored_from_revision_id,
                    created_at,
                })
            },
        )
        .collect()
}

fn mutate(
    db: &Connection,
    key: &PromptSectionKey,
    state: PromptSectionState,
    operation: PromptSectionOperation,
    restored_from_revision_id: Option<i64>,
) -> Result<PromptSectionRevision, BridgeError> {
    validate_key(key)?;
    validate_state(key, &state)?;
    let transaction = db.unchecked_transaction()?;
    apply_active_state(&transaction, key, &state)?;
    let revision = append_revision(
        &transaction,
        key,
        operation,
        &state,
        restored_from_revision_id,
    )?;
    transaction.commit()?;
    Ok(revision)
}

fn validate_key(key: &PromptSectionKey) -> Result<(), BridgeError> {
    if !key.target.section_ids().contains(&key.section_id.as_str()) {
        return Err(BridgeError::Invalid(format!(
            "prompt section {:?} is not available for target {}",
            key.section_id,
            key.target.storage_key()
        )));
    }
    Ok(())
}

fn validate_state(key: &PromptSectionKey, state: &PromptSectionState) -> Result<(), BridgeError> {
    let PromptSectionState::Overridden { text } = state else {
        return Ok(());
    };
    if text.trim().is_empty() {
        return Err(BridgeError::Invalid(
            "prompt section override cannot be empty; delete the section instead".into(),
        ));
    }
    if text.len() > MAX_SECTION_BYTES {
        return Err(BridgeError::Invalid(format!(
            "prompt section override exceeds the {MAX_SECTION_BYTES} byte limit"
        )));
    }
    PromptCompiler::new(key.target.compiler_role())
        .stable_section(&key.section_id, text)
        .compile()?;
    Ok(())
}

fn apply_active_state(
    db: &Connection,
    key: &PromptSectionKey,
    state: &PromptSectionState,
) -> Result<(), BridgeError> {
    if state == &PromptSectionState::Default {
        db.execute(
            "DELETE FROM configuration_entries WHERE kind=?1 AND id=?2",
            params![CONFIG_KIND, key.configuration_id()],
        )?;
        return Ok(());
    }
    let now = Utc::now().to_rfc3339();
    let payload =
        serde_json::to_string(state).map_err(|error| BridgeError::Invalid(error.to_string()))?;
    db.execute(
        "INSERT INTO configuration_entries(kind,id,payload,created_at,updated_at)
         VALUES(?1,?2,?3,?4,?4)
         ON CONFLICT(kind,id) DO UPDATE SET payload=excluded.payload,updated_at=excluded.updated_at",
        params![CONFIG_KIND, key.configuration_id(), payload, now],
    )?;
    Ok(())
}

fn append_revision(
    db: &Connection,
    key: &PromptSectionKey,
    operation: PromptSectionOperation,
    state: &PromptSectionState,
    restored_from_revision_id: Option<i64>,
) -> Result<PromptSectionRevision, BridgeError> {
    let (state_name, content) = state_columns(state);
    let created_at = Utc::now().to_rfc3339();
    db.execute(
        "INSERT INTO prompt_section_revisions(target,section_id,operation,state,content,restored_from_revision_id,created_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7)",
        params![
            key.target.storage_key(),
            key.section_id,
            operation.as_str(),
            state_name,
            content,
            restored_from_revision_id,
            created_at
        ],
    )?;
    Ok(PromptSectionRevision {
        id: db.last_insert_rowid(),
        key: key.clone(),
        operation,
        state: state.clone(),
        restored_from_revision_id,
        created_at,
    })
}

fn revision_state(
    db: &Connection,
    key: &PromptSectionKey,
    revision_id: i64,
) -> Result<Option<PromptSectionState>, BridgeError> {
    let columns: Option<(String, Option<String>)> = db
        .query_row(
            "SELECT state,content FROM prompt_section_revisions
             WHERE id=?1 AND target=?2 AND section_id=?3",
            params![revision_id, key.target.storage_key(), key.section_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    columns
        .map(|(state, content)| state_from_columns(&state, content))
        .transpose()
}

fn state_columns(state: &PromptSectionState) -> (&'static str, Option<&str>) {
    match state {
        PromptSectionState::Default => ("default", None),
        PromptSectionState::Overridden { text } => ("overridden", Some(text)),
        PromptSectionState::Deleted => ("deleted", None),
    }
}

fn state_from_columns(
    state: &str,
    content: Option<String>,
) -> Result<PromptSectionState, BridgeError> {
    match (state, content) {
        ("default", None) => Ok(PromptSectionState::Default),
        ("overridden", Some(text)) => Ok(PromptSectionState::Overridden { text }),
        ("deleted", None) => Ok(PromptSectionState::Deleted),
        _ => Err(BridgeError::Invalid(format!(
            "invalid stored prompt section revision state {state:?}"
        ))),
    }
}

fn key_from_configuration_id(value: &str) -> Result<PromptSectionKey, BridgeError> {
    for target in prompts::PROMPT_TARGETS {
        let prefix = format!("{}:", target.storage_key());
        if let Some(section_id) = value.strip_prefix(&prefix) {
            return PromptSectionKey::new(*target, section_id);
        }
    }
    Err(BridgeError::Invalid(format!(
        "invalid stored prompt section key {value:?}"
    )))
}

pub(crate) fn append_reset_all_revisions(db: &Connection) -> Result<(), BridgeError> {
    let ids = {
        let mut statement = db
            .prepare("SELECT id FROM configuration_entries WHERE kind=?1 ORDER BY created_at,id")?;
        let ids = statement
            .query_map(params![CONFIG_KIND], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        ids
    };
    for id in ids {
        let Ok(key) = key_from_configuration_id(&id) else {
            continue;
        };
        append_revision(
            db,
            &key,
            PromptSectionOperation::Reset,
            &PromptSectionState::Default,
            None,
        )?;
    }
    Ok(())
}

pub(crate) fn install_revision_store(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS prompt_section_revisions (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            target TEXT NOT NULL,
            section_id TEXT NOT NULL,
            operation TEXT NOT NULL CHECK(operation IN ('override','delete','reset','restore')),
            state TEXT NOT NULL CHECK(state IN ('default','overridden','deleted')),
            content TEXT,
            restored_from_revision_id INTEGER REFERENCES prompt_section_revisions(id),
            created_at TEXT NOT NULL,
            CHECK(
                (state='overridden' AND content IS NOT NULL AND length(trim(content)) > 0)
                OR (state IN ('default','deleted') AND content IS NULL)
            ),
            CHECK(
                (operation='restore' AND restored_from_revision_id IS NOT NULL)
                OR (operation!='restore' AND restored_from_revision_id IS NULL)
            ),
            CHECK(
                operation='restore'
                OR (operation='override' AND state='overridden')
                OR (operation='delete' AND state='deleted')
                OR (operation='reset' AND state='default')
            ),
            CHECK(
                (target='orchestrator' AND section_id IN ('bridge_role','delegation_protocol'))
                OR (
                    target IN (
                        'worker:research','worker:implementation','worker:verification',
                        'worker:planning','worker:documentation'
                    )
                    AND section_id='worker_contract'
                )
            )
        );
        CREATE INDEX IF NOT EXISTS idx_prompt_section_revisions_key
            ON prompt_section_revisions(target,section_id,id DESC);
        CREATE TRIGGER IF NOT EXISTS prompt_section_revisions_no_update
            BEFORE UPDATE ON prompt_section_revisions
            BEGIN SELECT RAISE(ABORT,'prompt section revisions are append-only'); END;
        CREATE TRIGGER IF NOT EXISTS prompt_section_revisions_no_delete
            BEFORE DELETE ON prompt_section_revisions
            BEGIN SELECT RAISE(ABORT,'prompt section revisions are append-only'); END;
        CREATE TRIGGER IF NOT EXISTS prompt_section_revisions_no_replace
            BEFORE INSERT ON prompt_section_revisions
            WHEN EXISTS(SELECT 1 FROM prompt_section_revisions WHERE id=NEW.id)
            BEGIN SELECT RAISE(ABORT,'prompt section revisions are append-only'); END;",
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{agent_config, delegation::WorkerRole, store};
    use std::path::Path;

    fn db() -> Connection {
        store::open(Path::new(":memory:")).unwrap()
    }

    #[test]
    fn override_delete_reset_and_restore_round_trip() {
        let db = db();
        let key = PromptSectionKey::new(
            prompts::PromptTarget::Orchestrator,
            prompts::BRIDGE_ROLE_SECTION_ID,
        )
        .unwrap();
        assert_eq!(
            current_state(&db, &key).unwrap(),
            PromptSectionState::Default
        );

        let overridden = save_override(&db, &key, "Custom orchestrator policy").unwrap();
        assert_eq!(
            current_state(&db, &key).unwrap(),
            PromptSectionState::Overridden {
                text: "Custom orchestrator policy".into()
            }
        );
        let deleted = delete_section(&db, &key).unwrap();
        assert_eq!(
            current_state(&db, &key).unwrap(),
            PromptSectionState::Deleted
        );
        let deleted_stack = resolve(&db, key.target, 0).unwrap();
        assert!(!deleted_stack
            .sections
            .iter()
            .any(|section| section.id == prompts::BRIDGE_ROLE_SECTION_ID));
        assert!(deleted_stack
            .sections
            .iter()
            .any(|section| section.id == prompts::DELEGATION_PROTOCOL_SECTION_ID));
        let reset = reset_section(&db, &key).unwrap();
        assert_eq!(
            current_state(&db, &key).unwrap(),
            PromptSectionState::Default
        );
        let reset_stack = resolve(&db, key.target, 0).unwrap();
        assert!(reset_stack
            .sections
            .iter()
            .find(|section| section.id == prompts::BRIDGE_ROLE_SECTION_ID)
            .unwrap()
            .text
            .contains("starter orchestrator"));

        let restored_override = restore_revision(&db, &key, overridden.id).unwrap();
        assert_eq!(
            restored_override.restored_from_revision_id,
            Some(overridden.id)
        );
        assert!(matches!(
            current_state(&db, &key).unwrap(),
            PromptSectionState::Overridden { .. }
        ));
        restore_revision(&db, &key, deleted.id).unwrap();
        assert_eq!(
            current_state(&db, &key).unwrap(),
            PromptSectionState::Deleted
        );
        restore_revision(&db, &key, reset.id).unwrap();
        assert_eq!(
            current_state(&db, &key).unwrap(),
            PromptSectionState::Default
        );

        let history = revisions(&db, &key).unwrap();
        assert_eq!(history.len(), 6);
        assert_eq!(history[0], overridden);
        assert_eq!(history[1], deleted);
        assert_eq!(history[2], reset);
        assert!(history[3..]
            .iter()
            .all(|revision| revision.operation == PromptSectionOperation::Restore));
        assert!(db
            .execute(
                "UPDATE prompt_section_revisions SET content='rewritten' WHERE id=?1",
                params![overridden.id],
            )
            .unwrap_err()
            .to_string()
            .contains("append-only"));
        assert!(db
            .execute(
                "INSERT OR REPLACE INTO prompt_section_revisions(
                    id,target,section_id,operation,state,content,created_at
                 ) VALUES(?1,'orchestrator','bridge_role','override','overridden','rewritten','now')",
                params![overridden.id],
            )
            .unwrap_err()
            .to_string()
            .contains("append-only"));
        assert!(db
            .execute(
                "DELETE FROM prompt_section_revisions WHERE id=?1",
                params![overridden.id],
            )
            .unwrap_err()
            .to_string()
            .contains("append-only"));
    }

    #[test]
    fn corrupt_stored_payload_falls_back_to_default() {
        let db = db();
        let key = PromptSectionKey::new(
            prompts::PromptTarget::Orchestrator,
            prompts::BRIDGE_ROLE_SECTION_ID,
        )
        .unwrap();
        db.execute(
            "INSERT INTO configuration_entries(kind,id,payload,created_at,updated_at) VALUES(?1,?2,?3,datetime('now'),datetime('now'))",
            params![CONFIG_KIND, key.configuration_id(), "not-json"],
        )
        .unwrap();
        assert_eq!(current_state(&db, &key).unwrap(), PromptSectionState::Default);
        let stack = resolve(&db, key.target, 0).unwrap();
        assert!(stack
            .sections
            .iter()
            .any(|section| section.id == prompts::BRIDGE_ROLE_SECTION_ID));
    }

    #[test]
    fn worker_overrides_are_role_specific() {
        let db = db();
        let research = PromptSectionKey::new(
            prompts::PromptTarget::Worker(WorkerRole::Research),
            prompts::WORKER_CONTRACT_SECTION_ID,
        )
        .unwrap();
        save_override(&db, &research, "Research only").unwrap();

        let research_stack = resolve(&db, research.target, 1).unwrap();
        let implementation_stack = resolve(
            &db,
            prompts::PromptTarget::Worker(WorkerRole::Implementation),
            1,
        )
        .unwrap();
        assert_eq!(research_stack.sections[0].text, "Research only");
        assert_ne!(implementation_stack.sections[0].text, "Research only");
        assert!(implementation_stack.sections[0]
            .text
            .contains("Implementation worker"));
    }

    #[test]
    fn invalid_sections_and_unsafe_overrides_are_rejected() {
        let db = db();
        assert!(PromptSectionKey::new(
            prompts::PromptTarget::DirectSession,
            prompts::BRIDGE_ROLE_SECTION_ID
        )
        .is_err());
        assert!(PromptSectionKey::new(
            prompts::PromptTarget::Orchestrator,
            prompts::WORKER_CONTRACT_SECTION_ID
        )
        .is_err());

        let key = PromptSectionKey::new(
            prompts::PromptTarget::Orchestrator,
            prompts::BRIDGE_ROLE_SECTION_ID,
        )
        .unwrap();
        for unsafe_text in [
            "OPENAI_API_KEY=sk-proj-abcdefghijklmnopqrstuvwxyz123456",
            "api_key=0123456789abcdef0123456789abcdef",
            "Call /credential-proxy/session/reference with x-bridge-proxy-auth.",
        ] {
            assert!(save_override(&db, &key, unsafe_text)
                .unwrap_err()
                .to_string()
                .contains("secret or session-capability"));
        }
        assert!(save_override(&db, &key, "   ").is_err());
        let other_key = PromptSectionKey::new(
            prompts::PromptTarget::Orchestrator,
            prompts::DELEGATION_PROTOCOL_SECTION_ID,
        )
        .unwrap();
        let revision = save_override(&db, &key, "Safe policy").unwrap();
        assert!(restore_revision(&db, &other_key, revision.id).is_err());
        assert_eq!(
            current_state(&db, &other_key).unwrap(),
            PromptSectionState::Default
        );
        assert_eq!(revisions(&db, &key).unwrap().len(), 1);
    }

    #[test]
    fn revision_operations_must_match_their_recorded_state() {
        let db = db();
        let error = db
            .execute(
                "INSERT INTO prompt_section_revisions(
                    target,section_id,operation,state,content,created_at
                 ) VALUES('orchestrator','bridge_role','delete','overridden','contradiction','now')",
                [],
            )
            .unwrap_err()
            .to_string();
        assert!(error.contains("CHECK constraint failed"), "{error}");
    }

    #[test]
    fn failed_revision_append_rolls_back_active_state() {
        let db = db();
        let key = PromptSectionKey::new(
            prompts::PromptTarget::Orchestrator,
            prompts::BRIDGE_ROLE_SECTION_ID,
        )
        .unwrap();
        db.execute_batch(
            "CREATE TRIGGER fail_prompt_revision BEFORE INSERT ON prompt_section_revisions
             BEGIN SELECT RAISE(ABORT,'injected revision failure'); END;",
        )
        .unwrap();

        assert!(save_override(&db, &key, "Must roll back").is_err());
        assert_eq!(
            current_state(&db, &key).unwrap(),
            PromptSectionState::Default
        );
    }

    #[test]
    fn agent_reset_all_does_not_bypass_prompt_revision_history() {
        let db = db();
        let key = PromptSectionKey::new(
            prompts::PromptTarget::Orchestrator,
            prompts::BRIDGE_ROLE_SECTION_ID,
        )
        .unwrap();
        save_override(&db, &key, "Temporary policy").unwrap();

        agent_config::reset_all(&db).unwrap();

        assert_eq!(
            current_state(&db, &key).unwrap(),
            PromptSectionState::Default
        );
        let history = revisions(&db, &key).unwrap();
        assert_eq!(history.len(), 2);
        assert_eq!(history[1].operation, PromptSectionOperation::Reset);
        assert_eq!(history[1].state, PromptSectionState::Default);
    }

    #[test]
    fn agent_reset_all_recovers_from_an_unknown_prompt_section_key() {
        let db = db();
        db.execute(
            "INSERT INTO configuration_entries(kind,id,payload,created_at,updated_at)
             VALUES('prompt_section','future:unknown','{\"state\":\"deleted\"}','now','now')",
            [],
        )
        .unwrap();

        agent_config::reset_all(&db).unwrap();

        let remaining: i64 = db
            .query_row("SELECT COUNT(*) FROM configuration_entries", [], |row| row.get(0))
            .unwrap();
        assert_eq!(remaining, 0);
    }
}
