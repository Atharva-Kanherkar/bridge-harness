//! The one definition of "the same claim" for memory writes and the packet.
//!
//! Lexical and deterministic on purpose: an explicit save, a model proposal,
//! an agent proposal and the packet must agree on what a duplicate is, and a
//! test must be able to pin it. It catches repeats and light edits, the shape
//! of a double click or a re-proposal. Paraphrases ("no em dashes" against
//! "avoid em dashes in prose") need a model and are not claimed here.

use std::collections::BTreeSet;

/// Jaccard similarity, in basis points, at which two claims count as one.
pub const NEAR_DUPLICATE_BPS: u32 = 8_000;
/// Below this many content terms a claim is too short to compare by overlap:
/// "uses zed" and "uses vim" share half their terms and say opposite things.
pub const MIN_TERMS: usize = 3;

/// Words that carry no claim. Kept short: every entry is a word that two
/// different claims could share without meaning the same thing.
const STOP_WORDS: &[&str] = &[
    "a", "an", "and", "are", "as", "at", "be", "by", "for", "from", "in", "is", "it", "of", "on",
    "or", "that", "the", "their", "them", "they", "this", "to", "was", "were", "with",
];

/// Case, whitespace and edge punctuation folded. Two bodies with the same
/// normal form are the same text as far as any reader is concerned.
pub fn normalize(body: &str) -> String {
    body.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
        .trim_matches(|c: char| !c.is_alphanumeric())
        .to_string()
}

/// The content terms of a claim: lowercase alphanumeric runs, stop words out.
pub fn terms(body: &str) -> BTreeSet<String> {
    body.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty() && !STOP_WORDS.contains(word))
        .map(str::to_owned)
        .collect()
}

/// Jaccard similarity of two term sets in basis points, `0..=10_000`. Two
/// empty sets share nothing to compare, so they score 0 rather than 10_000.
pub fn jaccard_bps(left: &BTreeSet<String>, right: &BTreeSet<String>) -> u32 {
    let union = left.union(right).count();
    if union == 0 {
        return 0;
    }
    let shared = left.intersection(right).count();
    ((shared * 10_000) / union) as u32
}

/// Whether two bodies make the same claim: the same normal form, or, when
/// both carry enough terms to compare, an overlap at or above
/// [`NEAR_DUPLICATE_BPS`].
pub fn near_duplicate(left: &str, right: &str) -> bool {
    let (left_normal, right_normal) = (normalize(left), normalize(right));
    if !left_normal.is_empty() && left_normal == right_normal {
        return true;
    }
    let (left_terms, right_terms) = (terms(left), terms(right));
    left_terms.len() >= MIN_TERMS
        && right_terms.len() >= MIN_TERMS
        && jaccard_bps(&left_terms, &right_terms) >= NEAR_DUPLICATE_BPS
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn case_whitespace_and_edge_punctuation_fold() {
        assert_eq!(normalize("  Prefer  vitest -t filters!! "), "prefer vitest -t filters");
        assert!(near_duplicate(
            "Prefer vitest -t filters over whole-file runs while triaging.",
            "prefer  vitest -t filters over whole-file runs while triaging"
        ));
    }

    #[test]
    fn a_light_edit_is_the_same_claim() {
        assert!(near_duplicate(
            "CI is rerunning on 6a77f0c1 after the daemon test fix landed",
            "CI is rerunning on 6a77f0c1 after the daemon test fix has landed"
        ));
    }

    #[test]
    fn short_claims_that_differ_are_not_duplicates() {
        assert!(!near_duplicate("Uses Zed as the editor", "Uses Vim as the editor"));
        assert!(!near_duplicate("Uses zed", "Uses vim"));
    }

    #[test]
    fn paraphrases_are_out_of_reach_and_say_so() {
        // Pinned so nobody mistakes this module for a curator. A model step
        // owns paraphrases; see the issue's follow-up slice.
        assert!(!near_duplicate(
            "Do not use em dashes in written output.",
            "The user does not want em dashes used in written text or prose produced for them."
        ));
    }

    #[test]
    fn empty_and_punctuation_only_bodies_are_never_duplicates() {
        assert!(!near_duplicate("", ""));
        assert!(!near_duplicate("!!!", "..."));
        assert_eq!(jaccard_bps(&BTreeSet::new(), &BTreeSet::new()), 0);
    }

    fn body() -> impl Strategy<Value = String> {
        prop::collection::vec(prop_oneof!["[a-zA-Z]{1,8}", "[ \t\n]{1,3}", "[.,!?-]"], 0..24)
            .prop_map(|parts| parts.concat())
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(512))]

        #[test]
        fn normalize_is_idempotent(text in any::<String>()) {
            let once = normalize(&text);
            prop_assert_eq!(normalize(&once), once);
        }

        #[test]
        fn jaccard_is_bounded_and_symmetric(left in body(), right in body()) {
            let (left, right) = (terms(&left), terms(&right));
            let score = jaccard_bps(&left, &right);
            prop_assert!(score <= 10_000);
            prop_assert_eq!(score, jaccard_bps(&right, &left));
        }

        #[test]
        fn near_duplicate_is_symmetric(left in body(), right in body()) {
            prop_assert_eq!(near_duplicate(&left, &right), near_duplicate(&right, &left));
        }

        #[test]
        fn a_body_with_content_duplicates_itself(text in "[a-z]{1,10}( [a-z]{1,10}){0,12}") {
            prop_assert!(near_duplicate(&text, &text));
        }

        #[test]
        fn case_and_spacing_variants_are_duplicates(words in prop::collection::vec("[a-z]{2,9}", 1..12), upper in any::<bool>(), pad in "[ \t]{0,3}") {
            let plain = words.join(" ");
            let variant = format!("{pad}{}{pad}.", if upper { plain.to_uppercase() } else { words.join("  ") });
            prop_assert!(near_duplicate(&plain, &variant));
        }
    }
}
