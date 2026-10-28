//! Workflows: saved, approved, repeatable jobs on the user's own harness.
//!
//! This module owns the definition, its revisions and its approval. Nothing
//! here starts a run; triggering and running live in their own modules and
//! read what is stored here.
//!
//! Three rules carry the safety story and are enforced in Rust, never in a
//! client:
//!
//! * **Revisions are immutable.** An edit inserts a revision; the only column
//!   ever updated on one is `approved_at`, set once.
//! * **Widening clears approval.** A revision that lets the workflow touch more
//!   (a new server, read or write turned on, a new always-allowed tool, a
//!   worktree, another harness, another project) waits for a person.
//! * **No project means no project row.** A workflow without a project runs in
//!   `<data_dir>/workflows/<id>/`, a folder Bridge owns. `projects.path` is a
//!   real repository and stays one.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use bridge_protocol::messages as wire;
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use uuid::Uuid;

use crate::workflow_schedule::Schedule;
use crate::BridgeError;

/// The version of the stored definition JSON.
pub const DEFINITION_SCHEMA_VERSION: i64 = 1;
pub const MAX_NAME_CHARS: usize = 80;
pub const MAX_INSTRUCTIONS_CHARS: usize = 20_000;
pub const MAX_EXPECTED_RESULT_CHARS: usize = 2_000;
/// Why Bridge disabled a workflow whose project row or folder is gone.
pub const DISABLED_PROJECT_REMOVED: &str = "project_removed";

/// Who wrote a revision. Stored, so an approval can be traced to its origin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreatedBy {
    User,
    DraftFromText,
    DraftFromSession,
    /// "Always allow for this workflow" on a real call. Approved by
    /// construction: the person made exactly this widening.
    AlwaysAllow,
}

impl CreatedBy {
    fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::DraftFromText => "draft_from_text",
            Self::DraftFromSession => "draft_from_session",
            Self::AlwaysAllow => "always_allow",
        }
    }
}

/// Installs the workflow tables. Runs as one schema migration.
pub fn install_store(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS workflows (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            project_id TEXT,
            enabled INTEGER NOT NULL DEFAULT 1,
            disabled_reason TEXT,
            current_revision_id TEXT NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_workflows_project ON workflows(project_id);
        CREATE TABLE IF NOT EXISTS workflow_revisions (
            id TEXT PRIMARY KEY,
            workflow_id TEXT NOT NULL REFERENCES workflows(id) ON DELETE CASCADE,
            revision INTEGER NOT NULL,
            schema_version INTEGER NOT NULL,
            definition TEXT NOT NULL,
            digest TEXT NOT NULL,
            approved_at TEXT,
            created_by TEXT NOT NULL
                CHECK (created_by IN ('user','draft_from_text','draft_from_session','always_allow')),
            created_at TEXT NOT NULL,
            UNIQUE(workflow_id, revision)
        );
        CREATE TABLE IF NOT EXISTS workflow_runs (
            id TEXT PRIMARY KEY,
            workflow_id TEXT NOT NULL REFERENCES workflows(id) ON DELETE CASCADE,
            revision_id TEXT NOT NULL,
            trigger_kind TEXT NOT NULL CHECK (trigger_kind IN ('manual','schedule','catch_up')),
            occasion TEXT NOT NULL,
            idempotency_key TEXT NOT NULL UNIQUE,
            missed_slots INTEGER NOT NULL DEFAULT 0,
            dry_run INTEGER NOT NULL DEFAULT 0,
            status TEXT NOT NULL CHECK (status IN (
                'queued','waiting_quota','running','waiting_approval',
                'succeeded','unverified','failed','cancelled','skipped')),
            failure_code TEXT,
            summary TEXT,
            session_id TEXT,
            lease_owner TEXT,
            lease_expires_at TEXT,
            quota_unknown INTEGER NOT NULL DEFAULT 0,
            cost_microusd INTEGER,
            tokens INTEGER,
            unread INTEGER NOT NULL DEFAULT 1,
            scheduled_for TEXT,
            started_at TEXT,
            finished_at TEXT,
            created_at TEXT NOT NULL,
            CHECK ((lease_owner IS NULL) = (lease_expires_at IS NULL))
        );
        CREATE UNIQUE INDEX IF NOT EXISTS idx_workflow_runs_one_active ON workflow_runs(workflow_id)
            WHERE status IN ('queued','waiting_quota','running','waiting_approval');
        CREATE INDEX IF NOT EXISTS idx_workflow_runs_recent ON workflow_runs(workflow_id, created_at);
        CREATE TABLE IF NOT EXISTS workflow_run_effects (
            id TEXT PRIMARY KEY,
            run_id TEXT NOT NULL REFERENCES workflow_runs(id) ON DELETE CASCADE,
            effect_key TEXT NOT NULL UNIQUE,
            tool_identity TEXT NOT NULL,
            arguments_digest TEXT NOT NULL,
            decision TEXT NOT NULL CHECK (decision IN (
                'previewed','approved','always_allowed','declined','executed','failed')),
            preview TEXT,
            decided_by TEXT,
            created_at TEXT NOT NULL,
            executed_at TEXT
        );",
    )?;
    Ok(())
}

/// The private folder a workflow with no project runs in.
pub fn workflow_home(data_dir: &Path, workflow_id: &str) -> PathBuf {
    data_dir.join("workflows").join(workflow_id)
}

// --- validation ---------------------------------------------------------------

/// What one registered adapter offers, as validation needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdapterFacts {
    pub id: String,
    /// Empty when the catalog is discovered at run time; then any non-empty
    /// model name is accepted.
    pub models: Vec<String>,
}

/// Everything validation needs that is not in the definition. Built by the
/// API layer from the adapter registry and the harness's configuration, so the
/// rules here stay pure and testable without spawning anything.
#[derive(Debug, Clone, Default)]
pub struct ValidationContext {
    pub adapters: Vec<AdapterFacts>,
    /// Connector servers the harness's own configuration holds.
    pub servers: BTreeSet<String>,
}

/// Does this definition need an adapter that enforces exact tool identities?
/// True for any connector grant, and for a run with no built-in tools at all,
/// because only such an adapter can deny the built-ins by name.
pub fn needs_identity_enforcement(definition: &wire::WorkflowDefinition) -> bool {
    !definition.grants.is_empty() || definition.files == wire::WorkflowFiles::None
}

/// The authority. A client may pre-empt an obvious mistake; this decides.
pub fn validate(definition: &wire::WorkflowDefinition, context: &ValidationContext) -> Result<(), String> {
    let name = definition.name.trim();
    if name.is_empty() {
        return Err("A workflow needs a name".into());
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(format!("A workflow name is at most {MAX_NAME_CHARS} characters"));
    }

    let [step] = definition.steps.as_slice() else {
        return Err(format!(
            "A workflow has exactly one step for now; this one has {}",
            definition.steps.len()
        ));
    };
    let harness = step.harness.as_str();
    let Some(adapter) = context.adapters.iter().find(|adapter| adapter.id == harness) else {
        return Err(format!("`{harness}` is not a registered agent"));
    };
    let model = step.model.trim();
    if model.is_empty() {
        return Err("Choose a model for the workflow's agent".into());
    }
    if !adapter.models.is_empty() && !adapter.models.iter().any(|known| known == model) {
        return Err(format!("`{model}` is not a model `{harness}` offers"));
    }
    if step.effort.as_deref().is_some_and(|effort| effort.trim().is_empty()) {
        return Err("An effort, when set, cannot be empty".into());
    }
    let instructions = step.instructions.trim();
    if instructions.is_empty() {
        return Err("Tell the workflow what to do".into());
    }
    if instructions.chars().count() > MAX_INSTRUCTIONS_CHARS {
        return Err(format!("Instructions are at most {MAX_INSTRUCTIONS_CHARS} characters"));
    }

    if definition.files == wire::WorkflowFiles::Worktree && definition.project_id.is_none() {
        return Err("A worktree needs a project; choose one or turn file access off".into());
    }
    if needs_identity_enforcement(definition) {
        crate::briefing_policy::adapter_may_brief(harness).map_err(|refusal| {
            format!("`{harness}` cannot run this workflow: {}", refusal.reason())
        })?;
    }

    if let wire::WorkflowTrigger::Schedule { cron, timezone } = &definition.trigger {
        Schedule::parse(cron, timezone)?;
    }
    validate_budget(&definition.budget)?;

    let mut servers = BTreeSet::new();
    for grant in &definition.grants {
        let server = grant.server.trim();
        if server.is_empty() || server != grant.server {
            return Err("A connector grant needs a server id with no surrounding spaces".into());
        }
        if !servers.insert(server) {
            return Err(format!("`{server}` is granted twice"));
        }
        if !grant.read && grant.write == wire::WorkflowWriteAccess::Never {
            return Err(format!("The grant for `{server}` allows nothing; remove it instead"));
        }
        if !context.servers.contains(server) {
            return Err(format!("`{server}` is not a connector `{harness}` is configured with"));
        }
    }

    let mut always = BTreeSet::new();
    for identity in &definition.always_allow {
        let Some((server, _tool)) = split_identity(identity) else {
            return Err(format!("`{identity}` is not an exact tool identity (mcp__<server>__<tool>)"));
        };
        let writable = definition
            .grants
            .iter()
            .any(|grant| wire_server_name(&grant.server) == server && grant.write == wire::WorkflowWriteAccess::Ask);
        if !writable {
            return Err(format!("`{identity}` is always allowed, but `{server}` is not granted writes"));
        }
        if !always.insert(identity.as_str()) {
            return Err(format!("`{identity}` is always allowed twice"));
        }
    }

    if definition.expected_result.chars().count() > MAX_EXPECTED_RESULT_CHARS {
        return Err(format!("The expected result is at most {MAX_EXPECTED_RESULT_CHARS} characters"));
    }
    Ok(())
}

/// The same bounds Work briefings enforce, for the same reasons.
fn validate_budget(budget: &wire::WorkBriefLimits) -> Result<(), String> {
    use crate::work::{MAX_TOOL_CALLS_CEILING, MAX_TURNS_CEILING, MAX_WALL_SECONDS, MIN_WALL_SECONDS};
    if !(MIN_WALL_SECONDS..=MAX_WALL_SECONDS).contains(&budget.max_wall_seconds) {
        return Err(format!("A run's time limit is between {MIN_WALL_SECONDS} and {MAX_WALL_SECONDS} seconds"));
    }
    if !(1..=MAX_TURNS_CEILING).contains(&budget.max_turns) {
        return Err(format!("A run's turn limit is between 1 and {MAX_TURNS_CEILING}"));
    }
    if !(1..=MAX_TOOL_CALLS_CEILING).contains(&budget.max_tool_calls) {
        return Err(format!("A run's tool-call limit is between 1 and {MAX_TOOL_CALLS_CEILING}"));
    }
    if budget.max_output_tokens.is_some_and(|tokens| tokens <= 0) {
        return Err("A token ceiling, when set, is positive".into());
    }
    if budget.cost_ceiling_microusd.is_some_and(|cost| cost <= 0) {
        return Err("A cost ceiling, when set, is positive".into());
    }
    Ok(())
}

/// The server segment a harness uses in tool names for a configured server:
/// anything outside `[A-Za-z0-9_-]` becomes `_` (`claude.ai Slack` is
/// `claude_ai_Slack` in `mcp__claude_ai_Slack__slack_read_thread`).
pub fn wire_server_name(server: &str) -> String {
    server
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' })
        .collect()
}

/// Split `mcp__<server>__<tool>`, the form the harness names connector tools.
pub fn split_identity(identity: &str) -> Option<(&str, &str)> {
    let rest = identity.strip_prefix("mcp__")?;
    let (server, tool) = rest.split_once("__")?;
    (!server.is_empty() && !tool.is_empty() && tool == tool.to_lowercase()).then_some((server, tool))
}

/// Whether a tool call reads or may change something.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolClass {
    Read,
    Write,
}

/// Classify a tool by name with the briefing read-verb vocabulary. Anything
/// that is not recognisably a read is a write: fail closed.
pub fn tool_class(identity: &str) -> ToolClass {
    match split_identity(identity) {
        Some((_, tool)) if crate::briefing_policy::is_read_tool_name(tool) => ToolClass::Read,
        _ => ToolClass::Write,
    }
}

/// Does `next` let the workflow touch anything `previous` did not?
pub fn widens(previous: &wire::WorkflowDefinition, next: &wire::WorkflowDefinition) -> bool {
    if previous.project_id != next.project_id {
        return true;
    }
    let harness = |definition: &wire::WorkflowDefinition| definition.steps.first().map(|step| step.harness.clone());
    if harness(previous) != harness(next) {
        return true;
    }
    if previous.files == wire::WorkflowFiles::None && next.files == wire::WorkflowFiles::Worktree {
        return true;
    }
    let grant_widens = next.grants.iter().any(|grant| {
        match previous.grants.iter().find(|old| old.server == grant.server) {
            None => true,
            Some(old) => {
                (grant.read && !old.read)
                    || (grant.write == wire::WorkflowWriteAccess::Ask && old.write == wire::WorkflowWriteAccess::Never)
            }
        }
    });
    grant_widens || next.always_allow.iter().any(|identity| !previous.always_allow.contains(identity))
}

// --- store ---------------------------------------------------------------------

fn now() -> String {
    Utc::now().to_rfc3339()
}

fn digest(definition: &wire::WorkflowDefinition) -> Result<String, BridgeError> {
    let value = serde_json::to_value(definition).map_err(|error| BridgeError::Invalid(error.to_string()))?;
    Ok(crate::usage_import::sha256_hex(&crate::work_connectors::canonical_json(&value)))
}

fn project_exists(db: &Connection, project_id: &str) -> Result<bool, BridgeError> {
    Ok(db.query_row(
        "SELECT EXISTS(SELECT 1 FROM projects WHERE id=?1)",
        params![project_id],
        |row| row.get(0),
    )?)
}

struct Row {
    id: String,
    enabled: bool,
    disabled_reason: Option<String>,
    revision_id: String,
    revision: i64,
    approved_at: Option<String>,
    definition: String,
    created_at: String,
    updated_at: String,
}

const SELECT_WORKFLOW: &str = "SELECT w.id, w.enabled, w.disabled_reason, r.id, r.revision, r.approved_at,
        r.definition, w.created_at, w.updated_at
     FROM workflows w JOIN workflow_revisions r ON r.id = w.current_revision_id";

fn read_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Row> {
    Ok(Row {
        id: row.get(0)?,
        enabled: row.get(1)?,
        disabled_reason: row.get(2)?,
        revision_id: row.get(3)?,
        revision: row.get(4)?,
        approved_at: row.get(5)?,
        definition: row.get(6)?,
        created_at: row.get(7)?,
        updated_at: row.get(8)?,
    })
}

fn into_workflow(row: Row) -> Result<wire::Workflow, BridgeError> {
    let definition: wire::WorkflowDefinition = serde_json::from_str(&row.definition)
        .map_err(|error| BridgeError::Invalid(format!("stored workflow {} is unreadable: {error}", row.id)))?;
    Ok(wire::Workflow {
        id: row.id,
        enabled: row.enabled,
        disabled_reason: row.disabled_reason,
        revision_id: row.revision_id,
        revision: row.revision,
        approved: row.approved_at.is_some(),
        approved_at: row.approved_at,
        definition,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

/// One workflow at its current revision, or `None`.
pub fn get(db: &Connection, workflow_id: &str) -> Result<Option<wire::Workflow>, BridgeError> {
    let row = db
        .query_row(&format!("{SELECT_WORKFLOW} WHERE w.id=?1"), params![workflow_id], read_row)
        .optional()?;
    row.map(into_workflow).transpose()
}

/// One workflow, or a refusal that names it.
pub fn require(db: &Connection, workflow_id: &str) -> Result<wire::Workflow, BridgeError> {
    get(db, workflow_id)?.ok_or_else(|| BridgeError::Invalid(format!("workflow {workflow_id} does not exist")))
}

/// Every workflow, most recently changed first.
pub fn list(db: &Connection) -> Result<Vec<wire::WorkflowSummary>, BridgeError> {
    let now = Utc::now();
    let mut statement = db.prepare(&format!("{SELECT_WORKFLOW} ORDER BY w.updated_at DESC, w.id"))?;
    let rows = statement.query_map([], read_row)?.collect::<Result<Vec<_>, _>>()?;
    rows.into_iter()
        .map(|row| {
            let workflow = into_workflow(row)?;
            let next_run_at = crate::workflow_trigger::next_run_at(db, &workflow, now)?;
            let last_run = crate::workflow_trigger::latest_run(db, &workflow.id)?;
            let step = workflow.definition.steps.first().cloned();
            Ok(wire::WorkflowSummary {
                id: workflow.id,
                name: workflow.definition.name.clone(),
                project_id: workflow.definition.project_id.clone(),
                enabled: workflow.enabled,
                disabled_reason: workflow.disabled_reason,
                approved: workflow.approved,
                trigger: workflow.definition.trigger.clone(),
                harness: step
                    .as_ref()
                    .map(|step| step.harness.clone())
                    .ok_or_else(|| BridgeError::Invalid("stored workflow has no step".into()))?,
                model: step.map(|step| step.model).unwrap_or_default(),
                files: workflow.definition.files,
                updated_at: workflow.updated_at,
                next_run_at,
                last_run,
            })
        })
        .collect()
}

/// Save a validated definition: create a workflow, or add a revision to one.
///
/// Validation is the caller's job (it needs the adapter registry); this checks
/// only what the database knows, the project.
pub fn save(
    db: &Connection,
    data_dir: &Path,
    workflow_id: Option<&str>,
    definition: &wire::WorkflowDefinition,
    created_by: CreatedBy,
) -> Result<wire::SaveWorkflowResult, BridgeError> {
    if let Some(project_id) = definition.project_id.as_deref() {
        if !project_exists(db, project_id)? {
            return Err(BridgeError::Invalid(format!("project {project_id} does not exist")));
        }
    }
    let stamp = now();
    let digest = digest(definition)?;
    let json = serde_json::to_string(definition).map_err(|error| BridgeError::Invalid(error.to_string()))?;
    let revision_id = Uuid::new_v4().to_string();
    let transaction = db.unchecked_transaction()?;

    let (id, revision, approved_at) = match workflow_id {
        None => (Uuid::new_v4().to_string(), 1, (created_by == CreatedBy::AlwaysAllow).then(|| stamp.clone())),
        Some(id) => {
            let current = require(&transaction, id)?;
            let approved_at = match created_by {
                CreatedBy::AlwaysAllow => Some(stamp.clone()),
                _ if widens(&current.definition, definition) => None,
                // Narrowing or rewording carries the approval it already had.
                _ => current.approved_at.clone(),
            };
            (id.to_owned(), current.revision + 1, approved_at)
        }
    };

    if revision == 1 {
        transaction.execute(
            "INSERT INTO workflows(id,name,project_id,enabled,disabled_reason,current_revision_id,created_at,updated_at)
             VALUES(?1,?2,?3,1,NULL,?4,?5,?5)",
            params![id, definition.name.trim(), definition.project_id, revision_id, stamp],
        )?;
    }
    transaction.execute(
        "INSERT INTO workflow_revisions(id,workflow_id,revision,schema_version,definition,digest,approved_at,created_by,created_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
        params![revision_id, id, revision, DEFINITION_SCHEMA_VERSION, json, digest, approved_at, created_by.as_str(), stamp],
    )?;
    if revision > 1 {
        transaction.execute(
            "UPDATE workflows SET name=?2, project_id=?3, current_revision_id=?4, updated_at=?5 WHERE id=?1",
            params![id, definition.name.trim(), definition.project_id, revision_id, stamp],
        )?;
    }
    transaction.commit()?;

    let home = workflow_home(data_dir, &id);
    if definition.project_id.is_none() {
        std::fs::create_dir_all(&home)
            .map_err(|error| BridgeError::Invalid(format!("could not create the workflow's folder: {error}")))?;
    } else if home.exists() {
        let _ = std::fs::remove_dir_all(&home);
    }

    let workflow = require(db, &id)?;
    Ok(wire::SaveWorkflowResult { needs_approval: !workflow.approved, workflow })
}

/// Approve the current revision. A stale revision id is refused: the person
/// approved something that is no longer what would run.
pub fn approve(db: &Connection, workflow_id: &str, revision_id: &str) -> Result<wire::Workflow, BridgeError> {
    let workflow = require(db, workflow_id)?;
    if workflow.revision_id != revision_id {
        return Err(BridgeError::Invalid(format!(
            "workflow {workflow_id} changed since revision {revision_id}; review revision {} instead",
            workflow.revision
        )));
    }
    db.execute(
        "UPDATE workflow_revisions SET approved_at=?2 WHERE id=?1 AND approved_at IS NULL",
        params![revision_id, now()],
    )?;
    require(db, workflow_id)
}

/// Turn a workflow on or off. Turning on a workflow whose project is still
/// missing is refused; if the project came back, its reason is cleared.
pub fn set_enabled(db: &Connection, workflow_id: &str, enabled: bool) -> Result<wire::Workflow, BridgeError> {
    let workflow = require(db, workflow_id)?;
    if enabled {
        if let Some(project_id) = workflow.definition.project_id.as_deref() {
            if !project_present(db, project_id)? {
                return Err(BridgeError::Invalid(format!(
                    "workflow {workflow_id} stays off: its project is gone"
                )));
            }
        }
    }
    db.execute(
        "UPDATE workflows SET enabled=?2, disabled_reason=NULL, updated_at=?3 WHERE id=?1",
        params![workflow_id, enabled, now()],
    )?;
    require(db, workflow_id)
}

/// Delete a workflow, its revisions, runs and effects, and its folder.
pub fn delete(db: &Connection, data_dir: &Path, workflow_id: &str) -> Result<(), BridgeError> {
    require(db, workflow_id)?;
    let transaction = db.unchecked_transaction()?;
    transaction.execute(
        "DELETE FROM workflow_run_effects WHERE run_id IN (SELECT id FROM workflow_runs WHERE workflow_id=?1)",
        params![workflow_id],
    )?;
    transaction.execute("DELETE FROM workflow_runs WHERE workflow_id=?1", params![workflow_id])?;
    transaction.execute("DELETE FROM workflow_revisions WHERE workflow_id=?1", params![workflow_id])?;
    transaction.execute("DELETE FROM workflows WHERE id=?1", params![workflow_id])?;
    transaction.commit()?;
    let home = workflow_home(data_dir, workflow_id);
    if home.exists() {
        std::fs::remove_dir_all(&home)
            .map_err(|error| BridgeError::Invalid(format!("deleted, but its folder remains: {error}")))?;
    }
    Ok(())
}

/// Is the project's row present and its folder still on disk?
fn project_present(db: &Connection, project_id: &str) -> Result<bool, BridgeError> {
    let path: Option<String> = db
        .query_row("SELECT path FROM projects WHERE id=?1", params![project_id], |row| row.get(0))
        .optional()?;
    Ok(path.is_some_and(|path| Path::new(&path).is_dir()))
}

/// Disable every enabled, project-bound workflow whose project is gone, and
/// return the ids it changed. Bridge has no project-removal path, so this runs
/// whenever workflows are read or claimed.
pub fn reconcile_projects(db: &Connection) -> Result<Vec<String>, BridgeError> {
    let mut statement = db.prepare(
        "SELECT id, project_id FROM workflows WHERE project_id IS NOT NULL AND enabled=1",
    )?;
    let candidates = statement
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?
        .collect::<Result<Vec<_>, _>>()?;
    let mut changed = Vec::new();
    for (id, project_id) in candidates {
        if !project_present(db, &project_id)? {
            db.execute(
                "UPDATE workflows SET enabled=0, disabled_reason=?2, updated_at=?3 WHERE id=?1",
                params![id, DISABLED_PROJECT_REMOVED, now()],
            )?;
            changed.push(id);
        }
    }
    Ok(changed)
}

/// A test core with stub Claude and Codex adapters, shared by the workflow
/// slices' tests. Stubs never start a provider.
#[cfg(test)]
pub(crate) mod test_support {
    use std::sync::Arc;

    use serde_json::{json, Value};

    use crate::adapters;
    use crate::model::AdapterDescriptor;
    use crate::runtime::BridgeCore;
    use crate::BridgeError;

    pub(crate) struct StubAdapter {
        pub id: &'static str,
    }

    impl adapters::HarnessAdapter for StubAdapter {
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
        fn descriptor(&self) -> AdapterDescriptor {
            serde_json::from_value(json!({
                "id": self.id, "label": self.id, "available": true, "authState": "unknown",
                "capabilities": [], "sandboxModes": ["read_only", "workspace_write"],
                "defaultModel": "sonnet",
                "models": [
                    {"id": "sonnet", "label": "Sonnet", "tier": "standard", "defaultForTier": true, "supportedEffortLevels": []},
                    {"id": "haiku", "label": "Haiku", "tier": "fast", "defaultForTier": true, "supportedEffortLevels": []}
                ]
            }))
            .unwrap()
        }
        fn start(&self, _: adapters::StartRequest<'_>) -> Result<adapters::StartedAdapter, BridgeError> {
            panic!("workflow store tests must not start a provider")
        }
        fn resume(&self, _: adapters::ResumeRequest<'_>) -> Result<adapters::StartedAdapter, BridgeError> {
            panic!("workflow store tests must not resume a provider")
        }
        fn supports_native_resume(&self) -> bool {
            false
        }
        fn normalize(&self, _: &Value) -> Vec<crate::agent::NormalizedEvent> {
            vec![]
        }
    }

    /// A valid no-project, read-only Slack workflow on a weekday schedule.
    pub(crate) fn definition() -> bridge_protocol::messages::WorkflowDefinition {
        use bridge_protocol::messages as wire;
        wire::WorkflowDefinition {
            name: "Morning brief".into(),
            project_id: None,
            steps: vec![wire::WorkflowStep {
                harness: wire::HarnessId::parse("claude").unwrap(),
                model: "haiku".into(),
                effort: None,
                instructions: "Summarise my Slack mentions.".into(),
            }],
            trigger: wire::WorkflowTrigger::Schedule { cron: "0 9 * * 1-5".into(), timezone: "UTC".into() },
            grants: vec![wire::WorkflowGrant { server: "claude_ai_Slack".into(), read: true, write: wire::WorkflowWriteAccess::Never }],
            always_allow: vec![],
            files: wire::WorkflowFiles::None,
            budget: wire::WorkBriefLimits {
                max_wall_seconds: 600,
                max_turns: 24,
                max_tool_calls: 64,
                max_output_tokens: None,
                cost_ceiling_microusd: None,
            },
            expected_result: String::new(),
            notify: wire::WorkflowNotify::default(),
        }
    }

    /// Save and approve `definition`, returning the stored workflow.
    pub(crate) fn save_approved(
        db: &rusqlite::Connection,
        data_dir: &std::path::Path,
        definition: &bridge_protocol::messages::WorkflowDefinition,
    ) -> bridge_protocol::messages::Workflow {
        let saved = super::save(db, data_dir, None, definition, super::CreatedBy::User).unwrap().workflow;
        super::approve(db, &saved.id, &saved.revision_id).unwrap()
    }

    /// A core whose registry holds stub `claude` and `codex` adapters.
    pub(crate) fn core() -> (tempfile::TempDir, Arc<BridgeCore>) {
        let dir = tempfile::tempdir().unwrap();
        let mut core = BridgeCore::for_tests(dir.path());
        let mut registry = adapters::AdapterRegistry::empty();
        registry.register(Box::new(StubAdapter { id: "claude" })).unwrap();
        registry.register(Box::new(StubAdapter { id: "codex" })).unwrap();
        core.adapter_registry = Arc::new(registry);
        (dir, Arc::new(core))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn db() -> (tempfile::TempDir, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let connection = crate::store::open(&dir.path().join("bridge.db")).unwrap();
        (dir, connection)
    }

    fn limits() -> wire::WorkBriefLimits {
        wire::WorkBriefLimits {
            max_wall_seconds: 600,
            max_turns: 24,
            max_tool_calls: 64,
            max_output_tokens: None,
            cost_ceiling_microusd: Some(500_000),
        }
    }

    pub(crate) fn definition() -> wire::WorkflowDefinition {
        wire::WorkflowDefinition {
            name: "Morning brief".into(),
            project_id: None,
            steps: vec![wire::WorkflowStep {
                harness: wire::HarnessId::parse("claude").unwrap(),
                model: "haiku".into(),
                effort: None,
                instructions: "Summarise my Slack mentions.".into(),
            }],
            trigger: wire::WorkflowTrigger::Schedule { cron: "0 9 * * 1-5".into(), timezone: "Asia/Kolkata".into() },
            grants: vec![wire::WorkflowGrant {
                server: "claude_ai_Slack".into(),
                read: true,
                write: wire::WorkflowWriteAccess::Never,
            }],
            always_allow: vec![],
            files: wire::WorkflowFiles::None,
            budget: limits(),
            expected_result: "Five lines or fewer.".into(),
            notify: wire::WorkflowNotify::default(),
        }
    }

    fn context() -> ValidationContext {
        ValidationContext {
            adapters: vec![
                AdapterFacts { id: "claude".into(), models: vec!["haiku".into(), "sonnet".into()] },
                AdapterFacts { id: "codex".into(), models: vec![] },
                AdapterFacts { id: "opencode".into(), models: vec![] },
            ],
            servers: BTreeSet::from(["claude_ai_Slack".to_owned(), "claude_ai_Linear".to_owned()]),
        }
    }

    fn add_project(db: &Connection, dir: &Path) -> String {
        let path = dir.join("repo");
        std::fs::create_dir_all(&path).unwrap();
        db.execute(
            "INSERT INTO projects(id,name,path,created_at) VALUES('p1','repo',?1,?2)",
            params![path.to_string_lossy(), now()],
        )
        .unwrap();
        "p1".into()
    }

    #[test]
    fn migration_installs_the_workflow_tables() {
        let (_dir, db) = db();
        for table in ["workflows", "workflow_revisions", "workflow_runs", "workflow_run_effects"] {
            let present: bool = db
                .query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)", params![table], |row| row.get(0))
                .unwrap();
            assert!(present, "{table}");
        }
    }

    #[test]
    fn saves_and_reads_back_with_and_without_a_project() {
        let (dir, db) = db();
        let saved = save(&db, dir.path(), None, &definition(), CreatedBy::User).unwrap();
        assert!(saved.needs_approval);
        assert_eq!(saved.workflow.definition, definition());
        assert_eq!(get(&db, &saved.workflow.id).unwrap().unwrap(), saved.workflow);

        let project = add_project(&db, dir.path());
        let mut bound = definition();
        bound.project_id = Some(project.clone());
        let saved = save(&db, dir.path(), None, &bound, CreatedBy::User).unwrap();
        assert_eq!(saved.workflow.definition.project_id, Some(project));
        assert_eq!(list(&db).unwrap().len(), 2);
    }

    #[test]
    fn an_edit_creates_a_new_immutable_revision() {
        let (dir, db) = db();
        let first = save(&db, dir.path(), None, &definition(), CreatedBy::User).unwrap().workflow;
        let mut edited = definition();
        edited.name = "Morning brief v2".into();
        let second = save(&db, dir.path(), Some(&first.id), &edited, CreatedBy::User).unwrap().workflow;
        assert_eq!(second.revision, 2);
        assert_ne!(second.revision_id, first.revision_id);
        let stored: String = db
            .query_row("SELECT definition FROM workflow_revisions WHERE id=?1", params![first.revision_id], |row| row.get(0))
            .unwrap();
        assert_eq!(serde_json::from_str::<wire::WorkflowDefinition>(&stored).unwrap(), definition());
    }

    #[test]
    fn widening_grants_clears_approval_and_narrowing_keeps_it() {
        let (dir, db) = db();
        let mut wide = definition();
        wide.grants[0].write = wire::WorkflowWriteAccess::Ask;
        let created = save(&db, dir.path(), None, &wide, CreatedBy::User).unwrap().workflow;
        let approved = approve(&db, &created.id, &created.revision_id).unwrap();
        assert!(approved.approved);

        // Narrow: write off. Approval carries.
        let narrowed = save(&db, dir.path(), Some(&created.id), &definition(), CreatedBy::User).unwrap();
        assert!(!narrowed.needs_approval);
        assert!(narrowed.workflow.approved);

        // Each widening clears it.
        let widenings: Vec<Box<dyn Fn(&mut wire::WorkflowDefinition)>> = vec![
            Box::new(|d| d.grants.push(wire::WorkflowGrant { server: "claude_ai_Linear".into(), read: true, write: wire::WorkflowWriteAccess::Never })),
            Box::new(|d| d.grants[0].write = wire::WorkflowWriteAccess::Ask),
            Box::new(|d| d.steps[0].harness = wire::HarnessId::parse("codex").unwrap()),
        ];
        for widen in widenings {
            let current = require(&db, &created.id).unwrap();
            approve(&db, &created.id, &current.revision_id).unwrap();
            let mut next = definition();
            widen(&mut next);
            let result = save(&db, dir.path(), Some(&created.id), &next, CreatedBy::User).unwrap();
            assert!(result.needs_approval, "{next:?}");
            save(&db, dir.path(), Some(&created.id), &definition(), CreatedBy::User).unwrap();
        }

        // The always-allow path is approved by construction.
        let mut always = definition();
        always.grants[0].write = wire::WorkflowWriteAccess::Ask;
        always.always_allow = vec!["mcp__claude_ai_Slack__slack_send_message".into()];
        let result = save(&db, dir.path(), Some(&created.id), &always, CreatedBy::AlwaysAllow).unwrap();
        assert!(!result.needs_approval);
    }

    #[test]
    fn widening_covers_project_files_and_always_allow() {
        let base = definition();
        let mut project = base.clone();
        project.project_id = Some("p".into());
        assert!(widens(&base, &project));
        let mut files = project.clone();
        files.files = wire::WorkflowFiles::Worktree;
        assert!(widens(&project, &files));
        let mut always = base.clone();
        always.always_allow.push("mcp__claude_ai_Slack__slack_send_message".into());
        assert!(widens(&base, &always));
        let mut reworded = base.clone();
        reworded.steps[0].instructions = "Say it shorter.".into();
        reworded.steps[0].model = "sonnet".into();
        assert!(!widens(&base, &reworded));
    }

    #[test]
    fn approving_a_stale_revision_is_refused() {
        let (dir, db) = db();
        let first = save(&db, dir.path(), None, &definition(), CreatedBy::User).unwrap().workflow;
        let mut edited = definition();
        edited.name = "Renamed".into();
        save(&db, dir.path(), Some(&first.id), &edited, CreatedBy::User).unwrap();
        let error = approve(&db, &first.id, &first.revision_id).unwrap_err().to_string();
        assert!(error.contains("changed since"), "{error}");
    }

    #[test]
    fn a_no_project_home_is_created_once_and_removed_on_delete() {
        let (dir, db) = db();
        let saved = save(&db, dir.path(), None, &definition(), CreatedBy::User).unwrap().workflow;
        let home = workflow_home(dir.path(), &saved.id);
        assert!(home.is_dir());
        std::fs::write(home.join("notes.md"), "kept").unwrap();
        save(&db, dir.path(), Some(&saved.id), &definition(), CreatedBy::User).unwrap();
        assert!(home.join("notes.md").exists(), "a second save keeps the same folder");
        let homes = std::fs::read_dir(dir.path().join("workflows")).unwrap().count();
        assert_eq!(homes, 1);
        delete(&db, dir.path(), &saved.id).unwrap();
        assert!(!home.exists());
        assert!(get(&db, &saved.id).unwrap().is_none());
    }

    #[test]
    fn a_no_project_workflow_never_creates_a_project_row() {
        let (dir, db) = db();
        let before: i64 = db.query_row("SELECT COUNT(*) FROM projects", [], |row| row.get(0)).unwrap();
        let saved = save(&db, dir.path(), None, &definition(), CreatedBy::User).unwrap().workflow;
        save(&db, dir.path(), Some(&saved.id), &definition(), CreatedBy::User).unwrap();
        let after: i64 = db.query_row("SELECT COUNT(*) FROM projects", [], |row| row.get(0)).unwrap();
        assert_eq!(before, after);
    }

    #[test]
    fn a_workflow_whose_project_folder_is_gone_is_disabled_with_a_reason() {
        let (dir, db) = db();
        let project = add_project(&db, dir.path());
        let mut bound = definition();
        bound.project_id = Some(project);
        let saved = save(&db, dir.path(), None, &bound, CreatedBy::User).unwrap().workflow;
        assert!(reconcile_projects(&db).unwrap().is_empty());

        std::fs::remove_dir_all(dir.path().join("repo")).unwrap();
        assert_eq!(reconcile_projects(&db).unwrap(), vec![saved.id.clone()]);
        let disabled = require(&db, &saved.id).unwrap();
        assert!(!disabled.enabled);
        assert_eq!(disabled.disabled_reason.as_deref(), Some(DISABLED_PROJECT_REMOVED));
        assert!(reconcile_projects(&db).unwrap().is_empty(), "already disabled is not changed again");
    }

    #[test]
    fn re_enabling_a_project_removed_workflow_is_refused() {
        let (dir, db) = db();
        let project = add_project(&db, dir.path());
        let mut bound = definition();
        bound.project_id = Some(project);
        let saved = save(&db, dir.path(), None, &bound, CreatedBy::User).unwrap().workflow;
        std::fs::remove_dir_all(dir.path().join("repo")).unwrap();
        reconcile_projects(&db).unwrap();
        assert!(set_enabled(&db, &saved.id, true).unwrap_err().to_string().contains("project is gone"));

        std::fs::create_dir_all(dir.path().join("repo")).unwrap();
        let enabled = set_enabled(&db, &saved.id, true).unwrap();
        assert!(enabled.enabled);
        assert_eq!(enabled.disabled_reason, None);
    }

    #[test]
    fn each_invalid_definition_is_refused_with_its_reason() {
        let ok = definition();
        assert_eq!(validate(&ok, &context()), Ok(()));
        let cases: Vec<(&str, Box<dyn Fn(&mut wire::WorkflowDefinition)>)> = vec![
            ("needs a name", Box::new(|d| d.name = "  ".into())),
            ("at most 80", Box::new(|d| d.name = "x".repeat(81))),
            ("exactly one step", Box::new(|d| { let step = d.steps[0].clone(); d.steps.push(step); })),
            ("exactly one step", Box::new(|d| d.steps.clear())),
            ("not a registered agent", Box::new(|d| d.steps[0].harness = wire::HarnessId::parse("nope").unwrap())),
            ("Choose a model", Box::new(|d| d.steps[0].model = " ".into())),
            ("not a model", Box::new(|d| d.steps[0].model = "opus-9".into())),
            ("what to do", Box::new(|d| d.steps[0].instructions = "".into())),
            ("needs a project", Box::new(|d| d.files = wire::WorkflowFiles::Worktree)),
            ("cannot run this workflow", Box::new(|d| d.steps[0].harness = wire::HarnessId::parse("codex").unwrap())),
            ("cannot run this workflow", Box::new(|d| d.steps[0].harness = wire::HarnessId::parse("opencode").unwrap())),
            ("five", Box::new(|d| d.trigger = wire::WorkflowTrigger::Schedule { cron: "0 9 * *".into(), timezone: "UTC".into() })),
            ("15 minutes", Box::new(|d| d.trigger = wire::WorkflowTrigger::Schedule { cron: "*/5 * * * *".into(), timezone: "UTC".into() })),
            ("IANA", Box::new(|d| d.trigger = wire::WorkflowTrigger::Schedule { cron: "0 9 * * *".into(), timezone: "Nowhere".into() })),
            ("time limit", Box::new(|d| d.budget.max_wall_seconds = 10)),
            ("turn limit", Box::new(|d| d.budget.max_turns = 0)),
            ("tool-call limit", Box::new(|d| d.budget.max_tool_calls = 1000)),
            ("cost ceiling", Box::new(|d| d.budget.cost_ceiling_microusd = Some(0))),
            ("granted twice", Box::new(|d| { let grant = d.grants[0].clone(); d.grants.push(grant); })),
            ("allows nothing", Box::new(|d| d.grants[0].read = false)),
            ("not a connector", Box::new(|d| d.grants[0].server = "claude_ai_Gmail".into())),
            ("exact tool identity", Box::new(|d| d.always_allow = vec!["slack_send_message".into()])),
            ("not granted writes", Box::new(|d| d.always_allow = vec!["mcp__claude_ai_Slack__slack_send_message".into()])),
            ("expected result", Box::new(|d| d.expected_result = "x".repeat(2001))),
        ];
        for (needle, change) in cases {
            let mut candidate = definition();
            change(&mut candidate);
            let error = validate(&candidate, &context()).unwrap_err();
            assert!(error.contains(needle), "expected `{needle}`, got `{error}`");
        }
    }

    #[test]
    fn codex_may_run_a_project_worktree_job_with_no_connectors() {
        let mut job = definition();
        job.project_id = Some("p".into());
        job.files = wire::WorkflowFiles::Worktree;
        job.grants.clear();
        job.steps[0].harness = wire::HarnessId::parse("codex").unwrap();
        job.steps[0].model = "gpt-5.5".into();
        assert_eq!(validate(&job, &context()), Ok(()));
    }

    #[test]
    fn every_mutation_publishes_workflows_changed() {
        let (_scratch, core) = test_support::core();
        let mut events = core.events.subscribe();
        let mut candidate = definition();
        // No connector grants, so validation never asks Claude's configuration.
        candidate.grants.clear();

        let saved = crate::api::save_workflow(&core, &wire::SaveWorkflowParams { workflow_id: None, definition: candidate })
            .unwrap()
            .workflow;
        let id = saved.id.clone();
        crate::api::approve_workflow(&core, &wire::ApproveWorkflowParams { workflow_id: id.clone(), revision_id: saved.revision_id })
            .unwrap();
        crate::api::set_workflow_enabled(&core, &wire::SetWorkflowEnabledParams { workflow_id: id.clone(), enabled: false }).unwrap();
        crate::api::delete_workflow(&core, &wire::DeleteWorkflowParams { workflow_id: id.clone() }).unwrap();

        let mut published = 0;
        while let Ok(event) = events.try_recv() {
            if let crate::events::CoreEvent::WorkflowsChanged { workflow_id } = event {
                assert_eq!(workflow_id, id);
                published += 1;
            }
        }
        assert_eq!(published, 4, "save, approve, disable and delete each publish once");
        // A client that lagged through all of it still learns to refetch.
        assert!(events.reconciliation_events().iter().any(|event| matches!(
            event,
            crate::events::CoreEvent::WorkflowsChanged { workflow_id } if *workflow_id == id
        )));
    }

    #[test]
    fn always_allow_matches_a_configured_server_by_its_wire_name() {
        let mut spaced = definition();
        spaced.grants = vec![wire::WorkflowGrant { server: "claude.ai Slack".into(), read: true, write: wire::WorkflowWriteAccess::Ask }];
        spaced.always_allow = vec!["mcp__claude_ai_Slack__slack_send_message".into()];
        let mut context = context();
        context.servers.insert("claude.ai Slack".into());
        assert_eq!(validate(&spaced, &context), Ok(()));
        assert_eq!(wire_server_name("claude.ai Slack"), "claude_ai_Slack");
        assert_eq!(wire_server_name("linear-prod_2"), "linear-prod_2");
    }

    #[test]
    fn tool_class_fails_closed_to_write() {
        assert_eq!(tool_class("mcp__claude_ai_Slack__slack_read_thread"), ToolClass::Read);
        assert_eq!(tool_class("mcp__claude_ai_Linear__list_issues"), ToolClass::Read);
        assert_eq!(tool_class("mcp__claude_ai_Slack__slack_send_message"), ToolClass::Write);
        // A read verb next to a mutation word is a write.
        assert_eq!(tool_class("mcp__claude_ai_Slack__get_or_create_channel"), ToolClass::Write);
        // An unknown verb, a built-in, or a malformed name is a write.
        assert_eq!(tool_class("mcp__claude_ai_Slack__slack_frobnicate"), ToolClass::Write);
        assert_eq!(tool_class("Bash"), ToolClass::Write);
        assert_eq!(tool_class("mcp__claude_ai_Slack__Slack_Read"), ToolClass::Write);
    }
}
