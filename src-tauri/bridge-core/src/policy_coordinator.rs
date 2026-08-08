use crate::{delegation::DelegationRequest, policy, session_forest::SessionForest, BridgeError};
use rusqlite::{params, Connection};
use std::path::Path;

pub struct WorkerRouteContext {
    pub workspace_id: String,
    pub parent_depth: i64,
    pub path: String,
    pub branch: String,
    pub outcome: policy::PolicyOutcome,
    /// Set when the decision raised or re-used a pending approval card. The
    /// launch is still live; the caller must not report it as a failure.
    pub pending_approval_id: Option<String>,
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
        let pending_approval_id =
            policy::record_decision(db, parent_session_id, turn_id, &input, &outcome)?;
        Ok(WorkerRouteContext {
            workspace_id,
            parent_depth,
            path,
            branch,
            outcome,
            pending_approval_id,
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
    let prior_write_decision = branch.iter().find(|entry| {
        entry.payload["turnId"] == turn_id
            && entry
                .payload
                .pointer("/request/writeMode")
                .and_then(serde_json::Value::as_str)
                != Some("readOnly")
            && matches!(
                entry.kind.as_str(),
                "approval.requested"
                    | "delegation.requested"
                    | "delegation.approved"
                    | "delegation.rejected"
            )
    });
    let user_entry = if let Some(decision) = prior_write_decision {
        let source_ids = decision
            .payload
            .pointer("/ownedPathProvenance/sourceEntryIds")
            .and_then(serde_json::Value::as_array);
        source_ids.and_then(|ids| {
            branch.iter().find(|entry| {
                entry.kind == "user.message"
                    && ids.iter().any(|id| id.as_str() == Some(entry.id.as_str()))
            })
        })
    } else {
        branch.iter().rfind(|entry| entry.kind == "user.message")
    };
    if let Some(entry) = user_entry {
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
        if entry.kind != "approval.resolved"
            || entry.payload["approvalType"] != "delegation_path_scope"
            || entry.payload["turnId"] != turn_id
        {
            return false;
        }
        let accepted = matches!(
            entry
                .payload
                .get("decision")
                .and_then(serde_json::Value::as_str),
            Some("accept")
        );
        let Some(request_entry_id) = entry.payload["requestEntryId"].as_str() else {
            return false;
        };
        accepted
            && branch.iter().any(|request| {
                request.id == request_entry_id
                    && request.kind == "approval.requested"
                    && request.payload["approvalId"] == entry.payload["approvalId"]
                    && request.payload["turnId"] == turn_id
            })
    }) {
        let paths = entry
            .payload
            .get("approvedOwnedPaths")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(serde_json::Value::as_str)
            .filter_map(|path| approved_path_token(path, workspace))
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
    let mut paths = Vec::new();
    let mut fence: Option<(char, usize)> = None;
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some((marker, length, can_close)) = markdown_fence_marker(trimmed) {
            match fence {
                None => fence = Some((marker, length)),
                Some((open_marker, open_length))
                    if can_close && marker == open_marker && length >= open_length =>
                {
                    fence = None;
                }
                Some(_) => {}
            }
            continue;
        }
        if fence.is_some() || trimmed.starts_with('>') {
            continue;
        }
        let prefix = "write scope:";
        let Some(scope) = trimmed
            .get(..prefix.len())
            .filter(|candidate| candidate.eq_ignore_ascii_case(prefix))
            .map(|_| trimmed[prefix.len()..].trim())
        else {
            continue;
        };
        paths.extend(
            scope
                .split([',', ';'])
                .flat_map(str::split_whitespace)
                .filter_map(|path| trusted_path_token(path, workspace)),
        );
    }
    paths.sort();
    paths.dedup();
    paths
}

fn markdown_fence_marker(line: &str) -> Option<(char, usize, bool)> {
    let marker = line.chars().next()?;
    if marker != '`' && marker != '~' {
        return None;
    }
    let length = line
        .chars()
        .take_while(|character| *character == marker)
        .count();
    if length < 3 {
        return None;
    }
    let remainder = &line[length..];
    Some((marker, length, remainder.trim().is_empty()))
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
    let workspace = workspace.canonicalize().ok()?;
    let grounding_path = if resolved.exists() {
        resolved.as_path()
    } else if wildcard.is_none() {
        resolved.parent().filter(|parent| parent.is_dir())?
    } else {
        return None;
    };
    let grounded = grounding_path.canonicalize().ok()?;
    if !grounded.starts_with(&workspace)
        || (resolved.is_dir() && subtree_has_escaping_symlink(&resolved, &workspace))
    {
        return None;
    }
    if wildcard.is_none() && resolved.is_dir() {
        Some(format!("{normalized}/**"))
    } else {
        Some(normalized)
    }
}

fn approved_path_token(token: &str, workspace: &Path) -> Option<String> {
    let normalized = policy::normalize_owned_pattern(token).ok()?;
    let wildcard = normalized.find(['*', '?', '[']);
    let base = wildcard
        .map(|index| normalized[..index].trim_end_matches('/'))
        .unwrap_or(normalized.as_str());
    if base.is_empty() {
        return None;
    }
    let workspace = workspace.canonicalize().ok()?;
    let resolved = workspace.join(base);
    let grounding_path = if resolved.exists() {
        resolved.as_path()
    } else {
        resolved.parent().filter(|parent| parent.is_dir())?
    };
    let grounded = grounding_path.canonicalize().ok()?;
    (grounded.starts_with(&workspace)
        && !(resolved.is_dir() && subtree_has_escaping_symlink(&resolved, &workspace)))
    .then_some(normalized)
}

fn subtree_has_escaping_symlink(root: &Path, workspace: &Path) -> bool {
    subtree_has_escaping_symlink_with_limit(root, workspace, 4_096)
}

fn subtree_has_escaping_symlink_with_limit(root: &Path, workspace: &Path, limit: usize) -> bool {
    let mut pending = vec![root.to_path_buf()];
    let mut visited = 0usize;
    while let Some(directory) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(directory) else {
            return true;
        };
        for entry in entries {
            visited += 1;
            if visited > limit {
                return true;
            }
            let Ok(entry) = entry else {
                return true;
            };
            let path = entry.path();
            let Ok(metadata) = std::fs::symlink_metadata(&path) else {
                return true;
            };
            if metadata.file_type().is_symlink() {
                let Ok(target) = path.canonicalize() else {
                    return true;
                };
                if !target.starts_with(workspace) {
                    return true;
                }
                continue;
            }
            if metadata.is_dir() {
                pending.push(path);
            }
        }
    }
    false
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
            evidence_ids: Vec::new(),
            relevant_files: vec!["src/**".into()],
            owned_paths: paths.iter().map(|path| (*path).into()).collect(),
            write_mode: WriteMode::Isolated,
            capability_tier: CapabilityTier::Standard,
            effort: Effort::Medium,
            network_access: false,
            writable_output_paths: vec![],
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
    fn fenced_or_quoted_scope_examples_do_not_authorize_writes() {
        let workspace = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(workspace.path().join("src/auth")).unwrap();
        let text = "Diagnostic example:\n```text\nWrite scope: src/auth/**\n```\n> Write scope: src/auth/**";
        assert!(explicit_write_scope(text, workspace.path()).is_empty());
        let mixed = "````text\n~~~\nWrite scope: src/auth/**\n```\n````";
        assert!(explicit_write_scope(mixed, workspace.path()).is_empty());
        let info_string = "```text\n```still-code\nWrite scope: src/auth/**\n```";
        assert!(explicit_write_scope(info_string, workspace.path()).is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_scope_cannot_escape_the_workspace() {
        use std::os::unix::fs::symlink;

        let workspace = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        symlink(outside.path(), workspace.path().join("external")).unwrap();
        assert!(explicit_write_scope("Write scope: external/**", workspace.path()).is_empty());
        assert!(explicit_write_scope("Write scope: external/new.rs", workspace.path()).is_empty());
        assert!(approved_path_token("external/**", workspace.path()).is_none());
        std::fs::create_dir_all(workspace.path().join("src")).unwrap();
        symlink(outside.path(), workspace.path().join("src/link")).unwrap();
        assert!(explicit_write_scope("Write scope: src/**", workspace.path()).is_empty());
        assert!(approved_path_token("src/**", workspace.path()).is_none());
        assert!(explicit_write_scope("Write scope: src", workspace.path()).is_empty());
        assert!(approved_path_token("src", workspace.path()).is_none());
        std::fs::remove_file(workspace.path().join("src/link")).unwrap();
        std::fs::create_dir_all(workspace.path().join("src/shared")).unwrap();
        symlink(
            workspace.path().join("src/shared"),
            workspace.path().join("src/internal"),
        )
        .unwrap();
        assert_eq!(
            explicit_write_scope("Write scope: src/**", workspace.path()),
            vec!["src/**"]
        );
        symlink(outside.path(), workspace.path().join("src/shared/external")).unwrap();
        assert!(explicit_write_scope("Write scope: src/**", workspace.path()).is_empty());
        std::fs::remove_file(workspace.path().join("src/shared/external")).unwrap();
        std::fs::remove_file(workspace.path().join("src/internal")).unwrap();
        assert_eq!(
            approved_path_token("src/new/**", workspace.path()),
            Some("src/new/**".into())
        );
    }

    #[test]
    fn symlink_scan_fails_closed_at_its_entry_budget() {
        let workspace = tempfile::tempdir().unwrap();
        let root = workspace.path().join("src");
        std::fs::create_dir_all(&root).unwrap();
        for name in ["a", "b", "c"] {
            std::fs::write(root.join(name), "").unwrap();
        }
        assert!(subtree_has_escaping_symlink_with_limit(
            &root,
            workspace.path(),
            2
        ));
        assert!(!subtree_has_escaping_symlink_with_limit(
            &root,
            workspace.path(),
            3
        ));
    }

    #[test]
    fn prior_write_decision_binds_scope_to_its_originating_turn() {
        let workspace = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(workspace.path().join("src/auth")).unwrap();
        std::fs::create_dir_all(workspace.path().join("src/other")).unwrap();
        let db = database(workspace.path());
        let forest = SessionForest::new(&db);
        forest
            .append(
                "parent",
                EntryKind::UserMessage,
                json!({"text":"Write scope: src/auth/**"}),
            )
            .unwrap();
        let first = PolicyCoordinator::decide_worker_route(
            &db,
            "parent",
            "turn-a",
            &request(&["src/auth/**"]),
            true,
        )
        .unwrap();
        assert!(matches!(
            first.outcome.decision,
            policy::RouteDecision::SpawnWorker(_)
        ));
        forest
            .append(
                "parent",
                EntryKind::UserMessage,
                json!({"text":"Write scope: src/**"}),
            )
            .unwrap();
        let rechecked = PolicyCoordinator::decide_worker_route(
            &db,
            "parent",
            "turn-a",
            &request(&["src/other/**"]),
            true,
        )
        .unwrap();
        assert_eq!(
            rechecked.outcome.reason,
            policy::RouteReason::OwnedPathProvenanceRequired
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
