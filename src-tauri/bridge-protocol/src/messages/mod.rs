//! Typed method payloads, one module per method domain. Field names are
//! camelCase on the wire, matching both the generated TypeScript and Tauri's
//! invoke-argument conversion.
//!
//! **Every** method in the registry is contracted here: a method that takes
//! arguments names a params struct, and a parameterless method is contracted
//! *as* parameterless (`_` below) so a host can reject any payload sent to it.
//! [`TYPED_METHODS`] is therefore total over [`MethodName::ALL`], and a test
//! below fails the build if it ever stops being.
//!
//! Params structs refuse unknown fields. The handshake already rejects a client
//! whose minor is newer than the server's, so no compatible client can send a
//! field the server does not know — which makes an unknown field a client bug,
//! and `invalid_params` a better answer than silently ignoring it.
//!
//! Results are contracted where the shape is the method's own. Methods that
//! return the aggregate application snapshot (`BridgeState`) or a domain
//! snapshot still stay uncontracted: those DTOs are shared by many methods and
//! land in their own slice.

mod approvals;
mod browser;
mod common;
mod completion;
mod config;
mod learning;
mod marketplace;
mod models;
mod projects;
mod routing;
mod sessions;
mod skills;
mod slash;
mod terminal;
mod workspaces;

pub use approvals::*;
pub use browser::*;
pub use common::*;
pub use completion::*;
pub use config::*;
pub use learning::*;
pub use marketplace::*;
pub use models::*;
pub use projects::*;
pub use routing::*;
pub use sessions::*;
pub use skills::*;
pub use slash::*;
pub use terminal::*;
pub use workspaces::*;

use schemars::schema_for;
use serde_json::Value;

use crate::methods::MethodName;

/// A method's contracted payloads: its params type name and, when the result
/// shape is the method's own rather than a shared snapshot, its result type
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

/// `_` means "no type here": no params, or a result this slice leaves
/// uncontracted.
macro_rules! payload_name {
    (_) => {
        None
    };
    ($payload:ident) => {
        Some(stringify!($payload))
    };
}

macro_rules! push_payload_schema {
    (_, $schemas:ident) => {};
    ($payload:ident, $schemas:ident) => {
        push_payload(
            &mut $schemas,
            stringify!($payload),
            serde_json::to_value(schema_for!($payload)).unwrap(),
        )
    };
}

/// The single table every payload artifact is derived from: `TYPED_METHODS` for
/// the registry, [`payload_schemas`] for the JSON Schemas and TypeScript. A
/// method cannot be contracted in one and forgotten in the other.
macro_rules! typed_methods {
    ($(($method:ident, $params:tt, $result:tt)),* $(,)?) => {
        pub const TYPED_METHODS: &[TypedMethod] = &[$(
            TypedMethod {
                method: MethodName::$method,
                params: payload_name!($params),
                result: payload_name!($result),
            },
        )*];

        /// JSON Schemas for every contracted payload type, in registry order
        /// and deduplicated — `UnitResult` is the result of many methods.
        pub fn payload_schemas() -> Vec<(&'static str, Value)> {
            let mut schemas: Vec<(&'static str, Value)> = Vec::new();
            $(
                push_payload_schema!($params, schemas);
                push_payload_schema!($result, schemas);
            )*
            schemas
        }
    };
}

fn push_payload(schemas: &mut Vec<(&'static str, Value)>, name: &'static str, schema: Value) {
    if let Some((_, existing)) = schemas.iter().find(|(existing, _)| *existing == name) {
        assert_eq!(existing, &schema, "conflicting schemas for {name}");
        return;
    }
    schemas.push((name, schema));
}

typed_methods![
    // health
    (Health, _, _),
    // state — the aggregate application snapshot
    (GetState, _, _),
    // projects
    (AddProject, AddProjectParams, _),
    // workspaces
    (CreateWorkspace, CreateWorkspaceParams, _),
    (ConnectWorkspaceFolder, ConnectWorkspaceFolderParams, _),
    (ListWorkspaceFiles, ListWorkspaceFilesParams, ListWorkspaceFilesResult),
    (RefreshWorkspace, RefreshWorkspaceParams, _),
    (ArchiveWorkspace, ArchiveWorkspaceParams, _),
    // sessions
    (GetSessionForest, GetSessionForestParams, _),
    (ReplaySessionEvents, ReplaySessionEventsParams, ReplaySessionEventsResult),
    (ActivateSessionEntry, ActivateSessionEntryParams, _),
    (CreateChat, CreateChatParams, _),
    (CreateWorkspaceSession, CreateWorkspaceSessionParams, _),
    (StartSession, StartSessionParams, _),
    (StartChat, StartChatParams, _),
    (UpdateChatModel, UpdateChatModelParams, _),
    (PrepareTurn, PrepareTurnParams, _),
    (SendTurn, SendTurnParams, UnitResult),
    (CompactSession, CompactSessionParams, UnitResult),
    (InterruptTurn, InterruptTurnParams, UnitResult),
    (RefreshAccountUsage, _, UnitResult),
    (StopSession, StopSessionParams, _),
    // approvals
    (ResolveApproval, ResolveApprovalParams, UnitResult),
    // terminal
    (OpenTerminal, OpenTerminalParams, UnitResult),
    (WriteTerminal, WriteTerminalParams, UnitResult),
    (ResizeTerminal, ResizeTerminalParams, UnitResult),
    // slash commands
    (ListSlashCommands, _, _),
    (ResolveSlashCommand, ResolveSlashCommandParams, _),
    // completion / verification
    (CreateCompletionPlan, CreateCompletionPlanParams, _),
    (RecordCompletionCheck, RecordCompletionCheckParams, _),
    (WaiveCompletion, WaiveCompletionParams, _),
    (RegisterVerifierManifest, RegisterVerifierManifestParams, UnitResult),
    (VerifierCandidates, VerifierCandidatesParams, _),
    // routing
    (GetRouterPreferences, GetRouterPreferencesParams, _),
    (UpdateRouterPreferences, UpdateRouterPreferencesParams, _),
    (RollbackRoutingPolicy, RollbackRoutingPolicyParams, _),
    // model profiles
    (GetModelSetup, _, _),
    (RecommendedModelProfiles, _, _),
    (SaveModelProfiles, SaveModelProfilesParams, _),
    (ResetModelProfiles, _, _),
    // configuration
    (GetConfigState, _, _),
    (SaveHarnessConfig, SaveHarnessConfigParams, _),
    (ResetHarnessConfig, ResetHarnessConfigParams, _),
    (RefreshOpencodeCatalog, RefreshOpencodeCatalogParams, _),
    (SetOpencodeProviderApiKey, SetOpencodeProviderApiKeyParams, _),
    (RemoveOpencodeProviderAuth, RemoveOpencodeProviderAuthParams, _),
    (SaveAgentConfig, SaveAgentConfigParams, _),
    (DeleteAgentConfig, DeleteAgentConfigParams, _),
    (SetDefaultAgent, SetDefaultAgentParams, _),
    (ResetAllConfig, _, _),
    // adaptive learning
    (GetLearningState, _, _),
    (RunLearning, RunLearningParams, _),
    (CancelLearningRun, CancelLearningRunParams, _),
    (UpdateLearningSchedule, UpdateLearningScheduleParams, _),
    (RegisterLearningTrigger, RegisterLearningTriggerParams, UnitResult),
    (GetLearningTriggerInstructions, GetLearningTriggerInstructionsParams, _),
    (EnableLearningTrigger, EnableLearningTriggerParams, UnitResult),
    (ApproveLearningRun, ApproveLearningRunParams, _),
    // browser bridge
    (BrowserBridgeState, _, _),
    (InstallBrowserNativeHost, _, _),
    (BrowserAction, BrowserActionParams, _),
    (SetBrowserPermission, SetBrowserPermissionParams, UnitResult),
    (ResolveBrowserApproval, ResolveBrowserApprovalParams, UnitResult),
    (TakeoverBrowser, _, UnitResult),
    (DetachBrowser, _, _),
    (RouteBrowser, RouteBrowserParams, _),
    (BrowserSkills, _, _),
    (ConfigureRemoteBrowser, ConfigureRemoteBrowserParams, UnitResult),
    (StartRemoteBrowser, StartRemoteBrowserParams, _),
    // marketplace
    (MarketplaceCatalog, _, _),
    (MarketplaceAppAuthStates, _, _),
    (MarketplaceAction, MarketplaceActionParams, _),
    // skills
    (SkillCatalog, _, _),
    (SkillSuggestions, SkillSuggestionsParams, _),
    (PreviewSkillChange, PreviewSkillChangeParams, _),
    (ExecuteSkillChange, ExecuteSkillChangeParams, _),
];

impl TypedMethod {
    /// The contract's entry for a method. Total over [`MethodName::ALL`], so
    /// callers that already hold a `MethodName` never handle a `None`.
    pub fn for_method(method: MethodName) -> &'static TypedMethod {
        TYPED_METHODS
            .iter()
            .find(|typed| typed.method == method)
            .expect("every method is contracted; the coverage test guarantees it")
    }

    /// The wire field names a method's params object carries, sorted, or `None`
    /// when the method is contracted parameterless. Hosts use this to check
    /// their command signatures still match the contract.
    pub fn params_fields(method: MethodName) -> Option<Vec<String>> {
        let typed = TypedMethod::for_method(method);
        let name = typed.params?;
        let (_, schema) = payload_schemas()
            .into_iter()
            .find(|(candidate, _)| *candidate == name)
            .unwrap_or_else(|| panic!("{name} has no schema"));
        let properties = schema
            .get("properties")
            .and_then(Value::as_object)
            .unwrap_or_else(|| panic!("{name} is a params type but describes no properties"));
        Some(properties.keys().cloned().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn every_method_in_the_registry_is_contracted_exactly_once() {
        let mut seen = HashSet::new();
        for entry in TYPED_METHODS {
            assert!(seen.insert(entry.method.as_str()), "duplicate {}", entry.method.as_str());
        }
        for method in MethodName::ALL.iter().copied() {
            assert!(
                seen.contains(method.as_str()),
                "{} has no params contract — add it to typed_methods!",
                method.as_str()
            );
        }
        assert_eq!(
            TYPED_METHODS.len(),
            MethodName::ALL.len(),
            "the payload contract and the method registry must stay 1:1"
        );
    }

    #[test]
    fn parameterless_methods_are_contracted_as_such() {
        // The absence is contract: a host must reject params sent to these.
        for method in [
            MethodName::Health,
            MethodName::GetState,
            MethodName::RefreshAccountUsage,
            MethodName::ListSlashCommands,
            MethodName::GetConfigState,
            MethodName::MarketplaceCatalog,
            MethodName::TakeoverBrowser,
        ] {
            assert!(
                TypedMethod::for_method(method).params.is_none(),
                "{} takes no parameters",
                method.as_str()
            );
            assert!(TypedMethod::params_fields(method).is_none());
        }
    }

    #[test]
    fn params_types_are_named_after_their_method() {
        // A copy-paste in the table would otherwise point a method at another
        // method's params and validate the wrong payload.
        for entry in TYPED_METHODS {
            let Some(params) = entry.params else { continue };
            let expected = format!(
                "{}Params",
                entry
                    .method
                    .command_name()
                    .split('_')
                    .map(|word| {
                        let mut characters = word.chars();
                        match characters.next() {
                            Some(first) => {
                                first.to_ascii_uppercase().to_string() + characters.as_str()
                            }
                            None => String::new(),
                        }
                    })
                    .collect::<String>()
            );
            assert_eq!(params, expected, "{} names the wrong params type", entry.method.as_str());
        }
    }

    #[test]
    fn params_fields_are_the_wire_names_a_host_validates() {
        assert_eq!(
            TypedMethod::params_fields(MethodName::ResizeTerminal),
            Some(vec!["cols".to_string(), "rows".to_string(), "workspaceId".to_string()])
        );
        assert_eq!(
            TypedMethod::params_fields(MethodName::ResolveApproval),
            Some(vec!["decision".to_string(), "eventId".to_string(), "sessionId".to_string()])
        );
    }

    #[test]
    fn unit_returning_methods_share_one_result_contract() {
        for method in [
            MethodName::InterruptTurn,
            MethodName::CompactSession,
            MethodName::RefreshAccountUsage,
            MethodName::ResolveApproval,
            MethodName::OpenTerminal,
            MethodName::SetBrowserPermission,
            MethodName::EnableLearningTrigger,
        ] {
            assert_eq!(
                TypedMethod::for_method(method).result,
                Some("UnitResult"),
                "{} returns JSON null",
                method.as_str()
            );
        }
        let unit_schemas = payload_schemas()
            .into_iter()
            .filter(|(name, _)| *name == "UnitResult")
            .count();
        assert_eq!(unit_schemas, 1, "shared payload types are emitted once");
    }

    #[test]
    fn every_contracted_payload_has_a_schema() {
        let schemas = payload_schemas();
        for entry in TYPED_METHODS {
            for payload in [entry.params, entry.result].into_iter().flatten() {
                assert!(
                    schemas.iter().any(|(name, _)| *name == payload),
                    "{payload} is contracted but has no schema"
                );
            }
        }
    }
}
