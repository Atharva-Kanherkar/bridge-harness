//! Wire types more than one domain needs.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A reasoning-effort level on the wire. Mirrors `bridge_core::delegation::Effort`
/// variant for variant; a mirror test in bridge-core keeps the two from drifting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Effort {
    Low,
    Medium,
    High,
    Xhigh,
}

/// Successful result for commands that return no value. JSON-RPC carries Rust
/// unit as an explicit `null` result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct UnitResult(pub ());

/// Serialize and deserialize a payload, so a test asserts on exactly what the
/// wire carries rather than on the Rust value it started from.
#[cfg(test)]
pub(crate) fn round_trip<T>(value: &T) -> T
where
    T: Serialize + for<'de> Deserialize<'de>,
{
    serde_json::from_str(&serde_json::to_string(value).unwrap()).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_results_are_explicit_json_null() {
        assert_eq!(serde_json::to_value(UnitResult(())).unwrap(), serde_json::Value::Null);
        assert_eq!(round_trip(&UnitResult(())), UnitResult(()));
    }

    #[test]
    fn effort_levels_are_lowercase_on_the_wire() {
        assert_eq!(serde_json::to_value(Effort::Xhigh).unwrap(), serde_json::json!("xhigh"));
        assert_eq!(round_trip(&Effort::Medium), Effort::Medium);
        assert!(serde_json::from_value::<Effort>(serde_json::json!("Xhigh")).is_err());
        assert!(serde_json::from_value::<Effort>(serde_json::json!("extreme")).is_err());
    }
}
