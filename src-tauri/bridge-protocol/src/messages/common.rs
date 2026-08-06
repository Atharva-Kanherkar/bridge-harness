//! Wire types more than one domain needs.

use schemars::gen::SchemaGenerator;
use schemars::schema::Schema;
use schemars::JsonSchema;
use serde::de::{self, Deserializer};
use serde::{Deserialize, Serialize};

use crate::MAX_SAFE_INTEGER;

/// A signed integer that can cross a JavaScript boundary without losing bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct JsSafeI64(i64);

impl JsSafeI64 {
    pub fn new(value: i64) -> Result<Self, &'static str> {
        if (-MAX_SAFE_INTEGER..=MAX_SAFE_INTEGER).contains(&value) {
            Ok(Self(value))
        } else {
            Err("integer exceeds JavaScript's safe integer range")
        }
    }

    pub fn get(self) -> i64 {
        self.0
    }
}

impl TryFrom<i64> for JsSafeI64 {
    type Error = &'static str;

    fn try_from(value: i64) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl<'de> Deserialize<'de> for JsSafeI64 {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::new(i64::deserialize(deserializer)?).map_err(de::Error::custom)
    }
}

impl JsonSchema for JsSafeI64 {
    fn schema_name() -> String {
        "JsSafeI64".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        serde_json::from_value(serde_json::json!({
            "type": "integer",
            "minimum": -MAX_SAFE_INTEGER,
            "maximum": MAX_SAFE_INTEGER,
        }))
        .unwrap()
    }
}

/// A non-negative integer that can cross a JavaScript boundary without losing bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct JsSafeU64(u64);

impl JsSafeU64 {
    pub fn new(value: u64) -> Result<Self, &'static str> {
        if value <= MAX_SAFE_INTEGER as u64 {
            Ok(Self(value))
        } else {
            Err("integer exceeds JavaScript's safe integer range")
        }
    }

    pub fn get(self) -> u64 {
        self.0
    }
}

impl TryFrom<u64> for JsSafeU64 {
    type Error = &'static str;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl<'de> Deserialize<'de> for JsSafeU64 {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::new(u64::deserialize(deserializer)?).map_err(de::Error::custom)
    }
}

impl JsonSchema for JsSafeU64 {
    fn schema_name() -> String {
        "JsSafeU64".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        serde_json::from_value(serde_json::json!({
            "type": "integer",
            "minimum": 0,
            "maximum": MAX_SAFE_INTEGER,
        }))
        .unwrap()
    }
}

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

    #[test]
    fn javascript_safe_integers_enforce_both_boundaries() {
        for value in [-MAX_SAFE_INTEGER, 0, MAX_SAFE_INTEGER] {
            let safe = serde_json::from_value::<JsSafeI64>(serde_json::json!(value)).unwrap();
            assert_eq!(safe.get(), value);
        }
        for value in [0, MAX_SAFE_INTEGER as u64] {
            let safe = serde_json::from_value::<JsSafeU64>(serde_json::json!(value)).unwrap();
            assert_eq!(safe.get(), value);
        }
        for value in [MAX_SAFE_INTEGER + 1, -(MAX_SAFE_INTEGER + 1)] {
            let error = serde_json::from_value::<JsSafeI64>(serde_json::json!(value)).unwrap_err();
            assert!(error.to_string().contains("safe integer"), "{error}");
        }
        let error = serde_json::from_value::<JsSafeU64>(serde_json::json!(
            MAX_SAFE_INTEGER as u64 + 1
        ))
        .unwrap_err();
        assert!(error.to_string().contains("safe integer"), "{error}");
        assert!(serde_json::from_value::<JsSafeU64>(serde_json::json!(-1)).is_err());

        let signed_schema = serde_json::to_value(schemars::schema_for!(JsSafeI64)).unwrap();
        assert_eq!(signed_schema["minimum"].as_f64(), Some(-(MAX_SAFE_INTEGER as f64)));
        assert_eq!(signed_schema["maximum"].as_f64(), Some(MAX_SAFE_INTEGER as f64));
        let unsigned_schema = serde_json::to_value(schemars::schema_for!(JsSafeU64)).unwrap();
        assert_eq!(unsigned_schema["minimum"].as_f64(), Some(0.0));
        assert_eq!(unsigned_schema["maximum"].as_f64(), Some(MAX_SAFE_INTEGER as f64));
    }
}
