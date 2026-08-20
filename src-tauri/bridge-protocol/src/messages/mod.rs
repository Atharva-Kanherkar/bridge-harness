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
//! Params structs first contracted in protocol 0.5 refuse unknown fields. The
//! 19 params schemas published before 0.5 stay open until the next major
//! version: minor versions are additive, so a 0.5 server must continue to
//! accept every document the 0.4 schemas allowed.
//!
//! Results are contracted wherever a wire DTO exists — including the
//! aggregate `BridgeState` and `SessionForestSnapshot` trees. The remaining
//! domain snapshots are **documented exceptions** in [`DEFERRED_RESULTS`],
//! each naming the core type still to be mirrored; a method missing from both
//! tables fails the coverage test below.

mod agents;
mod approvals;
mod browser;
mod common;
mod completion;
mod config;
mod forest;
mod learning;
mod marketplace;
mod models;
mod projects;
mod routing;
mod sessions;
mod skills;
mod slash;
mod state;
mod terminal;
mod work;
mod workspaces;

pub use agents::*;
pub use approvals::*;
pub use browser::*;
pub use common::*;
pub use completion::*;
pub use config::*;
pub use forest::*;
pub use learning::*;
pub use marketplace::*;
pub use models::*;
pub use projects::*;
pub use routing::*;
pub use sessions::*;
pub use skills::*;
pub use slash::*;
pub use state::*;
pub use terminal::*;
pub use work::*;
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
    (Health, _, HealthResult),
    // state — the aggregate application snapshot
    (GetState, _, BridgeState),
    // projects
    (AddProject, AddProjectParams, BridgeState),
    // workspaces
    (CreateWorkspace, CreateWorkspaceParams, BridgeState),
    (ConnectWorkspaceFolder, ConnectWorkspaceFolderParams, BridgeState),
    (ListWorkspaceFiles, ListWorkspaceFilesParams, ListWorkspaceFilesResult),
    (ListWorkspaceTree, ListWorkspaceTreeParams, ListWorkspaceTreeResult),
    (ReadWorkspaceFile, ReadWorkspaceFileParams, ReadWorkspaceFileResult),
    (WriteWorkspaceFile, WriteWorkspaceFileParams, WriteWorkspaceFileResult),
    (RefreshWorkspace, RefreshWorkspaceParams, BridgeState),
    (ArchiveWorkspace, ArchiveWorkspaceParams, BridgeState),
    (WorkspaceChanges, WorkspaceChangesParams, WorkspaceChangesResult),
    // sessions
    (GetSessionForest, GetSessionForestParams, SessionForestSnapshot),
    (ReplaySessionEvents, ReplaySessionEventsParams, ReplaySessionEventsResult),
    (ActivateSessionEntry, ActivateSessionEntryParams, SessionForestSnapshot),
    (CreateChat, CreateChatParams, BridgeState),
    (CreateWorkspaceSession, CreateWorkspaceSessionParams, BridgeState),
    (StartSession, StartSessionParams, BridgeState),
    (StartChat, StartChatParams, BridgeState),
    (UpdateChatModel, UpdateChatModelParams, BridgeState),
    (PrepareTurn, PrepareTurnParams, SanitizedTurn),
    (SendTurn, SendTurnParams, UnitResult),
    (CompactSession, CompactSessionParams, UnitResult),
    (SearchSessionEntries, SearchSessionEntriesParams, SearchSessionEntriesResult),
    (InterruptTurn, InterruptTurnParams, UnitResult),
    (RefreshAccountUsage, _, UnitResult),
    (StopSession, StopSessionParams, BridgeState),
    // approvals
    (ResolveApproval, ResolveApprovalParams, UnitResult),
    // terminal
    (OpenTerminal, OpenTerminalParams, UnitResult),
    (WriteTerminal, WriteTerminalParams, UnitResult),
    (ResizeTerminal, ResizeTerminalParams, UnitResult),
    // slash commands
    (ListSlashCommands, _, SlashCommandsResult),
    (ResolveSlashCommand, ResolveSlashCommandParams, SlashCommandResolveResult),
    // completion / verification
    (CreateCompletionPlan, CreateCompletionPlanParams, CompletionSummary),
    (RecordCompletionCheck, RecordCompletionCheckParams, CompletionSummary),
    (WaiveCompletion, WaiveCompletionParams, CompletionSummary),
    (RegisterVerifierManifest, RegisterVerifierManifestParams, UnitResult),
    (VerifierCandidates, VerifierCandidatesParams, VerifierCandidatesResult),
    // base-branch divergence
    (WorkspaceBaseDivergence, WorkspaceBaseDivergenceParams, BaseBranchDivergence),
    (RefreshWorkspaceBase, RefreshWorkspaceBaseParams, BaseBranchDivergence),
    // worker worktree adoption
    (PendingWorkerAdoptions, PendingWorkerAdoptionsParams, PendingWorkerAdoptionsResult),
    (AdoptWorkerWorktree, AdoptWorkerWorktreeParams, WorkerRepositoryBinding),
    (DiscardWorkerWorktree, DiscardWorkerWorktreeParams, WorkerRepositoryBinding),
    // routing
    (GetRouterPreferences, GetRouterPreferencesParams, RouterPreferences),
    (UpdateRouterPreferences, UpdateRouterPreferencesParams, RouterPreferences),
    (RollbackRoutingPolicy, RollbackRoutingPolicyParams, _),
    // model profiles
    (GetModelSetup, _, _),
    (RecommendedModelProfiles, _, RecommendedModelProfilesResult),
    (SaveModelProfiles, SaveModelProfilesParams, _),
    (ResetModelProfiles, _, _),
    // configuration
    (GetConfigState, _, ConfigState),
    (SaveHarnessConfig, SaveHarnessConfigParams, ConfigState),
    (ResetHarnessConfig, ResetHarnessConfigParams, ConfigState),
    (RefreshOpencodeCatalog, RefreshOpencodeCatalogParams, _),
    (SetOpencodeProviderApiKey, SetOpencodeProviderApiKeyParams, _),
    (RemoveOpencodeProviderAuth, RemoveOpencodeProviderAuthParams, _),
    (SaveAgentConfig, SaveAgentConfigParams, ConfigState),
    (DeleteAgentConfig, DeleteAgentConfigParams, ConfigState),
    (SetDefaultAgent, SetDefaultAgentParams, ConfigState),
    (ResetAllConfig, _, ConfigState),
    // adaptive learning
    (GetLearningState, GetLearningStateParams, _),
    (RunLearning, RunLearningParams, _),
    (CancelLearningRun, CancelLearningRunParams, _),
    (UpdateLearningSchedule, UpdateLearningScheduleParams, LearningSchedule),
    (RegisterLearningTrigger, RegisterLearningTriggerParams, UnitResult),
    (GetLearningTriggerInstructions, GetLearningTriggerInstructionsParams, LearningTriggerInstructionsResult),
    (EnableLearningTrigger, EnableLearningTriggerParams, UnitResult),
    (ApproveLearningRun, ApproveLearningRunParams, _),
    // browser bridge
    (BrowserBridgeState, _, _),
    (InstallBrowserNativeHost, _, InstallBrowserNativeHostResult),
    (BrowserAction, BrowserActionParams, BrowserActionResult),
    (SetBrowserPermission, SetBrowserPermissionParams, UnitResult),
    (ResolveBrowserApproval, ResolveBrowserApprovalParams, UnitResult),
    (TakeoverBrowser, _, UnitResult),
    (DetachBrowser, _, DetachBrowserResult),
    (RouteBrowser, RouteBrowserParams, BrowserRouteDecision),
    (BrowserSkills, _, BrowserSkillsResult),
    (ConfigureRemoteBrowser, ConfigureRemoteBrowserParams, UnitResult),
    (StartRemoteBrowser, StartRemoteBrowserParams, _),
    // marketplace
    // agents — whether an agent's runtime is installed at all
    (ListManagedAgents, _, ManagedAgentList),
    (InspectManagedAgent, InspectManagedAgentParams, ManagedAgentInspection),
    (InstallManagedAgent, InstallManagedAgentParams, ManagedAgentOperationResult),
    (RepairManagedAgent, RepairManagedAgentParams, ManagedAgentOperationResult),
    (UninstallManagedAgent, UninstallManagedAgentParams, ManagedAgentOperationResult),
    (MarketplaceCatalog, _, _),
    (MarketplaceAppAuthStates, _, _),
    (MarketplaceAction, MarketplaceActionParams, _),
    // work
    (GetWorkBoard, _, WorkBoard),
    (WorkTaskAction, TaskActionParams, UnitResult),
    (WorkTaskPin, TaskPinParams, UnitResult),
    (WorkTaskPrepareSession, TaskPrepareSessionParams, WorkTaskDraft),
    (WorkTaskOpenEvidence, TaskOpenEvidenceParams, WorkEvidenceTarget),
    (ReadWorkSettings, _, WorkSettingsSnapshot),
    (WriteWorkSettings, WriteSettingsParams, WorkSettingsSnapshot),
    (WorkBriefingOptions, _, WorkBriefingOptions),
    (RunWorkBriefing, RunBriefingParams, WorkBriefReceipt),
    (CancelWorkBriefing, _, WorkBriefReceipt),
    // skills
    (SkillCatalog, _, _),
    (SkillSuggestions, SkillSuggestionsParams, _),
    (PreviewSkillChange, PreviewSkillChangeParams, _),
    (ExecuteSkillChange, ExecuteSkillChangeParams, _),
];

/// The documented exceptions to result typing: every method whose result is
/// deliberately not yet contracted, with the core type a future slice must
/// mirror. The registry publishes these as `resultDeferred`, so non-TypeScript
/// clients can tell "intentionally untyped, shape is this named core DTO"
/// apart from "someone forgot". A method appearing in neither this table nor
/// with a typed result fails the coverage test.
pub const DEFERRED_RESULTS: &[(MethodName, &str)] = &[
    (MethodName::RollbackRoutingPolicy, "bridge_core::learning_job::LearningState"),
    (MethodName::GetModelSetup, "bridge_core::model_profiles::ModelSetupState"),
    (MethodName::SaveModelProfiles, "bridge_core::model_profiles::ModelSetupState"),
    (MethodName::ResetModelProfiles, "bridge_core::model_profiles::ModelSetupState"),
    (MethodName::RefreshOpencodeCatalog, "bridge_core::opencode_adapter::OpenCodeCatalog"),
    (MethodName::SetOpencodeProviderApiKey, "bridge_core::opencode_adapter::OpenCodeCatalog"),
    (MethodName::RemoveOpencodeProviderAuth, "bridge_core::opencode_adapter::OpenCodeCatalog"),
    (MethodName::GetLearningState, "bridge_core::learning_job::LearningState"),
    (MethodName::RunLearning, "bridge_core::learning_job::LearningRun"),
    (MethodName::CancelLearningRun, "bridge_core::learning_job::LearningRun"),
    (MethodName::ApproveLearningRun, "bridge_core::learning_job::LearningRun"),
    (MethodName::BrowserBridgeState, "bridge_core::browser_bridge::BrowserBridgeSnapshot"),
    (MethodName::StartRemoteBrowser, "provider-defined remote browser session descriptor"),
    (MethodName::MarketplaceCatalog, "bridge_core::marketplace::MarketplaceCatalog"),
    (MethodName::MarketplaceAppAuthStates, "Vec<bridge_core::marketplace::MarketplaceAppAuthState>"),
    (MethodName::MarketplaceAction, "bridge_core::marketplace::MarketplaceActionResult"),
    (MethodName::SkillCatalog, "bridge_core::skill_marketplace::SkillCatalog"),
    (MethodName::SkillSuggestions, "Vec<bridge_core::skill_marketplace::CapabilitySuggestion>"),
    (MethodName::PreviewSkillChange, "bridge_core::skill_marketplace::SkillPreview"),
    (MethodName::ExecuteSkillChange, "Vec<bridge_core::skill_marketplace::SkillActionResult>"),
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
        Some(
            Self::params_schema_fields(method)?
                .into_iter()
                .map(|(name, _)| name)
                .collect(),
        )
    }

    /// The wire fields and JSON Schema fragments for a method's params,
    /// sorted by field name. The shell's drift gate uses the fragments to
    /// compare both argument names and their Rust/JSON types.
    pub fn params_schema_fields(method: MethodName) -> Option<Vec<(String, Value)>> {
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
        Some(
            properties
                .iter()
                .map(|(field, schema)| (field.clone(), schema.clone()))
                .collect(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    const OPEN_UNTIL_NEXT_MAJOR: &[&str] = &[
        "AddProjectParams",
        "CreateWorkspaceParams",
        "ConnectWorkspaceFolderParams",
        "ListWorkspaceFilesParams",
        "RefreshWorkspaceParams",
        "ArchiveWorkspaceParams",
        "GetSessionForestParams",
        "ActivateSessionEntryParams",
        "CreateChatParams",
        "CreateWorkspaceSessionParams",
        "UpdateChatModelParams",
        "ReplaySessionEventsParams",
        "StartSessionParams",
        "StartChatParams",
        "PrepareTurnParams",
        "SendTurnParams",
        "StopSessionParams",
        "InterruptTurnParams",
        "CompactSessionParams",
    ];

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
    fn every_result_is_typed_or_a_documented_exception() {
        let deferred: HashSet<&str> = DEFERRED_RESULTS
            .iter()
            .map(|(method, _)| method.as_str())
            .collect();
        assert_eq!(
            deferred.len(),
            DEFERRED_RESULTS.len(),
            "a method may be deferred only once"
        );
        for entry in TYPED_METHODS {
            let typed = entry.result.is_some();
            let excused = deferred.contains(entry.method.as_str());
            assert!(
                typed != excused,
                "{} must have exactly one of a typed result or a documented \
                 exception in DEFERRED_RESULTS (typed: {typed}, deferred: {excused})",
                entry.method.as_str()
            );
        }
        for (_, reason) in DEFERRED_RESULTS {
            assert!(
                !reason.trim().is_empty(),
                "every deferred result names the core type still to be mirrored"
            );
        }
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
            MethodName::GetWorkBoard,
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

    #[test]
    fn params_schema_strictness_preserves_minor_version_compatibility() {
        let schemas = payload_schemas();
        for entry in TYPED_METHODS {
            let Some(params) = entry.params else { continue };
            let schema = &schemas
                .iter()
                .find(|(name, _)| *name == params)
                .unwrap_or_else(|| panic!("{params} has no schema"))
                .1;
            let additional = schema.get("additionalProperties");
            if OPEN_UNTIL_NEXT_MAJOR.contains(&params) {
                assert!(
                    additional.is_none(),
                    "{params} was published open in protocol 0.4 and cannot close in a minor bump"
                );
            } else {
                assert_eq!(
                    additional,
                    Some(&Value::Bool(false)),
                    "{params} was first contracted in 0.5 and must reject unknown fields"
                );
            }
        }
    }
}
