//! The workspaces domain: the folders and worktrees sessions run in.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreateWorkspaceParams {
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConnectWorkspaceFolderParams {
    pub workspace_id: String,
    /// Folder to connect; a Git repository resolves to its root and links a
    /// project, a plain folder connects as-is.
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ListWorkspaceFilesParams {
    pub session_id: String,
}

/// Repository-relative file paths; empty for sessions with no workspace root.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct ListWorkspaceFilesResult(pub Vec<String>);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ListWorkspaceTreeParams {
    pub workspace_id: String,
}

/// Repository-relative file paths for the editor's tree and file palette.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct ListWorkspaceTreeResult(pub Vec<String>);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReadWorkspaceFileParams {
    pub workspace_id: String,
    /// Path relative to the workspace root. Absolute paths and `..` are refused.
    pub path: String,
}

/// One workspace file opened in the editor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReadWorkspaceFileResult {
    pub path: String,
    /// Empty when `binary` or `tooLarge` is set.
    pub content: String,
    /// SHA-256 of the bytes on disk. A later write must present this hash, or
    /// it is rejected as a lost update. Empty when `tooLarge` — a file we
    /// declined to read has no write token, and a write carrying an empty
    /// token is refused.
    pub sha256: String,
    /// Over the editor's size ceiling; shown as a notice, not opened.
    pub too_large: bool,
    /// Binary, or text in an encoding editing would rewrite. Read-only: a
    /// write over a file whose bytes are binary is refused server-side.
    pub binary: bool,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WriteWorkspaceFileParams {
    pub workspace_id: String,
    pub path: String,
    pub content: String,
    /// The hash the editor last read. `null` means "create; must not exist",
    /// which is decided atomically by `O_EXCL`. A mismatch fails the write
    /// rather than clobbering an agent's edit.
    pub base_sha256: Option<String>,
}

/// The hash of the bytes just written, so the editor can keep going without
/// a re-read.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WriteWorkspaceFileResult {
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RefreshWorkspaceParams {
    pub workspace_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ListWorkspaceBranchesParams {
    pub workspace_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ListWorkspaceBranchesResult {
    pub current: Option<String>,
    pub branches: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CheckoutWorkspaceBranchParams {
    pub workspace_id: String,
    pub branch: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveWorkspaceParams {
    pub workspace_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspaceChangesParams {
    pub workspace_id: String,
}

/// A file's importance for review triage. Mirrors
/// `bridge_core::completion::RiskTier`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RiskTier {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceRepositoryState {
    Normal,
    Unborn,
    NotGit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceChangeKind {
    Added,
    Modified,
    Deleted,
    Renamed,
    ModeOnly,
}

/// One file's working-tree diff against `HEAD`. Mirrors
/// `bridge_core::git::WorkspaceFileChange`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceFileChange {
    pub path: String,
    pub previous_path: Option<String>,
    pub change_kind: WorkspaceChangeKind,
    pub additions: i64,
    pub deletions: i64,
    /// Unified diff text; empty for files detected as binary.
    pub patch: String,
    pub patch_truncated: bool,
    pub binary: bool,
    pub importance: RiskTier,
    pub labels: Vec<String>,
    /// Lockfiles, generated output, vendored trees: real changes, low review
    /// signal. A UI may collapse these by default, but never omit them.
    pub low_signal: bool,
}

/// `workspaces/workspace_changes`' result: every path that differs from
/// `HEAD` in the workspace's working tree, tracked or not. Mirrors
/// `bridge_core::git::WorkspaceChangeset`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceChangesResult {
    pub base_commit: Option<String>,
    pub repository_state: WorkspaceRepositoryState,
    pub files: Vec<WorkspaceFileChange>,
    pub total_files: Option<usize>,
    pub files_truncated: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::common::round_trip;
    use serde_json::json;

    #[test]
    fn params_round_trip_with_camel_case_wire_names() {
        let connect = ConnectWorkspaceFolderParams {
            workspace_id: "w-1".into(),
            path: "/repos/demo".into(),
        };
        let wire = serde_json::to_value(&connect).unwrap();
        assert_eq!(wire, json!({"workspaceId": "w-1", "path": "/repos/demo"}));
        assert_eq!(round_trip(&connect), connect);

        assert_eq!(
            serde_json::to_value(ListWorkspaceFilesParams {
                session_id: "s".into()
            })
            .unwrap(),
            json!({"sessionId": "s"})
        );
        let create = CreateWorkspaceParams {
            title: "Payments".into(),
        };
        assert_eq!(round_trip(&create), create);
        let refresh = RefreshWorkspaceParams {
            workspace_id: "w-1".into(),
        };
        assert_eq!(round_trip(&refresh), refresh);
        let list_branches = ListWorkspaceBranchesParams {
            workspace_id: "w-1".into(),
        };
        assert_eq!(round_trip(&list_branches), list_branches);
        let checkout = CheckoutWorkspaceBranchParams {
            workspace_id: "w-1".into(),
            branch: "feat/real-branch-menu".into(),
        };
        assert_eq!(
            serde_json::to_value(&checkout).unwrap(),
            json!({"workspaceId": "w-1", "branch": "feat/real-branch-menu"})
        );
        assert_eq!(round_trip(&checkout), checkout);
        let archive = ArchiveWorkspaceParams {
            workspace_id: "w-1".into(),
        };
        assert_eq!(round_trip(&archive), archive);
        let changes = WorkspaceChangesParams {
            workspace_id: "w-1".into(),
        };
        assert_eq!(round_trip(&changes), changes);
    }

    #[test]
    fn workspace_change_serializes_with_snake_case_importance() {
        let change = WorkspaceFileChange {
            path: "src/App.tsx".into(),
            previous_path: Some("src/OldApp.tsx".into()),
            change_kind: WorkspaceChangeKind::Renamed,
            additions: 4,
            deletions: 1,
            patch: "@@ -1 +1,4 @@".into(),
            patch_truncated: true,
            binary: false,
            importance: RiskTier::Medium,
            labels: vec!["frontend".into()],
            low_signal: false,
        };
        let wire = serde_json::to_value(&change).unwrap();
        assert_eq!(
            wire,
            json!({
                "path": "src/App.tsx",
                "previousPath": "src/OldApp.tsx",
                "changeKind": "renamed",
                "additions": 4,
                "deletions": 1,
                "patch": "@@ -1 +1,4 @@",
                "patchTruncated": true,
                "binary": false,
                "importance": "medium",
                "labels": ["frontend"],
                "lowSignal": false,
            })
        );
        assert_eq!(round_trip(&change), change);

        let result = WorkspaceChangesResult {
            base_commit: Some("abc123".into()),
            repository_state: WorkspaceRepositoryState::Normal,
            files: vec![change],
            total_files: Some(1),
            files_truncated: false,
        };
        assert_eq!(round_trip(&result), result);
    }

    #[test]
    fn file_listing_result_is_a_bare_array_on_the_wire() {
        let result = ListWorkspaceFilesResult(vec!["src/main.rs".into(), "README.md".into()]);
        assert_eq!(
            serde_json::to_value(&result).unwrap(),
            json!(["src/main.rs", "README.md"])
        );
        assert_eq!(round_trip(&result), result);
    }

    #[test]
    fn params_reject_payloads_missing_their_required_fields() {
        assert!(serde_json::from_value::<CreateWorkspaceParams>(json!({})).is_err());
        assert!(serde_json::from_value::<ConnectWorkspaceFolderParams>(json!({})).is_err());
        assert!(
            serde_json::from_value::<ConnectWorkspaceFolderParams>(json!({"path": "/x"})).is_err(),
            "workspaceId is required"
        );
        assert!(serde_json::from_value::<ListWorkspaceFilesParams>(json!({})).is_err());
        assert!(serde_json::from_value::<RefreshWorkspaceParams>(json!({})).is_err());
        assert!(serde_json::from_value::<ListWorkspaceBranchesParams>(json!({})).is_err());
        assert!(serde_json::from_value::<CheckoutWorkspaceBranchParams>(
            json!({"workspaceId": "w-1"})
        )
        .is_err());
        assert!(serde_json::from_value::<ArchiveWorkspaceParams>(json!({})).is_err());
        // Wire names are camelCase; snake_case spellings are not accepted.
        assert!(
            serde_json::from_value::<ArchiveWorkspaceParams>(json!({"workspace_id": "w-1"}))
                .is_err()
        );
        assert!(serde_json::from_value::<WorkspaceChangesParams>(json!({})).is_err());
        assert!(serde_json::from_value::<WorkspaceChangesParams>(
            json!({"workspaceId": "w-1", "unexpected": true})
        )
        .is_err());
    }
}
