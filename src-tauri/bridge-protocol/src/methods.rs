//! The method namespace: every request a Bridge client can make, grouped by
//! domain. Wire names are `domain/command`, where `command` is the exact
//! Tauri command name — the migration compatibility adapter maps an invoke to
//! its method by name alone.
//!
//! A test in the shell crate parses `generate_handler![...]` and asserts this
//! registry matches the registered command surface 1:1, so adding a command
//! without extending the contract (or vice versa) fails the build gates.

macro_rules! methods {
    ($(($variant:ident, $domain:literal, $command:literal)),* $(,)?) => {
        /// A method in the registry. See the module docs for naming rules.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum MethodName {
            $($variant),*
        }

        impl MethodName {
            pub const ALL: &'static [MethodName] = &[$(MethodName::$variant),*];

            /// The domain used for grouping and capability advertisement.
            pub const fn domain(self) -> &'static str {
                match self { $(MethodName::$variant => $domain),* }
            }

            /// The Tauri command this method corresponds to during migration.
            pub const fn command_name(self) -> &'static str {
                match self { $(MethodName::$variant => $command),* }
            }

            /// The wire name: `domain/command`.
            pub const fn as_str(self) -> &'static str {
                match self { $(MethodName::$variant => concat!($domain, "/", $command)),* }
            }
        }
    };
}

methods![
    // health
    (Health, "health", "health"),
    // state — the aggregate application snapshot
    (GetState, "state", "get_state"),
    // projects
    (AddProject, "projects", "add_project"),
    // workspaces
    (CreateWorkspace, "workspaces", "create_workspace"),
    (ConnectWorkspaceFolder, "workspaces", "connect_workspace_folder"),
    (ListWorkspaceFiles, "workspaces", "list_workspace_files"),
    (ListWorkspaceTree, "workspaces", "list_workspace_tree"),
    (ReadWorkspaceFile, "workspaces", "read_workspace_file"),
    (WriteWorkspaceFile, "workspaces", "write_workspace_file"),
    (RefreshWorkspace, "workspaces", "refresh_workspace"),
    (ArchiveWorkspace, "workspaces", "archive_workspace"),
    (WorkspaceChanges, "workspaces", "workspace_changes"),
    // sessions
    (GetSessionForest, "sessions", "get_session_forest"),
    (ReplaySessionEvents, "sessions", "replay_session_events"),
    (ActivateSessionEntry, "sessions", "activate_session_entry"),
    (CreateChat, "sessions", "create_chat"),
    (CreateWorkspaceSession, "sessions", "create_workspace_session"),
    (StartSession, "sessions", "start_session"),
    (StartChat, "sessions", "start_chat"),
    (UpdateChatModel, "sessions", "update_chat_model"),
    (PrepareTurn, "sessions", "prepare_turn"),
    (SendTurn, "sessions", "send_turn"),
    (SubmitInput, "sessions", "submit_input"),
    (CompactSession, "sessions", "compact_session"),
    (InterruptTurn, "sessions", "interrupt_turn"),
    (RefreshAccountUsage, "sessions", "refresh_account_usage"),
    (StopSession, "sessions", "stop_session"),
    // approvals
    (ResolveApproval, "approvals", "resolve_approval"),
    // terminal
    (OpenTerminal, "terminal", "open_terminal"),
    (WriteTerminal, "terminal", "write_terminal"),
    (ResizeTerminal, "terminal", "resize_terminal"),
    // slash commands
    (ListSlashCommands, "slash", "list_slash_commands"),
    (ResolveSlashCommand, "slash", "resolve_slash_command"),
    // completion / verification
    (CreateCompletionPlan, "completion", "create_completion_plan"),
    (RecordCompletionCheck, "completion", "record_completion_check"),
    (WaiveCompletion, "completion", "waive_completion"),
    (RegisterVerifierManifest, "completion", "register_verifier_manifest"),
    (VerifierCandidates, "completion", "verifier_candidates"),
    // base-branch divergence
    (WorkspaceBaseDivergence, "worktrees", "workspace_base_divergence"),
    (RefreshWorkspaceBase, "worktrees", "refresh_workspace_base"),
    // worker worktree adoption
    (PendingWorkerAdoptions, "worktrees", "pending_worker_adoptions"),
    (AdoptWorkerWorktree, "worktrees", "adopt_worker_worktree"),
    (DiscardWorkerWorktree, "worktrees", "discard_worker_worktree"),
    // routing
    (GetRouterPreferences, "routing", "get_router_preferences"),
    (UpdateRouterPreferences, "routing", "update_router_preferences"),
    (RollbackRoutingPolicy, "routing", "rollback_routing_policy"),
    // model profiles
    (GetModelSetup, "models", "get_model_setup"),
    (RecommendedModelProfiles, "models", "recommended_model_profiles"),
    (SaveModelProfiles, "models", "save_model_profiles"),
    (ResetModelProfiles, "models", "reset_model_profiles"),
    // configuration
    (GetConfigState, "config", "get_config_state"),
    (SaveHarnessConfig, "config", "save_harness_config"),
    (ResetHarnessConfig, "config", "reset_harness_config"),
    (RefreshOpencodeCatalog, "config", "refresh_opencode_catalog"),
    (SetOpencodeProviderApiKey, "config", "set_opencode_provider_api_key"),
    (RemoveOpencodeProviderAuth, "config", "remove_opencode_provider_auth"),
    (SaveAgentConfig, "config", "save_agent_config"),
    (DeleteAgentConfig, "config", "delete_agent_config"),
    (SetDefaultAgent, "config", "set_default_agent"),
    (ResetAllConfig, "config", "reset_all_config"),
    // adaptive learning
    (GetLearningState, "learning", "get_learning_state"),
    (RunLearning, "learning", "run_learning"),
    (CancelLearningRun, "learning", "cancel_learning_run"),
    (UpdateLearningSchedule, "learning", "update_learning_schedule"),
    (RegisterLearningTrigger, "learning", "register_learning_trigger"),
    (GetLearningTriggerInstructions, "learning", "get_learning_trigger_instructions"),
    (EnableLearningTrigger, "learning", "enable_learning_trigger"),
    (ApproveLearningRun, "learning", "approve_learning_run"),
    // browser bridge
    (BrowserBridgeState, "browser", "browser_bridge_state"),
    (InstallBrowserNativeHost, "browser", "install_browser_native_host"),
    (BrowserAction, "browser", "browser_action"),
    (SetBrowserPermission, "browser", "set_browser_permission"),
    (ResolveBrowserApproval, "browser", "resolve_browser_approval"),
    (TakeoverBrowser, "browser", "takeover_browser"),
    (DetachBrowser, "browser", "detach_browser"),
    (RouteBrowser, "browser", "route_browser"),
    (BrowserSkills, "browser", "browser_skills"),
    (ConfigureRemoteBrowser, "browser", "configure_remote_browser"),
    (StartRemoteBrowser, "browser", "start_remote_browser"),
    // agents — the runtime lifecycle for the built-in integrations. Distinct
    // from `marketplace`, which is about plugins running inside an agent.
    (ListManagedAgents, "agents", "list_managed_agents"),
    (InspectManagedAgent, "agents", "inspect_managed_agent"),
    (InstallManagedAgent, "agents", "install_managed_agent"),
    (RepairManagedAgent, "agents", "repair_managed_agent"),
    (UninstallManagedAgent, "agents", "uninstall_managed_agent"),
    // marketplace
    (MarketplaceCatalog, "marketplace", "marketplace_catalog"),
    (MarketplaceAppAuthStates, "marketplace", "marketplace_app_auth_states"),
    (MarketplaceAction, "marketplace", "marketplace_action"),
    // work — the ranked board of what needs doing. Facts are store-only, so
    // this method reads SQLite and starts nothing.
    (GetWorkBoard, "work", "get_work_board"),
    (WorkTaskAction, "work", "task_action"),
    (WorkTaskPin, "work", "task_pin"),
    (WorkTaskPrepareSession, "work", "task_prepare_session"),
    (WorkTaskOpenEvidence, "work", "task_open_evidence"),
    // work settings and the briefing surface. Reading and writing settings is
    // what makes a briefing reachable from a fresh install at all; the options
    // method reports which harnesses passed the conformance gate and why the
    // others were refused.
    (ReadWorkSettings, "work", "read_settings"),
    (WriteWorkSettings, "work", "write_settings"),
    (WorkBriefingOptions, "work", "briefing_options"),
    // triggers. Both funnel through the one claim path in bridge-core; the
    // receipt says whether this call started the run, observed somebody
    // else's, or was refused with a stable code.
    (RunWorkBriefing, "work", "run_briefing"),
    (CancelWorkBriefing, "work", "cancel_briefing"),
    // skills
    (SkillCatalog, "skills", "skill_catalog"),
    (SkillSuggestions, "skills", "skill_suggestions"),
    (PreviewSkillChange, "skills", "preview_skill_change"),
    (ExecuteSkillChange, "skills", "execute_skill_change"),
];

impl MethodName {
    /// Parse a wire name (`domain/command`).
    pub fn parse(method: &str) -> Option<MethodName> {
        MethodName::ALL.iter().copied().find(|candidate| candidate.as_str() == method)
    }

    /// Look up the method for a Tauri command name.
    pub fn from_command(command: &str) -> Option<MethodName> {
        MethodName::ALL.iter().copied().find(|candidate| candidate.command_name() == command)
    }

    /// All domains, sorted and deduplicated — the server's capability list.
    pub fn domains() -> Vec<&'static str> {
        let mut domains: Vec<&'static str> =
            MethodName::ALL.iter().map(|method| method.domain()).collect();
        domains.sort_unstable();
        domains.dedup();
        domains
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn wire_names_and_commands_are_unique_and_parse_back() {
        let mut wire = HashSet::new();
        let mut commands = HashSet::new();
        for method in MethodName::ALL.iter().copied() {
            assert!(wire.insert(method.as_str()), "duplicate wire name {}", method.as_str());
            assert!(
                commands.insert(method.command_name()),
                "duplicate command {}",
                method.command_name()
            );
            assert_eq!(MethodName::parse(method.as_str()), Some(method));
            assert_eq!(MethodName::from_command(method.command_name()), Some(method));
            assert_eq!(
                method.as_str(),
                format!("{}/{}", method.domain(), method.command_name())
            );
        }
        assert_eq!(MethodName::parse("no-such/method"), None);
        assert_eq!(MethodName::from_command("no_such_command"), None);
    }

    #[test]
    fn reserved_names_stay_outside_the_registry() {
        assert_eq!(MethodName::parse(crate::HANDSHAKE_METHOD), None);
        assert_eq!(MethodName::parse(crate::CANCEL_METHOD), None);
    }

    #[test]
    fn domains_are_sorted_and_deduplicated() {
        let domains = MethodName::domains();
        let mut sorted = domains.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(domains, sorted);
        assert!(domains.contains(&"sessions"));
        assert!(domains.contains(&"terminal"));
        assert!(domains.contains(&"approvals"));
    }
}
