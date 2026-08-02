//! Typed method payloads, added domain by domain as command bodies move onto
//! `BridgeCore`. Field names are camelCase on the wire, matching both the
//! generated TypeScript and Tauri's invoke-argument conversion.
//!
//! Results that return the aggregate application snapshot (`BridgeState`)
//! stay untyped here until the snapshot DTO itself is contracted — that is
//! its own slice, shared by many methods.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::methods::MethodName;

// --- projects ---------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AddProjectParams {
    /// Path to a Git repository (any path inside it resolves to the root).
    pub path: String,
}

// --- workspaces --------------------------------------------------------------

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

// --- registry ----------------------------------------------------------------

/// A method whose payloads are contracted: its params type name and, when the
/// result is not the still-untyped `BridgeState` snapshot, its result type
/// name. Type names refer to exported types in this module, and the generated
/// TypeScript `BridgeMethodParams`/`BridgeMethodResults` maps are built from
/// this table.
pub struct TypedMethod {
    pub method: MethodName,
    pub params: &'static str,
    pub result: Option<&'static str>,
}

pub const TYPED_METHODS: &[TypedMethod] = &[
    TypedMethod { method: MethodName::AddProject, params: "AddProjectParams", result: None },
    TypedMethod {
        method: MethodName::CreateWorkspace,
        params: "CreateWorkspaceParams",
        result: None,
    },
    TypedMethod {
        method: MethodName::ConnectWorkspaceFolder,
        params: "ConnectWorkspaceFolderParams",
        result: None,
    },
    TypedMethod {
        method: MethodName::ListWorkspaceFiles,
        params: "ListWorkspaceFilesParams",
        result: Some("ListWorkspaceFilesResult"),
    },
    TypedMethod {
        method: MethodName::RefreshWorkspace,
        params: "RefreshWorkspaceParams",
        result: None,
    },
    TypedMethod {
        method: MethodName::ArchiveWorkspace,
        params: "ArchiveWorkspaceParams",
        result: None,
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::HashSet;

    fn round_trip<T>(value: &T) -> T
    where
        T: Serialize + for<'de> serde::Deserialize<'de>,
    {
        serde_json::from_str(&serde_json::to_string(value).unwrap()).unwrap()
    }

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
        let add = AddProjectParams { path: "/repos/demo".into() };
        assert_eq!(round_trip(&add), add);
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
        // A validator (or the future compat adapter) must not accept an
        // empty object where the contract names required fields.
        assert!(serde_json::from_value::<AddProjectParams>(json!({})).is_err());
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
        assert!(serde_json::from_value::<ArchiveWorkspaceParams>(
            json!({"workspace_id": "w-1"})
        )
        .is_err());
    }

    #[test]
    fn typed_methods_are_unique_and_cover_the_projects_and_workspaces_domains() {
        let mut seen = HashSet::new();
        for entry in TYPED_METHODS {
            assert!(seen.insert(entry.method.as_str()), "duplicate {}", entry.method.as_str());
        }
        for method in MethodName::ALL.iter().copied() {
            if matches!(method.domain(), "projects" | "workspaces") {
                assert!(
                    TYPED_METHODS.iter().any(|entry| entry.method == method),
                    "{} is in a typed domain but has no typed params",
                    method.as_str()
                );
            }
        }
    }
}
