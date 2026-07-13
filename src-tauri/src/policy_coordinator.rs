use crate::{delegation::DelegationRequest, policy, session_forest::SessionForest, BridgeError};
use rusqlite::{params, Connection};
use std::path::Path;

pub struct WorkerRouteContext {
    pub workspace_id: String,
    pub parent_depth: i64,
    pub path: String,
    pub branch: String,
    pub outcome: policy::PolicyOutcome,
}

pub struct PolicyCoordinator;

impl PolicyCoordinator {
    pub fn decide_worker_route(
        db: &Connection,
        parent_session_id: &str,
        turn_id: &str,
        request: &DelegationRequest,
        child_worktrees_available: bool,
    ) -> Result<WorkerRouteContext, BridgeError> {
        let (workspace_id, parent_depth, path, branch): (String, i64, String, String) = db
            .query_row(
                "SELECT s.workspace_id,COALESCE(s.depth,0),w.path,w.branch FROM sessions s JOIN workspaces w ON w.id=s.workspace_id WHERE s.id=?1",
                params![parent_session_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )?;
        let budget = policy::load_request_budget(db, &workspace_id, turn_id)?;
        let owned_path_provenance = if request.write_mode == crate::delegation::WriteMode::ReadOnly
        {
            policy::OwnedPathProvenance::default()
        } else {
            owned_path_provenance(db, parent_session_id, turn_id, Path::new(&path))?
        };
        let input = policy::PolicyInput {
            workspace_id: workspace_id.clone(),
            worktree_id: workspace_id.clone(),
            parent_session_id: parent_session_id.into(),
            turn_id: turn_id.into(),
            parent_depth,
            request: request.clone(),
            owned_path_provenance,
            requested_harness: request.runtime_harness(),
            task_family: policy::role_name(request.role).into(),
            active_workers: policy::load_workers(db, &workspace_id, "active")?,
            warm_workers: policy::load_workers(db, &workspace_id, "warm")?,
            budget: budget.clone(),
            retry_count: 0,
            parent_can_execute: false,
            requires_user_approval: false,
            child_worktrees_available,
        };
        let outcome = policy::PolicyEngine::default().decide(&input);
        policy::record_decision(
            db,
            parent_session_id,
            turn_id,
            request,
            &outcome,
            &budget,
            &input.owned_path_provenance,
        )?;
        Ok(WorkerRouteContext {
            workspace_id,
            parent_depth,
            path,
            branch,
            outcome,
        })
    }
}

fn owned_path_provenance(
    db: &Connection,
    parent_session_id: &str,
    turn_id: &str,
    workspace: &Path,
) -> Result<policy::OwnedPathProvenance, BridgeError> {
    let branch = SessionForest::new(db)
        .active_branch(parent_session_id)
        .map_err(|error| BridgeError::Invalid(error.to_string()))?;
    let mut trusted_paths = Vec::new();
    let mut source_entry_ids = Vec::new();
    if let Some(entry) = branch.iter().rfind(|entry| entry.kind == "user.message") {
        let paths = entry
            .payload
            .get("text")
            .and_then(serde_json::Value::as_str)
            .map(|text| explicit_write_scope(text, workspace))
            .unwrap_or_default();
        if !paths.is_empty() {
            trusted_paths.extend(paths);
            source_entry_ids.push(entry.id.clone());
        }
    }
    for entry in branch.iter().filter(|entry| {
        entry.kind == "approval.resolved"
            && entry.payload["approvalType"] == "delegation_path_scope"
            && entry.payload["turnId"] == turn_id
            && matches!(
                entry
                    .payload
                    .get("decision")
                    .and_then(serde_json::Value::as_str),
                Some("accept" | "acceptForSession")
            )
    }) {
        let paths = entry
            .payload
            .get("approvedOwnedPaths")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(serde_json::Value::as_str)
            .filter_map(|path| policy::normalize_owned_pattern(path).ok())
            .collect::<Vec<_>>();
        if !paths.is_empty() {
            trusted_paths.extend(paths);
            source_entry_ids.push(entry.id.clone());
        }
    }
    trusted_paths.sort();
    trusted_paths.dedup();
    source_entry_ids.sort();
    source_entry_ids.dedup();
    Ok(policy::OwnedPathProvenance {
        trusted_paths,
        source_entry_ids,
    })
}

fn explicit_write_scope(text: &str, workspace: &Path) -> Vec<String> {
    if !workspace.is_dir() {
        return Vec::new();
    }
    let mut paths = text
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            let prefix = "write scope:";
            line.get(..prefix.len())
                .filter(|candidate| candidate.eq_ignore_ascii_case(prefix))
                .map(|_| line[prefix.len()..].trim())
        })
        .flat_map(|scope| scope.split([',', ';']))
        .flat_map(str::split_whitespace)
        .filter_map(|path| trusted_path_token(path, workspace))
        .collect::<Vec<_>>();
    paths.sort();
    paths.dedup();
    paths
}

fn trusted_path_token(token: &str, workspace: &Path) -> Option<String> {
    let token = token.trim_matches(|character: char| {
        matches!(
            character,
            '`' | '"' | '\'' | '(' | ')' | '{' | '}' | ',' | ';' | ':'
        )
    });
    let token = if token.ends_with('.')
        && (token.contains('/') || token.chars().filter(|character| *character == '.').count() > 1)
    {
        token.trim_end_matches('.')
    } else {
        token
    };
    let token = token.trim_matches('`');
    if token.is_empty() || token.contains("://") {
        return None;
    }
    let looks_like_path = token.contains('/')
        || token
            .rsplit_once('.')
            .is_some_and(|(stem, extension)| !stem.is_empty() && !extension.is_empty());
    if !looks_like_path {
        return None;
    }
    let normalized = policy::normalize_owned_pattern(token).ok()?;
    let wildcard = normalized.find(['*', '?', '[']);
    let base = wildcard
        .map(|index| normalized[..index].trim_end_matches('/'))
        .unwrap_or(normalized.as_str());
    if base.is_empty() {
        return None;
    }
    let resolved = workspace.join(base);
    let grounded =
        resolved.exists() || (wildcard.is_none() && resolved.parent().is_some_and(Path::is_dir));
    if !grounded {
        return None;
    }
    if wildcard.is_none() && resolved.is_dir() {
        Some(format!("{normalized}/**"))
    } else {
        Some(normalized)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        delegation::{
            CapabilityTier, Effort, OutputContract, WorkerRole, WriteMode, SCHEMA_VERSION,
        },
        session_forest::{EntryKind, SessionForest},
        store,
    };
    use serde_json::json;

    fn database(workspace: &Path) -> Connection {
        let db = store::open(Path::new(":memory:")).unwrap();
        db.execute(
            "INSERT INTO projects(id,name,path,created_at) VALUES('p','Demo',?1,'now')",
            params![workspace.to_string_lossy()],
        )
        .unwrap();
        db.execute("INSERT INTO workspaces(id,project_id,city,title,branch,path,status,created_at) VALUES('w','p','Kyoto','Task','bridge/task',?1,'idle','now')", params![workspace.to_string_lossy()]).unwrap();
        db.execute("INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,depth) VALUES('parent','w','codex','Parent','working','reported',0)", []).unwrap();
        db.execute("INSERT INTO session_heads(session_id,restoration_mode,updated_at) VALUES('parent','fresh','now')", []).unwrap();
        db
    }

    fn request(paths: &[&str]) -> DelegationRequest {
        DelegationRequest {
            schema_version: SCHEMA_VERSION,
            role: WorkerRole::Implementation,
            objective: "Implement the requested change".into(),
            acceptance_criteria: vec!["Tests pass".into()],
            known_facts: Vec::new(),
            decisions: Vec::new(),
            relevant_files: vec!["src/**".into()],
            owned_paths: paths.iter().map(|path| (*path).into()).collect(),
            write_mode: WriteMode::Isolated,
            capability_tier: CapabilityTier::Standard,
            effort: Effort::Medium,
            verification: vec!["cargo test".into()],
            output_contract: OutputContract::ImplementationResult,
            harness: Some("codex".into()),
            model: None,
        }
    }

    #[test]
    fn user_path_evidence_uses_latest_explicit_scope_and_workspace_facts() {
        let workspace = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(workspace.path().join("src/auth")).unwrap();
        std::fs::write(workspace.path().join("src/auth/session.rs"), "").unwrap();
        let db = database(workspace.path());
        let forest = SessionForest::new(&db);
        let trusted = forest
            .append(
                "parent",
                EntryKind::UserMessage,
                json!({"text":"Write scope: src/auth/session.rs"}),
            )
            .unwrap();
        let branch_point = forest
            .append(
                "parent",
                EntryKind::AssistantMessage,
                json!({"text":"I will inspect it"}),
            )
            .unwrap();
        forest
            .append(
                "parent",
                EntryKind::UserMessage,
                json!({"text":"Write scope: src/secret.rs"}),
            )
            .unwrap();
        forest.move_head("parent", Some(&branch_point.id)).unwrap();
        forest
            .append(
                "parent",
                EntryKind::AssistantMessage,
                json!({"text":"Model-authored scope: src/**"}),
            )
            .unwrap();

        let route = PolicyCoordinator::decide_worker_route(
            &db,
            "parent",
            "turn-1",
            &request(&["src/auth/session.rs"]),
            true,
        )
        .unwrap();
        assert!(matches!(
            route.outcome.decision,
            policy::RouteDecision::SpawnWorker(_)
        ));
        let decision = SessionForest::new(&db).active_branch("parent").unwrap();
        let decision = decision.last().unwrap();
        assert_eq!(
            decision.payload["ownedPathProvenance"]["trustedPaths"],
            json!(["src/auth/session.rs"])
        );
        assert_eq!(
            decision.payload["ownedPathProvenance"]["sourceEntryIds"],
            json!([trusted.id])
        );
    }

    #[test]
    fn historical_negated_and_diagnostic_paths_do_not_authorize_writes() {
        let workspace = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(workspace.path().join("src/auth")).unwrap();
        std::fs::write(workspace.path().join("src/auth/session.rs"), "").unwrap();
        let db = database(workspace.path());
        let forest = SessionForest::new(&db);
        forest
            .append(
                "parent",
                EntryKind::UserMessage,
                json!({"text":"Write scope: src/auth/session.rs"}),
            )
            .unwrap();
        forest
            .append(
                "parent",
                EntryKind::AssistantMessage,
                json!({"text":"Earlier task completed"}),
            )
            .unwrap();
        forest
            .append(
                "parent",
                EntryKind::UserMessage,
                json!({"text":"Do not modify src/auth/session.rs; explain this diagnostic only"}),
            )
            .unwrap();
        let route = PolicyCoordinator::decide_worker_route(
            &db,
            "parent",
            "turn-2",
            &request(&["src/auth/session.rs"]),
            true,
        )
        .unwrap();
        assert_eq!(
            route.outcome.reason,
            policy::RouteReason::OwnedPathProvenanceRequired
        );
    }

    #[test]
    fn new_file_scope_requires_existing_immediate_parent() {
        let workspace = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(workspace.path().join("src")).unwrap();
        assert!(
            explicit_write_scope("Write scope: src/missing/deep/new.rs", workspace.path())
                .is_empty()
        );
        std::fs::create_dir_all(workspace.path().join("src/missing/deep")).unwrap();
        assert_eq!(
            explicit_write_scope("Write scope: src/missing/deep/new.rs", workspace.path()),
            vec!["src/missing/deep/new.rs"]
        );
    }

    #[test]
    fn assistant_and_request_paths_do_not_create_provenance() {
        let workspace = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(workspace.path().join("src")).unwrap();
        let db = database(workspace.path());
        SessionForest::new(&db)
            .append(
                "parent",
                EntryKind::AssistantMessage,
                json!({"text":"I claim src/**"}),
            )
            .unwrap();
        let route = PolicyCoordinator::decide_worker_route(
            &db,
            "parent",
            "turn-1",
            &request(&["src/**"]),
            true,
        )
        .unwrap();
        assert_eq!(
            route.outcome.reason,
            policy::RouteReason::OwnedPathProvenanceRequired
        );
        assert!(matches!(
            route.outcome.decision,
            policy::RouteDecision::RequireUserApproval
        ));
        let entry = SessionForest::new(&db).active_branch("parent").unwrap();
        let entry = entry.last().unwrap();
        assert_eq!(entry.kind, "approval.requested");
        assert_eq!(entry.payload["approvalType"], "delegation_path_scope");
        assert_eq!(entry.payload["requestedOwnedPaths"], json!(["src/**"]));
    }
}
