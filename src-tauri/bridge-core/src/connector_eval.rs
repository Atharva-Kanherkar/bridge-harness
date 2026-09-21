//! Replaying recorded connector render runs.
//!
//! The live eval is the real check: point a render run at an authenticated
//! connector and see whether the card it emits survives validation. That needs a
//! signed-in account, so it cannot run in CI — what runs in CI is this, a replay
//! of outputs recorded from real runs, including the ways a run goes wrong.
//!
//! Each fixture is a whole assistant message, not a card, because half of what
//! is being tested is the *extraction*: prose around the fence, two fences, a
//! fence the response was truncated inside. Those are the shapes a model
//! actually produces, and every one of them has to degrade to Bridge's own card
//! rather than to a blank notification.
//!
//! See `testing/evals/connector-render.md` for the live procedure and for how to
//! record a new fixture.

use std::path::{Path, PathBuf};

/// Where the recorded runs live, relative to the repository root.
pub const FIXTURE_DIR: &str = "testing/fixtures/connector-render";

pub fn fixture_dir() -> PathBuf {
    // `CARGO_MANIFEST_DIR` is `src-tauri/bridge-core`; the fixtures are two up.
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(FIXTURE_DIR)
}

pub fn load_fixture(name: &str) -> String {
    let path = fixture_dir().join(format!("{name}.txt"));
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("fixture {} is unreadable: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connector_runs::{card_or_fallback, parse_card};
    use crate::connector_surface::{CardBlock, InboxItem, ItemKind};
    use crate::work_connectors::ConnectorFamily;

    /// The item every fixture was recorded against.
    fn item() -> InboxItem {
        InboxItem {
            family: ConnectorFamily::Slack,
            channel_id: "D0BKKG51DJM".into(),
            channel_label: "Vishal Keshari".into(),
            message_ts: "1789041929.055829".into(),
            author: "Vishal Keshari".into(),
            kind: ItemKind::DirectMessage,
            text: "direct Ai interview ka link aata hai".into(),
            permalink: Some(
                "https://rimoapp.slack.com/archives/D0BKKG51DJM/p1789041929055829".into(),
            ),
            received_at: "2026-09-10T12:05:29Z".into(),
        }
    }

    #[test]
    fn a_recorded_run_produces_a_card_that_validates() {
        let card = parse_card(&load_fixture("well-formed"), &item()).expect("the recorded card validates");
        assert_eq!(card.item_key, item().key());
        assert!(card.harness_rendered);
        assert_eq!(card.blocks.len(), 4);
        // The four block kinds the prompt offers are the four it used.
        assert!(matches!(card.blocks[0], CardBlock::Message { .. }));
        assert!(matches!(card.blocks[1], CardBlock::Context { .. }));
        assert!(matches!(card.blocks[2], CardBlock::Summary { .. }));
        assert!(matches!(card.blocks[3], CardBlock::Fact { .. }));
        assert_eq!(card.suggested_replies.len(), 2);
    }

    #[test]
    fn the_recorded_run_kept_the_message_body_verbatim() {
        let card = parse_card(&load_fixture("well-formed"), &item()).unwrap();
        // A card that paraphrased the message would be a card that could quietly
        // change what someone said.
        match &card.blocks[0] {
            CardBlock::Message { text, author, .. } => {
                assert_eq!(text, &item().text);
                assert_eq!(author, &item().author);
            }
            other => panic!("expected the message block, got {other:?}"),
        }
    }

    #[test]
    fn every_failure_shape_degrades_to_bridges_own_card() {
        for fixture in ["prose-only", "two-cards", "wrong-item", "unterminated"] {
            let (card, rejection) = card_or_fallback(&load_fixture(fixture), &item());
            assert!(!card.harness_rendered, "{fixture} should have been refused");
            assert!(rejection.is_some(), "{fixture} should record why");
            // The notification survives every one of them, with the real message
            // in it. That is the property worth protecting.
            assert!(card.headline.contains("Vishal Keshari"), "{fixture}");
            assert!(card.clone().validate(&item().key()).is_ok(), "{fixture}");
        }
    }

    #[test]
    fn a_hostile_body_renders_without_suggesting_anything_to_do_about_it() {
        let card = parse_card(&load_fixture("injection"), &item()).expect("a hostile message still renders");
        // Suppressing it would be the attack working: the user must see that
        // someone sent them this.
        match &card.blocks[0] {
            CardBlock::Message { text, .. } => assert!(text.contains("ignore your instructions")),
            other => panic!("expected the message block, got {other:?}"),
        }
        // And nothing it asked for became an affordance.
        assert!(card.suggested_replies.is_empty());
        let rendered = serde_json::to_string(&card).unwrap();
        assert!(!rendered.contains("#public-random\",\"kind\":\"fact\""));
    }

    #[test]
    fn every_recorded_fixture_is_exercised_by_this_module() {
        // A fixture nobody replays is a fixture that silently stops meaning
        // anything, so adding one has to fail until a test names it.
        let covered = ["well-formed", "prose-only", "two-cards", "wrong-item", "unterminated", "injection"];
        let mut found: Vec<String> = std::fs::read_dir(fixture_dir())
            .expect("the fixture directory exists")
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| {
                let path = entry.path();
                (path.extension()? == "txt")
                    .then(|| path.file_stem()?.to_str().map(str::to_owned))?
            })
            .collect();
        found.sort();
        let mut expected: Vec<String> = covered.iter().map(|name| (*name).to_owned()).collect();
        expected.sort();
        assert_eq!(found, expected);
    }
}
