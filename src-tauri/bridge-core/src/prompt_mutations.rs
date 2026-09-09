//! Approved, durable append proposals for Bridge-owned shared role guidance.

use crate::{
    prompt_compiler::PromptCompiler,
    prompt_mutation_policy::{self, PromptMutationDecision},
    prompt_sections::{self, PromptRevisionAttribution, PromptSectionKey, PromptSectionState},
    prompts,
    session_forest::{self, EntryKind, SessionForest},
    store, BridgeError,
};
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;

pub const CONTROL_FENCE: &str = "bridge-prompt-change";
const MAX_GUIDANCE_BYTES: usize = 16 * 1024;
const MAX_TOTAL_BYTES: usize = 64 * 1024;
const MAX_RATIONALE_BYTES: usize = 4 * 1024;
const MAX_PENDING: i64 = 128;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PromptMutationRequest {
    pub schema_version: u32,
    pub request_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_session_id: Option<String>,
    pub guidance: String,
    pub rationale: String,
}

impl PromptMutationRequest {
    fn validate(&self) -> Result<(), BridgeError> {
        if self.schema_version != 1 {
            return invalid("Prompt change schemaVersion must be 1");
        }
        validate_identifier(&self.request_id, "requestId")?;
        if let Some(target) = &self.target_session_id {
            validate_identifier(target, "targetSessionId")?;
        }
        if self.guidance.trim().is_empty() || self.guidance.len() > MAX_GUIDANCE_BYTES {
            return invalid("Prompt guidance must be nonempty and at most 16 KiB");
        }
        if self.rationale.trim().is_empty() || self.rationale.len() > MAX_RATIONALE_BYTES {
            return invalid("Prompt change rationale must be nonempty and at most 4 KiB");
        }
        // Rationale is durable and visible as well; don't persist secret material.
        PromptCompiler::new("prompt_change")
            .stable_section("guidance", &self.guidance)
            .stable_section("rationale", &self.rationale)
            .compile()?;
        Ok(())
    }
}

fn validate_identifier(value: &str, name: &str) -> Result<(), BridgeError> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
    {
        return invalid(&format!("{name} must be a 1–128 byte identifier"));
    }
    Ok(())
}

fn invalid<T>(message: &str) -> Result<T, BridgeError> {
    Err(BridgeError::Invalid(message.into()))
}

#[derive(Debug, Clone)]
pub struct ParsedPromptMutation {
    pub request: PromptMutationRequest,
    pub visible_text: String,
}

/// Only an unquoted, top-level, exact control fence is executable. Examples
/// nested inside other markdown fences and block quotes are ordinary content.
pub fn parse_assistant_control(text: &str) -> Result<Option<ParsedPromptMutation>, BridgeError> {
    let mut ordinary_fence: Option<(u8, usize)> = None;
    let mut control_start = None;
    let mut body_start = 0;
    let mut captured: Option<(usize, usize, &str)> = None;
    let mut other_control = false;
    let mut offset = 0;
    for raw in text.split_inclusive('\n') {
        let line = raw.trim_end_matches(['\r', '\n']);
        if let Some(start) = control_start {
            if line == "```" {
                captured = Some((start, offset + raw.len(), &text[body_start..offset]));
                control_start = None;
            } else if line.trim_start().starts_with("```") || line.trim_start().starts_with("~~~") {
                return invalid("Nested prompt-change control fences are not allowed");
            }
        } else if let Some((marker, length)) = ordinary_fence {
            let trimmed = line.trim();
            if trimmed.bytes().take_while(|byte| *byte == marker).count() >= length
                && trimmed.bytes().all(|byte| byte == marker)
            {
                ordinary_fence = None;
            }
        } else if line.starts_with("```bridge-prompt-change") {
            if line != "```bridge-prompt-change" || captured.is_some() {
                return invalid("Use exactly one bridge-prompt-change control fence");
            }
            control_start = Some(offset);
            body_start = offset + raw.len();
        } else if !line.trim_start().starts_with('>') {
            let trimmed = line.trim_start();
            if [
                "```bridge-delegate",
                "```bridge-steer",
                "```bridge-stop",
                "```bridge-worker-result",
            ]
            .contains(&line)
            {
                other_control = true;
            }
            if let Some(marker @ (b'`' | b'~')) = trimmed.as_bytes().first().copied() {
                let count = trimmed.bytes().take_while(|byte| *byte == marker).count();
                if count >= 3 {
                    ordinary_fence = Some((marker, count));
                }
            }
        }
        offset += raw.len();
    }
    if control_start.is_some() {
        return invalid("Unclosed bridge-prompt-change control fence");
    }
    let Some((start, end, body)) = captured else {
        return Ok(None);
    };
    if other_control {
        return invalid("A prompt-change turn cannot contain another Bridge control action");
    }
    if body.len() > 32 * 1024 {
        return invalid("Prompt change control payload exceeds 32 KiB");
    }
    let request: PromptMutationRequest = serde_json::from_str(body)
        .map_err(|error| BridgeError::Invalid(format!("Invalid prompt change: {error}")))?;
    request.validate()?;
    Ok(Some(ParsedPromptMutation {
        request,
        visible_text: format!("{}{}", &text[..start], &text[end..])
            .trim()
            .to_owned(),
    }))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptMutationStatus {
    Pending,
    Accepted,
    Declined,
    Stale,
    Denied,
}

impl PromptMutationStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Accepted => "accepted",
            Self::Declined => "declined",
            Self::Stale => "stale",
            Self::Denied => "denied",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptMutationProposal {
    pub id: String,
    pub request_id: String,
    pub actor_session_id: String,
    pub actor_turn_id: String,
    pub actor_role: String,
    pub target_session_id: String,
    pub target: String,
    pub section_id: String,
    pub before_text: String,
    pub after_text: String,
    pub appended_text: String,
    pub rationale: String,
    pub base_revision_id: Option<i64>,
    pub base_hash: String,
    pub status: PromptMutationStatus,
    pub approval_session_id: String,
    pub approval_event_id: i64,
    pub revision_id: Option<i64>,
    pub created_at: String,
    base_state: PromptSectionState,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptMutationResolution {
    pub proposal: PromptMutationProposal,
    pub status: PromptMutationStatus,
    pub already_resolved: bool,
}

fn hash(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn key(target: &str) -> Result<PromptSectionKey, BridgeError> {
    let target = prompts::PROMPT_TARGETS
        .iter()
        .find(|item| item.storage_key() == target)
        .copied()
        .ok_or_else(|| BridgeError::Invalid("Unknown prompt change target".into()))?;
    PromptSectionKey::new(target, prompts::ADDITIONAL_GUIDANCE_SECTION_ID)
}

fn current_text(state: &PromptSectionState) -> &str {
    match state {
        PromptSectionState::Overridden { text } => text,
        _ => "",
    }
}

fn append_text(before: &str, addition: &str) -> String {
    if before.is_empty() {
        addition.to_owned()
    } else {
        format!("{before}\n\n{addition}")
    }
}

fn append_approval_entry(
    tx: &Transaction<'_>,
    session_id: &str,
    kind: EntryKind,
    mut payload: Value,
) -> Result<i64, BridgeError> {
    kind.validate_payload(&payload)
        .map_err(|error| BridgeError::Invalid(error.to_string()))?;
    let branch = SessionForest::new(tx)
        .active_branch(session_id)
        .map_err(|error| BridgeError::Invalid(error.to_string()))?;
    payload[session_forest::TYPED_SCHEMA_MARKER] = json!(session_forest::TYPED_SCHEMA_VERSION);
    let entry = store::append_session_entry_tx(
        tx,
        session_id,
        branch.last().map(|entry| entry.id.as_str()),
        kind.as_str(),
        &payload,
        None,
        "eligible",
        None,
    )?;
    Ok(entry.sequence)
}

pub fn propose(
    db: &Connection,
    actor_session_id: &str,
    actor_turn_id: &str,
    request: &PromptMutationRequest,
) -> Result<PromptMutationProposal, BridgeError> {
    request.validate()?;
    validate_identifier(actor_session_id, "actor session")?;
    validate_identifier(actor_turn_id, "actor turn")?;
    let tx = db.unchecked_transaction()?;
    // Replays are looked up before liveness checks: a completed control turn
    // may be redelivered, but its immutable request cannot acquire new power.
    if let Some(existing) = for_turn(&tx, actor_session_id, actor_turn_id)? {
        let stored: String = tx.query_row(
            "SELECT request_json FROM prompt_mutation_proposals WHERE id=?1",
            params![existing.id],
            |row| row.get(0),
        )?;
        if serde_json::from_str::<PromptMutationRequest>(&stored)
            .ok()
            .as_ref()
            == Some(request)
        {
            tx.commit()?;
            return Ok(existing);
        }
        return invalid("This actor turn already proposed a different prompt change");
    }
    let target_session_id = request
        .target_session_id
        .as_deref()
        .unwrap_or(actor_session_id);
    let authority = match prompt_mutation_policy::decide(
        &tx,
        actor_session_id,
        actor_turn_id,
        target_session_id,
        false,
    )? {
        PromptMutationDecision::RequireApproval(authority) => authority,
        PromptMutationDecision::Reject(reason) => return invalid(&reason),
    };
    let pending: i64 = tx.query_row(
        "SELECT COUNT(*) FROM prompt_mutation_proposals WHERE status='pending'",
        [],
        |row| row.get(0),
    )?;
    let actor_pending: i64 = tx.query_row("SELECT COUNT(*) FROM prompt_mutation_proposals WHERE status='pending' AND actor_session_id=?1",
        params![actor_session_id], |row| row.get(0))?;
    if pending >= MAX_PENDING || actor_pending >= 8 {
        return invalid("Too many pending prompt changes; review an existing proposal first");
    }
    let section = PromptSectionKey::new(authority.target, prompts::ADDITIONAL_GUIDANCE_SECTION_ID)?;
    let base_state = prompt_sections::current_state(&tx, &section)?;
    let before_text = current_text(&base_state).to_owned();
    let after_text = append_text(&before_text, &request.guidance);
    if after_text.len() > MAX_TOTAL_BYTES {
        return invalid("Combined role guidance exceeds 64 KiB");
    }
    PromptCompiler::new(authority.target.compiler_role())
        .stable_section(&section.section_id, &after_text)
        .compile()?;
    let mut proposal = PromptMutationProposal {
        id: Uuid::new_v4().to_string(),
        request_id: request.request_id.clone(),
        actor_session_id: actor_session_id.into(),
        actor_turn_id: actor_turn_id.into(),
        actor_role: authority.actor_role,
        target_session_id: target_session_id.into(),
        target: authority.target.storage_key().into(),
        section_id: section.section_id.clone(),
        base_revision_id: prompt_sections::latest_revision_id(&tx, &section)?,
        base_hash: hash(&before_text),
        before_text,
        after_text,
        appended_text: request.guidance.clone(),
        rationale: request.rationale.clone(),
        status: PromptMutationStatus::Pending,
        approval_session_id: actor_session_id.into(),
        approval_event_id: 0,
        revision_id: None,
        created_at: Utc::now().to_rfc3339(),
        base_state,
    };
    proposal.approval_event_id = append_approval_entry(
        &tx,
        actor_session_id,
        EntryKind::ApprovalRequested,
        approval_payload(&proposal),
    )?;
    tx.execute("INSERT INTO prompt_mutation_proposals(id,actor_session_id,actor_turn_id,request_id,request_json,payload,status,created_at)
        VALUES(?1,?2,?3,?4,?5,?6,'pending',?7)",
        params![proposal.id, actor_session_id, actor_turn_id, request.request_id,
            serialize(request)?, serialize(&proposal)?, proposal.created_at])?;
    tx.commit()?;
    Ok(proposal)
}

fn serialize(value: &impl Serialize) -> Result<String, BridgeError> {
    serde_json::to_string(value).map_err(|error| BridgeError::Invalid(error.to_string()))
}

pub fn approval_payload(proposal: &PromptMutationProposal) -> Value {
    json!({
        "approvalType":"prompt_mutation", "approvalId":proposal.id, "proposalId":proposal.id,
        "requestId":proposal.request_id, "turnId":proposal.actor_turn_id,
        "title":"Approve shared role prompt change", "status":"pending",
        "text":"Append guidance to shared role defaults. Applies on the next matching launch; running turns retain their instructions.",
        "target":proposal.target, "targetSessionId":proposal.target_session_id,
        "sectionId":proposal.section_id, "operation":"append", "beforeText":proposal.before_text,
        "afterText":proposal.after_text, "appendedText":proposal.appended_text, "rationale":proposal.rationale,
        "actorSessionId":proposal.actor_session_id, "actorTurnId":proposal.actor_turn_id,
        "actorRole":proposal.actor_role, "baseRevisionId":proposal.base_revision_id,
        "baseHash":proposal.base_hash, "effect":"next_launch",
    })
}

pub fn get(
    db: &Connection,
    proposal_id: &str,
) -> Result<Option<PromptMutationProposal>, BridgeError> {
    let row: Option<(String, String, Option<i64>)> = db
        .query_row(
            "SELECT payload,status,revision_id FROM prompt_mutation_proposals WHERE id=?1",
            params![proposal_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    row.map(|(payload, status, revision_id)| {
        let mut proposal: PromptMutationProposal = serde_json::from_str(&payload)
            .map_err(|error| BridgeError::Invalid(format!("Invalid prompt proposal: {error}")))?;
        proposal.status = serde_json::from_value(json!(status)).map_err(|error| {
            BridgeError::Invalid(format!("Invalid prompt proposal status: {error}"))
        })?;
        proposal.revision_id = revision_id;
        Ok(proposal)
    })
    .transpose()
}

pub fn for_turn(
    db: &Connection,
    actor_session_id: &str,
    actor_turn_id: &str,
) -> Result<Option<PromptMutationProposal>, BridgeError> {
    let id: Option<String> = db.query_row(
        "SELECT id FROM prompt_mutation_proposals WHERE actor_session_id=?1 AND actor_turn_id=?2",
        params![actor_session_id, actor_turn_id], |row| row.get(0)).optional()?;
    id.map(|id| get(db, &id)).transpose().map(Option::flatten)
}

pub fn pending_for_session(
    db: &Connection,
    actor_session_id: &str,
) -> Result<Vec<PromptMutationProposal>, BridgeError> {
    let ids = db.prepare("SELECT id FROM prompt_mutation_proposals WHERE actor_session_id=?1 AND status='pending' ORDER BY created_at,id")?
        .query_map(params![actor_session_id], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    ids.into_iter()
        .map(|id| {
            get(db, &id)?.ok_or_else(|| BridgeError::Invalid("Prompt proposal disappeared".into()))
        })
        .collect()
}

pub fn resolve(
    db: &Connection,
    proposal_id: &str,
    accepted: bool,
) -> Result<PromptMutationResolution, BridgeError> {
    let tx = db.unchecked_transaction()?;
    let mut proposal = get(&tx, proposal_id)?
        .ok_or_else(|| BridgeError::Invalid("Prompt proposal does not exist".into()))?;
    if proposal.status != PromptMutationStatus::Pending {
        let status = proposal.status;
        tx.commit()?;
        return Ok(PromptMutationResolution {
            proposal,
            status,
            already_resolved: true,
        });
    }
    let section = key(&proposal.target)?;
    let request_is_current = SessionForest::new(&tx)
        .active_branch(&proposal.approval_session_id)
        .map_err(|error| BridgeError::Invalid(error.to_string()))?
        .iter()
        .any(|entry| {
            entry.sequence == proposal.approval_event_id
                && entry.kind == "approval.requested"
                && entry.payload["proposalId"] == proposal.id
                && entry.payload["approvalType"] == "prompt_mutation"
        });
    let current = prompt_sections::current_state(&tx, &section)?;
    let authority = prompt_mutation_policy::decide(
        &tx,
        &proposal.actor_session_id,
        &proposal.actor_turn_id,
        &proposal.target_session_id,
        true,
    )?;
    let authorized = matches!(authority, PromptMutationDecision::RequireApproval(ref allowed)
        if allowed.actor_role == proposal.actor_role && allowed.target == section.target);
    let status = if !accepted {
        PromptMutationStatus::Declined
    } else if !authorized || !request_is_current {
        PromptMutationStatus::Denied
    } else if current != proposal.base_state
        || prompt_sections::latest_revision_id(&tx, &section)? != proposal.base_revision_id
        || hash(current_text(&current)) != proposal.base_hash
    {
        PromptMutationStatus::Stale
    } else {
        let attribution = PromptRevisionAttribution {
            actor_session_id: proposal.actor_session_id.clone(),
            actor_turn_id: proposal.actor_turn_id.clone(),
            actor_role: proposal.actor_role.clone(),
            proposal_id: proposal.id.clone(),
            rationale: proposal.rationale.clone(),
        };
        let revision = prompt_sections::save_attributed_override_tx(
            &tx,
            &section,
            proposal.after_text.clone(),
            &attribution,
        )?;
        proposal.revision_id = Some(revision.id);
        PromptMutationStatus::Accepted
    };
    proposal.status = status;
    tx.execute("UPDATE prompt_mutation_proposals SET status=?2,revision_id=?3,resolved_at=?4 WHERE id=?1 AND status='pending'",
        params![proposal_id, status.as_str(), proposal.revision_id, Utc::now().to_rfc3339()])?;
    let reason = match status {
        PromptMutationStatus::Accepted => "Prompt change approved. It applies on the next matching launch; running turns retain their instructions.",
        PromptMutationStatus::Declined => "Prompt change declined. Guidance was not changed.",
        PromptMutationStatus::Stale => "Prompt change is stale because shared guidance changed. Submit a fresh proposal for review.",
        _ if !request_is_current => "Prompt change denied because its approval request is no longer on the active conversation branch.",
        _ => "Prompt change denied because the actor's authority or target relationship changed.",
    };
    let payload = resolution_payload(&proposal, reason);
    append_approval_entry(
        &tx,
        &proposal.approval_session_id,
        EntryKind::ApprovalResolved,
        payload,
    )?;
    tx.commit()?;
    Ok(PromptMutationResolution {
        proposal,
        status,
        already_resolved: false,
    })
}

fn resolution_payload(proposal: &PromptMutationProposal, reason: &str) -> Value {
    let status = proposal.status;
    let mut payload = approval_payload(proposal);
    payload["status"] = json!(status.as_str());
    payload["decision"] = json!(match status {
        PromptMutationStatus::Accepted => "accept",
        PromptMutationStatus::Declined => "decline",
        _ => status.as_str(),
    });
    payload["requestEventId"] = json!(proposal.approval_event_id);
    payload["revisionId"] = json!(proposal.revision_id);
    payload["text"] = json!(reason);
    payload["reason"] = json!(reason);
    payload
}

/// Settle before workspace deletion removes its approval cards and sessions.
/// The immutable proposal plus durable event remain after forest cleanup, and
/// archived requests cannot permanently consume the pending-proposal budget.
pub(crate) fn deny_pending_for_workspace_tx(
    tx: &Transaction<'_>,
    workspace_id: &str,
) -> Result<(), BridgeError> {
    let ids = tx.prepare("SELECT id FROM prompt_mutation_proposals WHERE status='pending'
        AND (actor_session_id IN (SELECT id FROM sessions WHERE workspace_id=?1)
            OR json_extract(payload,'$.targetSessionId') IN (SELECT id FROM sessions WHERE workspace_id=?1))")?
        .query_map(params![workspace_id], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    for id in ids {
        let mut proposal = get(tx, &id)?
            .ok_or_else(|| BridgeError::Invalid("Prompt proposal disappeared".into()))?;
        proposal.status = PromptMutationStatus::Denied;
        let reason = "Prompt change denied because its actor or target workspace was archived.";
        tx.execute("UPDATE prompt_mutation_proposals SET status='denied',resolved_at=?2 WHERE id=?1 AND status='pending'",
            params![id, Utc::now().to_rfc3339()])?;
        let payload = resolution_payload(&proposal, reason);
        append_approval_entry(
            tx,
            &proposal.approval_session_id,
            EntryKind::ApprovalResolved,
            payload.clone(),
        )?;
        store::event(
            tx,
            "prompt_mutation",
            "prompt_mutation.archived",
            &proposal.actor_session_id,
            &serialize(&payload)?,
        )?;
    }
    Ok(())
}

pub(crate) fn install_store(tx: &Transaction<'_>) -> Result<(), BridgeError> {
    tx.execute_batch("CREATE TABLE IF NOT EXISTS prompt_mutation_proposals (
        id TEXT PRIMARY KEY, actor_session_id TEXT NOT NULL, actor_turn_id TEXT NOT NULL,
        request_id TEXT NOT NULL, request_json TEXT NOT NULL, payload TEXT NOT NULL,
        status TEXT NOT NULL CHECK(status IN ('pending','accepted','declined','stale','denied')),
        revision_id INTEGER REFERENCES prompt_section_revisions(id), created_at TEXT NOT NULL, resolved_at TEXT,
        UNIQUE(actor_session_id,actor_turn_id),
        CHECK((status='pending' AND resolved_at IS NULL AND revision_id IS NULL)
            OR (status='accepted' AND resolved_at IS NOT NULL AND revision_id IS NOT NULL)
            OR (status IN ('declined','stale','denied') AND resolved_at IS NOT NULL AND revision_id IS NULL))
    );
    CREATE INDEX IF NOT EXISTS idx_prompt_mutation_pending ON prompt_mutation_proposals(actor_session_id,status);
    CREATE INDEX IF NOT EXISTS idx_prompt_mutation_receipts ON events(entity_id,kind,body)
        WHERE kind='prompt_mutation.feedback.queued' OR kind='prompt_mutation.control_turn'
            OR kind='prompt_mutation.parent_notice.queued'
            OR kind='prompt_mutation.parent_notice.mirrored';
    CREATE TRIGGER IF NOT EXISTS prompt_mutation_proposals_immutable_request BEFORE UPDATE ON prompt_mutation_proposals
        WHEN NEW.id != OLD.id OR NEW.actor_session_id != OLD.actor_session_id OR NEW.actor_turn_id != OLD.actor_turn_id
        OR NEW.request_id != OLD.request_id OR NEW.request_json != OLD.request_json OR NEW.payload != OLD.payload
        OR NEW.created_at != OLD.created_at OR OLD.status != 'pending'
        BEGIN SELECT RAISE(ABORT,'prompt proposal request and resolution are immutable'); END;
    CREATE TRIGGER IF NOT EXISTS prompt_mutation_proposals_no_delete BEFORE DELETE ON prompt_mutation_proposals
        BEGIN SELECT RAISE(ABORT,'prompt proposals are durable audit records'); END;
    CREATE TRIGGER IF NOT EXISTS prompt_mutation_proposals_no_replace BEFORE INSERT ON prompt_mutation_proposals
        WHEN EXISTS(SELECT 1 FROM prompt_mutation_proposals WHERE id=NEW.id
            OR (actor_session_id=NEW.actor_session_id AND actor_turn_id=NEW.actor_turn_id))
        BEGIN SELECT RAISE(ABORT,'prompt proposals are durable audit records'); END;")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{agent_config, delegation::WorkerRole, prompt_compiler};
    use std::path::Path;

    fn database() -> Connection {
        let db = store::open(Path::new(":memory:")).unwrap();
        db.execute_batch("INSERT INTO projects(id,name,path,created_at) VALUES('p','Project','/tmp/bridge-prompt-mutation-tests','now');
            INSERT INTO workspaces(id,project_id,city,title,branch,path,status,created_at) VALUES('w','p','Local','Test','main','/tmp/bridge-prompt-mutation-tests','ready','now');
            INSERT INTO sessions(id,harness,label,status,kind,active_turn_id,depth)
            VALUES('actor','codex','Orchestrator','working','orchestrator','turn',0);
            INSERT INTO sessions(id,harness,label,status,kind,active_turn_id,depth,parent_session_id)
            VALUES('worker','codex','Worker','working','workspace','worker-turn',1,'actor');
            INSERT INTO worker_runtime(session_id,parent_session_id,lifecycle_state,task_family,compatibility_key,result_status,updated_at)
            VALUES('worker','actor','working','implementation','key','pending','now');
            INSERT INTO worker_leases(session_id,workspace_id,role,capability_tier,write_mode,lease_status,created_at,updated_at)
            VALUES('worker','w','implementation','standard','isolated','active','now','now');").unwrap();
        db
    }

    fn request() -> PromptMutationRequest {
        PromptMutationRequest {
            schema_version: 1,
            request_id: "request-1".into(),
            target_session_id: None,
            guidance: "Keep conclusions tied to observed evidence.".into(),
            rationale: "The earlier result omitted verification evidence.".into(),
        }
    }

    fn grant_worker(db: &Connection) {
        let mut policy = agent_config::permission_policy(db).unwrap();
        policy.worker_prompt_proposal_roles = vec![WorkerRole::Implementation];
        agent_config::save_permission_policy(db, policy).unwrap();
    }

    fn fence(request: &PromptMutationRequest) -> String {
        format!(
            "Before\n```bridge-prompt-change\n{}\n```\nAfter",
            serde_json::to_string(request).unwrap()
        )
    }

    #[test]
    fn parser_accepts_one_strict_control_and_removes_only_that_fence() {
        let parsed = parse_assistant_control(&fence(&request()))
            .unwrap()
            .unwrap();
        assert_eq!(parsed.request, request());
        assert_eq!(parsed.visible_text, "Before\nAfter");
        assert!(
            parse_assistant_control(&format!("````markdown\n{}\n````", fence(&request())))
                .unwrap()
                .is_none()
        );
        assert!(parse_assistant_control(
            &fence(&request())
                .lines()
                .map(|line| format!("> {line}"))
                .collect::<Vec<_>>()
                .join("\n")
        )
        .unwrap()
        .is_none());
        assert!(parse_assistant_control("A normal response")
            .unwrap()
            .is_none());
    }

    #[test]
    fn parser_rejects_malformed_duplicate_nested_and_spoofed_controls() {
        let valid = fence(&request());
        for malformed in [
            format!("{valid}\n{valid}"),
            "```bridge-prompt-change\n{}".into(),
            "```bridge-prompt-change extra\n{}\n```".into(),
            "```bridge-prompt-change\n```json\n{}\n```\n```".into(),
            valid.replace("\"schemaVersion\":1", "\"schemaVersion\":2"),
            valid.replace(
                "\"requestId\":",
                "\"actorSessionId\":\"admin\",\"requestId\":",
            ),
        ] {
            assert!(parse_assistant_control(&malformed).is_err(), "{malformed}");
        }
    }

    #[test]
    fn parser_rejects_prompt_changes_mixed_with_stop_actions() {
        let stop = "```bridge-stop\n{\"sessionId\":\"worker\"}\n```";
        let change = fence(&request());
        for text in [format!("{stop}\n{change}"), format!("{change}\n{stop}")] {
            assert!(parse_assistant_control(&text).is_err());
        }
    }

    #[test]
    fn empty_guidance_preserves_compiled_bytes_and_defaults_remain_dynamic() {
        let db = database();
        for target in prompts::PROMPT_TARGETS {
            let stack = prompt_sections::resolve(&db, *target, 0).unwrap();
            assert!(!stack
                .sections
                .iter()
                .any(|section| section.id == prompts::ADDITIONAL_GUIDANCE_SECTION_ID));
            let mut legacy = PromptCompiler::new(target.compiler_role());
            for section in prompts::default_sections(*target, 0) {
                if section.id != prompts::ADDITIONAL_GUIDANCE_SECTION_ID {
                    legacy = legacy.stable_section(section.id, section.text);
                }
            }
            assert_eq!(
                prompt_compiler::compiler_for_resolved_stack(&stack)
                    .unwrap()
                    .compile()
                    .unwrap(),
                legacy.compile().unwrap()
            );
        }
        let mut worker_request = request();
        worker_request.target_session_id = Some("worker".into());
        let proposal = propose(&db, "actor", "turn", &worker_request).unwrap();
        resolve(&db, &proposal.id, true).unwrap();
        for depth in [0, 1] {
            let stack = prompt_sections::resolve(
                &db,
                prompts::PromptTarget::Worker(WorkerRole::Implementation),
                depth,
            )
            .unwrap();
            let contract = stack
                .sections
                .iter()
                .find(|section| section.id == prompts::WORKER_CONTRACT_SECTION_ID)
                .unwrap();
            assert_eq!(
                contract.text,
                crate::delegation::worker_contract(WorkerRole::Implementation, depth)
            );
            assert_eq!(
                stack
                    .sections
                    .iter()
                    .find(|section| section.id == prompts::ADDITIONAL_GUIDANCE_SECTION_ID)
                    .unwrap()
                    .text,
                worker_request.guidance
            );
        }
    }

    #[test]
    fn approval_appends_exact_bytes_and_immutable_attribution_with_reversible_history() {
        let db = database();
        let section = key("orchestrator").unwrap();
        let original =
            prompt_sections::save_override(&db, &section, "  Existing guidance\r\n").unwrap();
        let proposal = propose(&db, "actor", "turn", &request()).unwrap();
        assert_eq!(proposal.before_text, "  Existing guidance\r\n");
        assert_eq!(
            proposal.after_text,
            format!("  Existing guidance\r\n\n\n{}", request().guidance)
        );
        assert_eq!(
            prompt_sections::current_state(&db, &section).unwrap(),
            original.state
        );
        let event = store::session_entries(&db, "actor").unwrap().pop().unwrap();
        assert_eq!(event.payload["beforeText"], proposal.before_text);
        assert_eq!(event.payload["afterText"], proposal.after_text);
        let resolved = resolve(&db, &proposal.id, true).unwrap();
        assert_eq!(resolved.status, PromptMutationStatus::Accepted);
        let history = prompt_sections::revisions(&db, &section).unwrap();
        let attribution = history.last().unwrap().attribution.as_ref().unwrap();
        assert_eq!(attribution.actor_session_id, "actor");
        assert_eq!(attribution.actor_turn_id, "turn");
        assert_eq!(attribution.actor_role, "orchestrator");
        assert_eq!(attribution.proposal_id, proposal.id);
        assert_eq!(attribution.rationale, request().rationale);
        assert!(db
            .execute(
                "UPDATE prompt_section_revisions SET attribution='{}' WHERE id=?1",
                params![resolved.proposal.revision_id]
            )
            .is_err());
        prompt_sections::restore_revision(&db, &section, original.id).unwrap();
        assert_eq!(
            prompt_sections::current_state(&db, &section).unwrap(),
            original.state
        );
        assert_eq!(prompt_sections::revisions(&db, &section).unwrap().len(), 3);
        assert_eq!(
            prompt_sections::revisions(&db, &section).unwrap()[1]
                .attribution
                .as_ref(),
            Some(attribution)
        );
    }

    #[test]
    fn proposal_and_resolution_are_idempotent_and_conflicting_replay_is_rejected() {
        let db = database();
        let proposal = propose(&db, "actor", "turn", &request()).unwrap();
        assert_eq!(propose(&db, "actor", "turn", &request()).unwrap(), proposal);
        assert_eq!(store::session_entries(&db, "actor").unwrap().len(), 1);
        let mut conflicting = request();
        conflicting.guidance = "Different guidance".into();
        assert!(propose(&db, "actor", "turn", &conflicting).is_err());
        resolve(&db, &proposal.id, true).unwrap();
        let repeated = resolve(&db, &proposal.id, false).unwrap();
        assert!(repeated.already_resolved);
        assert_eq!(repeated.status, PromptMutationStatus::Accepted);
        assert_eq!(store::session_entries(&db, "actor").unwrap().len(), 2);
        assert_eq!(
            prompt_sections::revisions(&db, &key("orchestrator").unwrap())
                .unwrap()
                .len(),
            1
        );
        assert!(pending_for_session(&db, "actor").unwrap().is_empty());
        assert_eq!(
            for_turn(&db, "actor", "turn").unwrap().unwrap().status,
            PromptMutationStatus::Accepted
        );
    }

    #[test]
    fn reset_restore_aba_and_simultaneous_proposals_cannot_overwrite_newer_work() {
        let db = database();
        let section = key("orchestrator").unwrap();
        let proposal = propose(&db, "actor", "turn", &request()).unwrap();
        prompt_sections::save_override(&db, &section, "Intervening edit").unwrap();
        prompt_sections::reset_section(&db, &section).unwrap();
        assert_eq!(
            resolve(&db, &proposal.id, true).unwrap().status,
            PromptMutationStatus::Stale
        );
        assert_eq!(
            prompt_sections::current_state(&db, &section).unwrap(),
            PromptSectionState::Default
        );
        db.execute(
            "UPDATE sessions SET active_turn_id='turn-2' WHERE id='actor'",
            [],
        )
        .unwrap();
        let first = propose(&db, "actor", "turn-2", &request()).unwrap();
        db.execute(
            "UPDATE sessions SET active_turn_id='turn-3' WHERE id='actor'",
            [],
        )
        .unwrap();
        let second = propose(&db, "actor", "turn-3", &request()).unwrap();
        assert_eq!(
            resolve(&db, &first.id, true).unwrap().status,
            PromptMutationStatus::Accepted
        );
        assert_eq!(
            resolve(&db, &second.id, true).unwrap().status,
            PromptMutationStatus::Stale
        );
        assert_eq!(
            current_text(&prompt_sections::current_state(&db, &section).unwrap()),
            request().guidance
        );
    }

    #[test]
    fn decline_and_revoked_worker_grant_do_not_change_guidance() {
        let db = database();
        assert!(propose(&db, "worker", "worker-turn", &request()).is_err());
        grant_worker(&db);
        let worker = propose(&db, "worker", "worker-turn", &request()).unwrap();
        assert_eq!(worker.target, "worker:implementation");
        let mut policy = agent_config::permission_policy(&db).unwrap();
        policy.worker_prompt_proposal_roles.clear();
        policy.auto_approve_provider_permissions = true;
        agent_config::save_permission_policy(&db, policy).unwrap();
        assert_eq!(
            resolve(&db, &worker.id, true).unwrap().status,
            PromptMutationStatus::Denied
        );
        let own = propose(&db, "actor", "turn", &request()).unwrap();
        assert_eq!(
            resolve(&db, &own.id, false).unwrap().status,
            PromptMutationStatus::Declined
        );
        assert!(
            prompt_sections::revisions(&db, &key("orchestrator").unwrap())
                .unwrap()
                .is_empty()
        );
        assert!(
            prompt_sections::revisions(&db, &key("worker:implementation").unwrap())
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn review_may_outlive_actor_but_completed_actor_cannot_create_proposal() {
        let db = database();
        grant_worker(&db);
        let proposal = propose(&db, "worker", "worker-turn", &request()).unwrap();
        db.execute_batch("UPDATE sessions SET status='completed',active_turn_id=NULL WHERE id='worker';
            UPDATE worker_runtime SET lifecycle_state='stopped',result_status='completed' WHERE session_id='worker';").unwrap();
        assert_eq!(
            resolve(&db, &proposal.id, true).unwrap().status,
            PromptMutationStatus::Accepted
        );
        assert!(propose(&db, "worker", "later-turn", &request()).is_err());
        assert_eq!(
            propose(&db, "worker", "worker-turn", &request())
                .unwrap()
                .status,
            PromptMutationStatus::Accepted
        );
    }

    #[test]
    fn policy_rejects_spoofed_turn_direct_hidden_unrelated_and_worker_cross_target() {
        let db = database();
        assert!(propose(&db, "actor", "forged-turn", &request()).is_err());
        for kind in ["direct", "outcome_evaluation", "maintenance"] {
            db.execute(
                "UPDATE sessions SET kind=?1 WHERE id='actor'",
                params![kind],
            )
            .unwrap();
            assert!(propose(&db, "actor", "turn", &request()).is_err());
        }
        db.execute(
            "UPDATE sessions SET kind='orchestrator' WHERE id='actor'",
            [],
        )
        .unwrap();
        grant_worker(&db);
        let mut cross = request();
        cross.target_session_id = Some("actor".into());
        assert!(propose(&db, "worker", "worker-turn", &cross).is_err());
        db.execute_batch("INSERT INTO sessions(id,harness,label,status,kind,active_turn_id) VALUES('other','codex','Other','working','orchestrator','other-turn');
            UPDATE sessions SET parent_session_id='other' WHERE id='worker';
            UPDATE worker_runtime SET parent_session_id='other' WHERE session_id='worker';").unwrap();
        cross.target_session_id = Some("worker".into());
        assert!(propose(&db, "actor", "turn", &cross).is_err());
        assert!(pending_for_session(&db, "actor").unwrap().is_empty());
    }

    #[test]
    fn invalid_or_oversized_content_leaves_no_proposal_or_approval() {
        let db = database();
        for guidance in [
            String::new(),
            "x".repeat(MAX_GUIDANCE_BYTES + 1),
            "Use [secret:password]".into(),
        ] {
            let mut req = request();
            req.guidance = guidance;
            assert!(propose(&db, "actor", "turn", &req).is_err());
        }
        let section = key("orchestrator").unwrap();
        prompt_sections::save_override(&db, &section, "x".repeat(MAX_TOTAL_BYTES)).unwrap();
        assert!(propose(&db, "actor", "turn", &request()).is_err());
        assert!(pending_for_session(&db, "actor").unwrap().is_empty());
        assert!(store::session_entries(&db, "actor").unwrap().is_empty());
    }

    #[test]
    fn failures_roll_back_proposal_approval_prompt_revision_and_resolution_together() {
        let db = database();
        db.execute_batch("CREATE TRIGGER reject_proposal BEFORE INSERT ON prompt_mutation_proposals BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
        assert!(propose(&db, "actor", "turn", &request()).is_err());
        assert!(store::session_entries(&db, "actor").unwrap().is_empty());
        db.execute_batch("DROP TRIGGER reject_proposal;").unwrap();
        let proposal = propose(&db, "actor", "turn", &request()).unwrap();
        db.execute_batch("CREATE TRIGGER reject_resolution BEFORE INSERT ON session_entries WHEN NEW.kind='approval.resolved' BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
        assert!(resolve(&db, &proposal.id, true).is_err());
        assert_eq!(
            get(&db, &proposal.id).unwrap().unwrap().status,
            PromptMutationStatus::Pending
        );
        assert_eq!(
            prompt_sections::current_state(&db, &key("orchestrator").unwrap()).unwrap(),
            PromptSectionState::Default
        );
        assert!(
            prompt_sections::revisions(&db, &key("orchestrator").unwrap())
                .unwrap()
                .is_empty()
        );
        assert_eq!(store::session_entries(&db, "actor").unwrap().len(), 1);
    }

    #[test]
    fn an_approval_removed_from_the_active_branch_is_denied_without_leaking_pending_budget() {
        let db = database();
        let proposal = propose(&db, "actor", "turn", &request()).unwrap();
        SessionForest::new(&db).move_head("actor", None).unwrap();
        assert_eq!(
            resolve(&db, &proposal.id, true).unwrap().status,
            PromptMutationStatus::Denied
        );
        assert!(pending_for_session(&db, "actor").unwrap().is_empty());
        assert!(
            prompt_sections::revisions(&db, &key("orchestrator").unwrap())
                .unwrap()
                .is_empty()
        );
        assert!(store::session_entries(&db, "actor")
            .unwrap()
            .last()
            .unwrap()
            .payload["reason"]
            .as_str()
            .unwrap()
            .contains("active conversation branch"));
    }

    #[test]
    fn archival_settles_pending_proposals_atomically_and_preserves_the_audit() {
        let db = database();
        db.execute("UPDATE sessions SET workspace_id='w'", [])
            .unwrap();
        let proposal = propose(&db, "actor", "turn", &request()).unwrap();
        assert!(
            crate::workspaces::archive_workspace_records(&db, "w", 0, || invalid(
                "injected removal failure"
            ))
            .is_err()
        );
        assert_eq!(
            get(&db, &proposal.id).unwrap().unwrap().status,
            PromptMutationStatus::Pending
        );
        assert_eq!(store::session_entries(&db, "actor").unwrap().len(), 1);
        crate::workspaces::archive_workspace_records(&db, "w", 0, || Ok(())).unwrap();
        let stored = get(&db, &proposal.id).unwrap().unwrap();
        assert_eq!(stored.status, PromptMutationStatus::Denied);
        assert_eq!(stored.before_text, proposal.before_text);
        assert_eq!(stored.after_text, proposal.after_text);
        let pending: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM prompt_mutation_proposals WHERE status='pending'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(pending, 0);
        let audit: String = db.query_row("SELECT body FROM events WHERE kind='prompt_mutation.archived' AND entity_id='actor'", [], |row| row.get(0)).unwrap();
        assert!(audit.contains(&proposal.id));
        assert!(audit.contains("workspace was archived"));
    }

    #[test]
    fn conflicting_replace_cannot_erase_a_proposal_with_a_different_id() {
        let db = database();
        let proposal = propose(&db, "actor", "turn", &request()).unwrap();
        assert!(db.execute("INSERT OR REPLACE INTO prompt_mutation_proposals(id,actor_session_id,actor_turn_id,request_id,request_json,payload,status,created_at)
            SELECT 'replacement',actor_session_id,actor_turn_id,request_id,request_json,payload,status,created_at FROM prompt_mutation_proposals WHERE id=?1", params![proposal.id]).is_err());
        assert_eq!(get(&db, &proposal.id).unwrap(), Some(proposal));
    }

    #[test]
    fn migration_replay_preserves_attributed_history_and_proposal_foreign_keys() {
        let db = database();
        let proposal = propose(&db, "actor", "turn", &request()).unwrap();
        let accepted = resolve(&db, &proposal.id, true).unwrap().proposal;
        let section = key("orchestrator").unwrap();
        let history = prompt_sections::revisions(&db, &section).unwrap();
        assert!(history[0].attribution.is_some());
        for _ in 0..2 {
            let tx = db.unchecked_transaction().unwrap();
            prompt_sections::install_guidance_revision_store(&tx).unwrap();
            install_store(&tx).unwrap();
            tx.commit().unwrap();
        }
        assert_eq!(prompt_sections::revisions(&db, &section).unwrap(), history);
        assert_eq!(get(&db, &proposal.id).unwrap(), Some(accepted));
        let foreign_table: String = db.query_row(
            "SELECT \"table\" FROM pragma_foreign_key_list('prompt_mutation_proposals') WHERE \"from\"='revision_id'",
            [], |row| row.get(0),
        ).unwrap();
        assert_eq!(foreign_table, "prompt_section_revisions");
        let violations: i64 = db.query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| row.get(0)).unwrap();
        assert_eq!(violations, 0);
        assert!(db.execute("UPDATE prompt_section_revisions SET attribution=NULL", []).is_err());
        assert!(db.execute("DELETE FROM prompt_mutation_proposals", []).is_err());
    }
}
