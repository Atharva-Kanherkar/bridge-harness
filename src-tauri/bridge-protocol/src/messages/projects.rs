//! The projects domain: registering repositories Bridge can open workspaces in.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
// Published in protocol 0.4 without `additionalProperties: false`. Keep this
// shape open until the next major version so a 0.5 server remains compatible
// with every request accepted by the 0.4 schema.
#[serde(rename_all = "camelCase")]
pub struct AddProjectParams {
    /// Path to a Git repository (any path inside it resolves to the root).
    pub path: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::common::round_trip;
    use serde_json::json;

    #[test]
    fn add_project_round_trips() {
        let add = AddProjectParams { path: "/repos/demo".into() };
        assert_eq!(serde_json::to_value(&add).unwrap(), json!({"path": "/repos/demo"}));
        assert_eq!(round_trip(&add), add);
    }

    #[test]
    fn add_project_requires_a_path() {
        assert!(serde_json::from_value::<AddProjectParams>(json!({})).is_err());
    }
}
