//! Local interception for high-confidence credentials pasted into chat turns.
//!
//! This module deliberately produces only sanitized text and opaque metadata.
//! It does not persist or resolve the matched credential values.

use regex::Regex;
use serde::Serialize;
use std::{collections::HashMap, sync::LazyLock};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SecretInterception {
    pub reference: String,
    pub detector: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SanitizedTurn {
    pub text: String,
    pub interceptions: Vec<SecretInterception>,
}

struct Detector {
    name: &'static str,
    pattern: Regex,
}

impl Detector {
    fn new(name: &'static str, pattern: &str) -> Self {
        Self {
            name,
            pattern: Regex::new(pattern).expect("secret detector regex must compile"),
        }
    }
}

static DETECTORS: LazyLock<Vec<Detector>> = LazyLock::new(|| {
    vec![
        Detector::new(
            "private_key",
            r"(?s)(?P<secret>-----BEGIN (?:RSA |EC |OPENSSH |DSA )?PRIVATE KEY-----.*?-----END (?:RSA |EC |OPENSSH |DSA )?PRIVATE KEY-----)",
        ),
        Detector::new(
            "github",
            r"(?:^|[^A-Za-z0-9])(?P<secret>(?:github_pat_[A-Za-z0-9_]{20,255}|gh[pousr]_[A-Za-z0-9]{36,255}))",
        ),
        Detector::new(
            "slack",
            r"(?:^|[^A-Za-z0-9])(?P<secret>xox[baprs]-[A-Za-z0-9-]{10,255})",
        ),
        Detector::new(
            "anthropic",
            r"(?:^|[^A-Za-z0-9])(?P<secret>sk-ant-[A-Za-z0-9_-]{20,255})",
        ),
        Detector::new(
            "openai",
            r"(?:^|[^A-Za-z0-9])(?P<secret>sk-(?:proj-)?[A-Za-z0-9_-]{20,255})",
        ),
        Detector::new(
            "notion",
            r"(?:^|[^A-Za-z0-9])(?P<secret>secret_[A-Za-z0-9]{20,255})",
        ),
        Detector::new(
            "aws_access_key",
            r"(?:^|[^A-Z0-9])(?P<secret>(?:AKIA|ASIA)[A-Z0-9]{16})(?:$|[^A-Z0-9])",
        ),
        Detector::new(
            "bearer",
            r"(?i)\bbearer[ \t]+(?P<secret>[A-Za-z0-9._~+/=-]{16,2048})",
        ),
        Detector::new(
            "jwt",
            r"(?:^|[^A-Za-z0-9_-])(?P<secret>[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,})(?:$|[^A-Za-z0-9_-])",
        ),
        Detector::new(
            "credential_assignment",
            r#"(?x)\b(?:[A-Z0-9_]*(?:API[_-]?KEY|TOKEN|SECRET|PASSWORD|CREDENTIAL)[A-Z0-9_]*)\s*[:=]\s*["']?(?P<secret>[A-Za-z0-9._~+/=-]{12,2048})["']?"#,
        ),
    ]
});

#[derive(Clone)]
struct Match {
    start: usize,
    end: usize,
    detector: &'static str,
}

/// Replace credential-shaped values with opaque per-message references.
///
/// References are random rather than value-derived so metadata cannot be used
/// to correlate or brute-force intercepted credentials.
pub fn sanitize(text: &str) -> SanitizedTurn {
    let mut matches = Vec::new();
    for detector in DETECTORS.iter() {
        for captures in detector.pattern.captures_iter(text) {
            if let Some(secret) = captures.name("secret") {
                matches.push(Match {
                    start: secret.start(),
                    end: secret.end(),
                    detector: detector.name,
                });
            }
        }
    }
    if matches.is_empty() {
        return SanitizedTurn {
            text: text.to_owned(),
            interceptions: Vec::new(),
        };
    }

    // Prefer the widest match when detectors overlap, then process left-to-right.
    matches.sort_by(|left, right| {
        left.start
            .cmp(&right.start)
            .then_with(|| right.end.cmp(&left.end))
    });
    let mut selected: Vec<Match> = Vec::new();
    for candidate in matches {
        if selected
            .last()
            .is_some_and(|existing| candidate.start < existing.end)
        {
            continue;
        }
        selected.push(candidate);
    }

    let mut references: HashMap<&str, (String, &'static str)> = HashMap::new();
    let mut output = String::with_capacity(text.len());
    let mut cursor = 0;
    for matched in selected {
        output.push_str(&text[cursor..matched.start]);
        let value = &text[matched.start..matched.end];
        let (reference, _) = references
            .entry(value)
            .or_insert_with(|| (format!("sec_{}", Uuid::new_v4().simple()), matched.detector));
        output.push_str("[secret:");
        output.push_str(reference);
        output.push(']');
        cursor = matched.end;
    }
    output.push_str(&text[cursor..]);

    let mut interceptions = references
        .into_values()
        .map(|(reference, detector)| SecretInterception {
            reference,
            detector: detector.into(),
        })
        .collect::<Vec<_>>();
    interceptions.sort_by(|left, right| left.reference.cmp(&right.reference));
    SanitizedTurn {
        text: output,
        interceptions,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_intercepted(detector: &str, secret: &str) {
        let input = format!("use {secret} for this request");
        let sanitized = sanitize(&input);
        assert!(!sanitized.text.contains(secret));
        assert!(sanitized.text.contains("[secret:sec_"));
        assert!(sanitized
            .interceptions
            .iter()
            .any(|item| item.detector == detector));
    }

    #[test]
    fn intercepts_supported_high_confidence_credentials() {
        assert_intercepted("github", "ghp_abcdefghijklmnopqrstuvwxyzABCDEFGHIJ");
        assert_intercepted("github", "github_pat_11AA22bb33CC44dd55EE66ff77GG88hh");
        assert_intercepted("slack", "xoxb-123456789012-abcdefghijklmnop");
        assert_intercepted("notion", "secret_abcdefghijklmnopqrstuvwx");
        assert_intercepted("openai", "sk-proj-abcdefghijklmnopqrstuvwxyz123456");
        assert_intercepted("anthropic", "sk-ant-abcdefghijklmnopqrstuvwxyz123456");
        assert_intercepted("aws_access_key", "AKIAABCDEFGHIJKLMNOP");
        assert_intercepted(
            "jwt",
            "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.signature12345",
        );
    }

    #[test]
    fn intercepts_bearer_and_assignment_values_without_removing_context() {
        let bearer = "abcdefghijklmnop123456";
        let assigned = "0123456789abcdefghijklmnop";
        let input = format!("Authorization: Bearer {bearer}\nGITHUB_TOKEN={assigned}");
        let sanitized = sanitize(&input);
        assert!(sanitized
            .text
            .starts_with("Authorization: Bearer [secret:sec_"));
        assert!(sanitized.text.contains("GITHUB_TOKEN=[secret:sec_"));
        assert!(!sanitized.text.contains(bearer));
        assert!(!sanitized.text.contains(assigned));
        assert_eq!(sanitized.interceptions.len(), 2);
    }

    #[test]
    fn intercepts_a_multiline_private_key_as_one_value() {
        let secret = "-----BEGIN PRIVATE KEY-----\nabc123+/=\ndef456+/=\n-----END PRIVATE KEY-----";
        let sanitized = sanitize(&format!("sign with:\n{secret}\nthanks"));
        assert_eq!(sanitized.interceptions.len(), 1);
        assert_eq!(sanitized.interceptions[0].detector, "private_key");
        assert!(!sanitized.text.contains("abc123"));
        assert!(sanitized.text.ends_with("\nthanks"));
    }

    #[test]
    fn reuses_a_reference_for_repeated_values() {
        let secret = "ghp_abcdefghijklmnopqrstuvwxyzABCDEFGHIJ";
        let sanitized = sanitize(&format!("first {secret}; second {secret}"));
        assert_eq!(sanitized.interceptions.len(), 1);
        assert_eq!(sanitized.text.matches("[secret:sec_").count(), 2);
        let reference = &sanitized.interceptions[0].reference;
        assert_eq!(sanitized.text.matches(reference).count(), 2);
    }

    #[test]
    fn leaves_safe_code_and_identifiers_unchanged() {
        let safe = [
            "commit 0123456789abcdef0123456789abcdef01234567",
            "id 550e8400-e29b-41d4-a716-446655440000",
            "visit https://example.com/a/long/path?query=value",
            "const token = getTokenFromStore();",
            "TOKEN=development",
            "let secret_name = config.secret_name;",
        ];
        for input in safe {
            assert_eq!(sanitize(input).text, input);
            assert!(sanitize(input).interceptions.is_empty());
        }
    }
}
