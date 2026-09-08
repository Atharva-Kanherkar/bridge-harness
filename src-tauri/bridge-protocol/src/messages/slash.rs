//! The slash-command domain: discovering and expanding `/command` input.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ListSlashCommandsParams {
    /// Optional because Bridge can still list global capabilities before a chat exists.
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolveSlashCommandParams {
    /// The composer text to resolve; a non-slash string resolves to nothing.
    pub text: String,
    pub session_id: String,
}

/// Mirrors `bridge_core::slash::SlashCommand` — one entry of the `/` menu.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SlashCommand {
    pub name: String,
    pub description: String,
    pub harness: String,
    pub kind: String,
}

/// `slash/list_slash_commands`' result: a bare array on the wire.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct SlashCommandsResult(pub Vec<SlashCommand>);

/// Mirrors `bridge_core::api::SlashCommandResolve`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SlashCommandResolve {
    pub name: String,
    pub harness: String,
    pub kind: String,
    /// When true, the frontend should switch the direct chat to `harness`
    /// before sending.
    pub switch_harness: bool,
}

/// `slash/resolve_slash_command`'s result: the resolution, or `null` when the
/// text is not a known slash command.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct SlashCommandResolveResult(pub Option<SlashCommandResolve>);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::common::round_trip;
    use serde_json::json;

    #[test]
    fn slash_results_round_trip_and_resolve_may_be_null() {
        let commands = SlashCommandsResult(vec![SlashCommand {
            name: "review".into(),
            description: "Review the diff".into(),
            harness: "claude".into(),
            kind: "skill".into(),
        }]);
        assert_eq!(
            serde_json::to_value(&commands).unwrap(),
            json!([{"name": "review", "description": "Review the diff", "harness": "claude", "kind": "skill"}])
        );
        assert_eq!(round_trip(&commands), commands);

        let resolved = SlashCommandResolveResult(Some(SlashCommandResolve {
            name: "review".into(),
            harness: "claude".into(),
            kind: "skill".into(),
            switch_harness: true,
        }));
        assert_eq!(
            serde_json::to_value(&resolved).unwrap()["switchHarness"],
            json!(true)
        );
        assert_eq!(round_trip(&resolved), resolved);

        let unresolved = SlashCommandResolveResult(None);
        assert_eq!(serde_json::to_value(&unresolved).unwrap(), json!(null));
        assert_eq!(round_trip(&unresolved), unresolved);
    }

    #[test]
    fn resolve_slash_command_round_trips() {
        let resolve =
            ResolveSlashCommandParams { text: "/review".into(), session_id: "s-1".into() };
        assert_eq!(
            serde_json::to_value(&resolve).unwrap(),
            json!({"text": "/review", "sessionId": "s-1"})
        );
        assert_eq!(round_trip(&resolve), resolve);
    }

    #[test]
    fn slash_listing_may_be_global_or_scoped_to_a_session() {
        let global = ListSlashCommandsParams { session_id: None };
        assert_eq!(serde_json::to_value(&global).unwrap(), json!({"sessionId": null}));
        assert_eq!(round_trip(&global), global);

        let scoped = ListSlashCommandsParams { session_id: Some("s-1".into()) };
        assert_eq!(
            serde_json::to_value(&scoped).unwrap(),
            json!({"sessionId": "s-1"})
        );
        assert_eq!(round_trip(&scoped), scoped);
    }

    #[test]
    fn resolve_slash_command_requires_both_fields() {
        assert!(serde_json::from_value::<ResolveSlashCommandParams>(json!({})).is_err());
        assert!(serde_json::from_value::<ResolveSlashCommandParams>(json!({"text": "/x"})).is_err());
        assert!(
            serde_json::from_value::<ResolveSlashCommandParams>(
                json!({"text": "/x", "session_id": "s"})
            )
            .is_err(),
            "wire names are camelCase"
        );
    }
}
