//! Task identity across runs.
//!
//! A briefing runs again and again against sources that move, so almost every hard
//! question in reconciliation reduces to one: is this the same task as last time?
//!
//! It is the same task when it points at the same resource in the same account — not
//! when the model phrased it the same way. Model text is an input to a fingerprint
//! nowhere, because a rewording would then create a duplicate and a plagiarism would
//! merge two different things.
//!
//! The encoding is where the care goes. `format!("{instance}:{resource}")` looks
//! obviously fine and is not: `("a", "b:c")` and `("a:b", "c")` produce the same string,
//! so two accounts could be arranged to share an identity. Every component is therefore
//! length-prefixed, which makes the boundary unambiguous no matter what the components
//! contain.

use sha2::{Digest, Sha256};

/// The fingerprint scheme in force. Part of the value, so a future encoding can coexist
/// with this one instead of silently matching rows it did not create.
pub const FINGERPRINT_VERSION: &str = "v1";

/// Length-prefix one component so no arrangement of separators can forge another
/// component boundary.
fn write_component(hasher: &mut Sha256, component: &str) {
    hasher.update((component.len() as u64).to_le_bytes());
    hasher.update(component.as_bytes());
}

/// The identity of a task that points at a known resource.
///
/// `connector_instance_id` carries the account: one instance is one signed-in account, so
/// two accounts holding issue `42` are two instances and therefore two fingerprints. The
/// canonical resource id from slice 4 is already namespaced by family and instance, and
/// including the instance again is deliberate belt — the fingerprint should not depend on
/// another module's namespacing choice to stay account-aware.
pub fn fingerprint(connector_instance_id: &str, canonical_resource_id: &str) -> String {
    let mut hasher = Sha256::new();
    write_component(&mut hasher, FINGERPRINT_VERSION);
    write_component(&mut hasher, connector_instance_id);
    write_component(&mut hasher, canonical_resource_id);
    format!("{FINGERPRINT_VERSION}:{:x}", hasher.finalize())
}

/// The identity of a task Bridge could not tie to a resource.
///
/// `None`, and that is the whole design: the schema's unique index treats NULLs as
/// distinct, so several unidentifiable tasks coexist while two identified tasks can never
/// share a fingerprint. Inventing an id here — from the title, say — would merge tasks
/// that happen to be worded alike and split ones that are not.
pub fn fingerprint_for(
    connector_instance_id: &str,
    canonical_resource_id: Option<&str>,
) -> Option<String> {
    canonical_resource_id
        .map(str::trim)
        .filter(|resource| !resource.is_empty())
        .map(|resource| fingerprint(connector_instance_id, resource))
}

/// Which scheme produced a stored fingerprint, for a future migration to read.
pub fn version_of(stored: &str) -> Option<&str> {
    stored.split_once(':').map(|(version, _)| version)
}

/// Was this fingerprint produced by the scheme this build writes?
pub fn is_current(stored: &str) -> bool {
    version_of(stored) == Some(FINGERPRINT_VERSION)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn the_same_inputs_always_produce_the_same_fingerprint() {
        let once = fingerprint("slack-1", "slack:slack-1:1723459200.123");
        let again = fingerprint("slack-1", "slack:slack-1:1723459200.123");
        assert_eq!(once, again);
        assert!(once.starts_with("v1:"));
    }

    #[test]
    fn the_length_prefix_is_fixed_width_across_architectures() {
        assert_eq!(
            fingerprint("a", "b"),
            "v1:62933c6256860d7ab643853ed8518752b7d17ef4817ae8fd10db9ca26233b0a7"
        );
    }

    #[test]
    fn two_accounts_with_the_same_native_id_do_not_collide() {
        // The case that matters most: two Gmail accounts, two GitHub orgs, two Slack
        // workspaces all number things independently.
        let work = fingerprint("gmail-work", "gmail:gmail-work:thread-42");
        let personal = fingerprint("gmail-personal", "gmail:gmail-personal:thread-42");
        assert_ne!(work, personal);

        // And the same instance with two resources.
        assert_ne!(
            fingerprint("github-1", "github:github-1:PR_42"),
            fingerprint("github-1", "github:github-1:PR_43"),
        );
    }

    #[test]
    fn separator_stuffing_cannot_forge_another_fingerprint() {
        // `format!("{a}:{b}")` would make these three pairs collide. Length-prefixing
        // each component is what makes the boundary unambiguous.
        let arranged = [
            ("a", "b:c"),
            ("a:b", "c"),
            ("a:b:c", ""),
            ("", "a:b:c"),
        ];
        let fingerprints: BTreeSet<String> = arranged
            .iter()
            .map(|(instance, resource)| fingerprint(instance, resource))
            .collect();
        assert_eq!(
            fingerprints.len(),
            arranged.len(),
            "every arrangement of the same characters must be its own identity"
        );
    }

    #[test]
    fn a_component_that_looks_like_a_length_prefix_still_cannot_forge_one() {
        // The prefix is raw bytes rather than digits, so a component made of digits has
        // no way to imitate it.
        assert_ne!(fingerprint("7", "slack-1"), fingerprint("7slack-1", ""));
        assert_ne!(fingerprint("1", "1"), fingerprint("11", ""));
    }

    #[test]
    fn the_version_is_part_of_the_value() {
        let stored = fingerprint("slack-1", "slack:slack-1:1.1");
        assert_eq!(version_of(&stored), Some("v1"));
        assert!(is_current(&stored));
        // A row written by another scheme is recognisable as such rather than being
        // compared as though it were current.
        assert_eq!(version_of("v2:abc"), Some("v2"));
        assert!(!is_current("v2:abc"));
        assert!(!is_current("no-version"));
        assert_eq!(version_of("no-version"), None);
    }

    #[test]
    fn the_version_is_inside_the_hash_as_well_as_in_front_of_it() {
        // So a future scheme cannot produce the same digest with a different label, which
        // would let one row read as either version depending on who asked.
        let stored = fingerprint("slack-1", "slack:slack-1:1.1");
        let digest = stored.split_once(':').unwrap().1;
        let mut hasher = Sha256::new();
        write_component(&mut hasher, "v2");
        write_component(&mut hasher, "slack-1");
        write_component(&mut hasher, "slack:slack-1:1.1");
        assert_ne!(digest, format!("{:x}", hasher.finalize()));
    }

    #[test]
    fn an_unidentifiable_task_has_no_fingerprint() {
        // Not a derived one. Inventing an id from the title would merge tasks that happen
        // to be worded alike and split ones that are not.
        assert!(fingerprint_for("slack-1", None).is_none());
        assert!(fingerprint_for("slack-1", Some("")).is_none());
        assert!(fingerprint_for("slack-1", Some("   ")).is_none());
    }

    #[test]
    fn an_identifiable_task_gets_the_same_value_as_the_direct_call() {
        assert_eq!(
            fingerprint_for("slack-1", Some("slack:slack-1:1.1")),
            Some(fingerprint("slack-1", "slack:slack-1:1.1"))
        );
        // Surrounding whitespace in a stored id must not create a second identity.
        assert_eq!(
            fingerprint_for("slack-1", Some("  slack:slack-1:1.1  ")),
            Some(fingerprint("slack-1", "slack:slack-1:1.1"))
        );
    }

    #[test]
    fn no_model_authored_text_reaches_a_fingerprint() {
        // Structural: the function takes two Bridge-derived arguments and nothing else,
        // so a reworded title cannot create a duplicate task and two differently-sourced
        // tasks with identical wording cannot merge.
        let one = fingerprint_for("slack-1", Some("slack:slack-1:1.1")).unwrap();
        let other = fingerprint_for("slack-1", Some("slack:slack-1:1.1")).unwrap();
        assert_eq!(one, other, "identity does not depend on how a task was phrased");
    }
}
