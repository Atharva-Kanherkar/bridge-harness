//! Create and start through the methods used by this revision's composer.
//! CI keeps this file from each revision while sharing the timed workload.

use crate::{api, BridgeCore, BridgeError};
use bridge_protocol::messages::{
    ChatDefaults, ChatLaunchTarget, ChatModelChoice, CreateConfiguredChatParams,
    ResolveChatSettingsParams,
};
use std::sync::Arc;

pub(super) fn create_params(
    core: &Arc<BridgeCore>,
    workspace: &str,
) -> Result<CreateConfiguredChatParams, BridgeError> {
    let resolve = ResolveChatSettingsParams {
        workspace_id: Some(workspace.into()),
        overrides: ChatDefaults {
            target: Some(ChatLaunchTarget::Direct),
            model: Some(ChatModelChoice {
                harness: "codex".into(),
                model: Some("standard".into()),
                account: None,
            }),
            create_worktree: Some(false),
            ..Default::default()
        },
        expected_revision: None,
    };
    let settings = api::resolve_chat_settings(core, &resolve)?;
    Ok(CreateConfiguredChatParams {
        workspace_id: resolve.workspace_id,
        overrides: resolve.overrides,
        expected_revision: settings.revision,
        title: None,
        tool_selection: None,
    })
}

pub(super) fn create(
    core: &Arc<BridgeCore>,
    params: &CreateConfiguredChatParams,
) -> Result<String, BridgeError> {
    Ok(api::create_configured_chat(core, params)?.session_id)
}

pub(super) fn start(core: &Arc<BridgeCore>, session: String) -> Result<(), BridgeError> {
    api::start_chat(core, session)?;
    Ok(())
}
