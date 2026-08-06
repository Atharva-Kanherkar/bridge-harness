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
#[serde(rename_all = "camelCase")]
pub struct RefreshWorkspaceParams {
    pub workspace_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveWorkspaceParams {
    pub workspace_id: String,
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
            serde_json::to_value(ListWorkspaceFilesParams { session_id: "s".into() }).unwrap(),
            json!({"sessionId": "s"})
        );
        let create = CreateWorkspaceParams { title: "Payments".into() };
        assert_eq!(round_trip(&create), create);
        let refresh = RefreshWorkspaceParams { workspace_id: "w-1".into() };
        assert_eq!(round_trip(&refresh), refresh);
        let archive = ArchiveWorkspaceParams { workspace_id: "w-1".into() };
        assert_eq!(round_trip(&archive), archive);
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
        assert!(serde_json::from_value::<ArchiveWorkspaceParams>(json!({})).is_err());
        // Wire names are camelCase; snake_case spellings are not accepted.
        assert!(serde_json::from_value::<ArchiveWorkspaceParams>(json!({"workspace_id": "w-1"}))
            .is_err());
    }
}
