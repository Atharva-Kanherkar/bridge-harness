//! The slash-command domain: discovering and expanding `/command` input.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolveSlashCommandParams {
    /// The composer text to resolve; a non-slash string resolves to nothing.
    pub text: String,
    pub session_id: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::common::round_trip;
    use serde_json::json;

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
