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

// --- sessions ----------------------------------------------------------------

/// A harness identifier on the wire. Mirrors `bridge_core::model::Harness`
/// variant for variant; an exhaustive conversion in bridge-core keeps the two
/// from drifting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum HarnessId {
    Claude,
    Codex,
    OpenCode,
    Shell,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GetSessionForestParams {
    pub session_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ActivateSessionEntryParams {
    pub session_id: String,
    /// The forest entry to become the conversation head; files are not changed.
    pub entry_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreateChatParams {
    pub harness: HarnessId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreateWorkspaceSessionParams {
    pub workspace_id: String,
    /// Create the session in an isolated Git worktree (requires a connected
    /// repository).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub create_worktree: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UpdateChatModelParams {
    pub session_id: String,
    pub harness: HarnessId,
    /// Explicit model id; omitted selects the harness's default for the
    /// chat's tier.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct InterruptTurnParams {
    pub session_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CompactSessionParams {
    pub session_id: String,
}

// --- registry ----------------------------------------------------------------

/// A method whose payloads are contracted: its params type name and, when the
/// result is not the still-untyped `BridgeState` snapshot, its result type
/// name. Type names refer to exported types in this module, and the generated
/// TypeScript `BridgeMethodParams`/`BridgeMethodResults` maps are built from
/// this table.
pub struct TypedMethod {
    pub method: MethodName,
    /// The params type name, or `None` when the method takes no parameters —
    /// that absence is itself contract (a validator rejects any params).
    pub params: Option<&'static str>,
    pub result: Option<&'static str>,
}

pub const TYPED_METHODS: &[TypedMethod] = &[
    TypedMethod { method: MethodName::AddProject, params: Some("AddProjectParams"), result: None },
    TypedMethod {
        method: MethodName::CreateWorkspace,
        params: Some("CreateWorkspaceParams"),
        result: None,
    },
    TypedMethod {
        method: MethodName::ConnectWorkspaceFolder,
        params: Some("ConnectWorkspaceFolderParams"),
        result: None,
    },
    TypedMethod {
        method: MethodName::ListWorkspaceFiles,
        params: Some("ListWorkspaceFilesParams"),
        result: Some("ListWorkspaceFilesResult"),
    },
    TypedMethod {
        method: MethodName::RefreshWorkspace,
        params: Some("RefreshWorkspaceParams"),
        result: None,
    },
    TypedMethod {
        method: MethodName::ArchiveWorkspace,
        params: Some("ArchiveWorkspaceParams"),
        result: None,
    },
    TypedMethod {
        method: MethodName::GetSessionForest,
        params: Some("GetSessionForestParams"),
        result: None,
    },
    TypedMethod {
        method: MethodName::ActivateSessionEntry,
        params: Some("ActivateSessionEntryParams"),
        result: None,
    },
    TypedMethod { method: MethodName::CreateChat, params: Some("CreateChatParams"), result: None },
    TypedMethod {
        method: MethodName::CreateWorkspaceSession,
        params: Some("CreateWorkspaceSessionParams"),
        result: None,
    },
    TypedMethod {
        method: MethodName::UpdateChatModel,
        params: Some("UpdateChatModelParams"),
        result: None,
    },
    TypedMethod {
        method: MethodName::InterruptTurn,
        params: Some("InterruptTurnParams"),
        result: None,
    },
    TypedMethod {
        method: MethodName::CompactSession,
        params: Some("CompactSessionParams"),
        result: None,
    },
    TypedMethod { method: MethodName::RefreshAccountUsage, params: None, result: None },
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
    fn session_params_round_trip_and_omit_absent_options() {
        let create = CreateChatParams { harness: HarnessId::Codex, model: None, title: None };
        let wire = serde_json::to_value(&create).unwrap();
        assert_eq!(wire, json!({"harness": "codex"}), "absent options stay off the wire");
        assert_eq!(round_trip(&create), create);

        let update = UpdateChatModelParams {
            session_id: "s-1".into(),
            harness: HarnessId::OpenCode,
            model: Some("kimi-k2.5".into()),
        };
        let wire = serde_json::to_value(&update).unwrap();
        assert_eq!(
            wire,
            json!({"sessionId": "s-1", "harness": "opencode", "model": "kimi-k2.5"})
        );
        assert_eq!(round_trip(&update), update);

        let session = CreateWorkspaceSessionParams {
            workspace_id: "w-1".into(),
            create_worktree: Some(true),
        };
        assert_eq!(
            serde_json::to_value(&session).unwrap(),
            json!({"workspaceId": "w-1", "createWorktree": true})
        );
        let activate =
            ActivateSessionEntryParams { session_id: "s-1".into(), entry_id: "e-9".into() };
        assert_eq!(round_trip(&activate), activate);
        let forest = GetSessionForestParams { session_id: "s-1".into() };
        assert_eq!(round_trip(&forest), forest);
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
        assert!(serde_json::from_value::<GetSessionForestParams>(json!({})).is_err());
        assert!(serde_json::from_value::<ActivateSessionEntryParams>(
            json!({"sessionId": "s"})
        )
        .is_err());
        assert!(serde_json::from_value::<CreateChatParams>(json!({})).is_err());
        assert!(
            serde_json::from_value::<CreateChatParams>(json!({"harness": "cursor"})).is_err(),
            "unknown harness ids must be rejected"
        );
        assert!(serde_json::from_value::<CreateWorkspaceSessionParams>(json!({})).is_err());
        assert!(serde_json::from_value::<UpdateChatModelParams>(
            json!({"sessionId": "s"})
        )
        .is_err());
        assert!(serde_json::from_value::<InterruptTurnParams>(json!({})).is_err());
        assert!(serde_json::from_value::<CompactSessionParams>(
            json!({"session_id": "s"})
        )
        .is_err());
    }

    #[test]
    fn no_params_methods_are_contracted_as_such() {
        let refresh = TYPED_METHODS
            .iter()
            .find(|entry| entry.method == MethodName::RefreshAccountUsage)
            .unwrap();
        assert!(refresh.params.is_none(), "refresh_account_usage takes no parameters");
    }

    #[test]
    fn typed_methods_are_unique_and_cover_their_domains() {
        let mut seen = HashSet::new();
        for entry in TYPED_METHODS {
            assert!(seen.insert(entry.method.as_str()), "duplicate {}", entry.method.as_str());
        }
        // The live-turn half of the sessions domain is not yet contracted —
        // it lands with the event-publisher seam. Every other method in a
        // typed domain must have typed params; shrink this list as slice B
        // methods are contracted.
        const PENDING_SESSIONS_SLICE_B: &[MethodName] = &[
            MethodName::StartSession,
            MethodName::StartChat,
            MethodName::PrepareTurn,
            MethodName::SendTurn,
            MethodName::StopSession,
        ];
        for method in MethodName::ALL.iter().copied() {
            if matches!(method.domain(), "projects" | "workspaces" | "sessions")
                && !PENDING_SESSIONS_SLICE_B.contains(&method)
            {
                assert!(
                    TYPED_METHODS.iter().any(|entry| entry.method == method),
                    "{} is in a typed domain but has no typed params",
                    method.as_str()
                );
            }
        }
        for method in PENDING_SESSIONS_SLICE_B {
            assert!(
                TYPED_METHODS.iter().all(|entry| entry.method != *method),
                "{} is typed — remove it from the pending list",
                method.as_str()
            );
        }
    }
}
