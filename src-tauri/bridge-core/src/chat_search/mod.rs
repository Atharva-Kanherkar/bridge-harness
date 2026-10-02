//! Find a chat from a vague memory, across every chat.
//!
//! A funnel, cheapest stage first:
//!
//! - **T0** [`parse`]: time phrases, harness words, phrases and terms, ranked
//!   by rarity from the FTS vocabulary. No model.
//! - **T1** [`retrieve`]: one global MATCH over entries and one over the
//!   per-chat [`index`] digests, fused per chat. Tens of milliseconds, no
//!   model, and most queries stop here.
//! - **T2** [`agent`]: only when the index is unsure and the caller asked. A
//!   hidden, tool-free Claude turn sees small cards, may ask Bridge for three
//!   bounded lookups ([`tools`]), and names chats it was shown. Every budget is
//!   enforced here, not by the prompt.
//!
//! Search is read-only end to end. It asks the policy engine for nothing, and
//! a result widens nothing: opening a hit is an ordinary navigation.

pub mod agent;
pub mod index;
pub mod live;
pub mod parse;
pub mod retrieve;
pub mod settings;
pub mod tools;
pub mod warm;

#[cfg(test)]
mod eval;
#[cfg(test)]
mod eval_codex;

/// The hidden session a deep search runs its model turns in.
pub const CHAT_SEARCH_SESSION_KIND: &str = "chat_search";

use std::sync::{Arc, Mutex};
use std::time::Instant;

use bridge_protocol::messages::{
    self as wire, ChatSearchHit, ChatSearchStage, SearchChatsParams, SearchChatsResult,
    DEFAULT_CHAT_SEARCH_LIMIT, MAX_CHAT_SEARCH_LIMIT,
};
use chrono::{DateTime, Utc};
use rusqlite::Connection;

use crate::{BridgeCore, BridgeError};
use agent::{Budget, Outcome, SearchInput, SearchModel, SEED_CARDS};
use retrieve::Candidate;

/// Whether the model stage may run, decided before any model is started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeepGate {
    Allowed,
    /// Why not, in words a user can act on.
    Unavailable(String),
}

fn hit(candidate: &Candidate, why: Option<String>) -> ChatSearchHit {
    ChatSearchHit {
        session_id: candidate.session_id.clone(),
        title: candidate.title.clone(),
        harness: candidate.harness.clone(),
        workspace_id: candidate.workspace_id.clone(),
        workspace_title: candidate.workspace_title.clone(),
        last_active_at: candidate.last_active_at.clone(),
        match_count: candidate.match_count(),
        snippet: candidate.snippet.clone(),
        score: candidate.score,
        why: why.unwrap_or_else(|| candidate.why()),
        archived: candidate.archived,
        ended: candidate.ended,
    }
}

fn resolve_limit(limit: Option<u32>) -> Result<usize, BridgeError> {
    match limit {
        None => Ok(DEFAULT_CHAT_SEARCH_LIMIT as usize),
        Some(value) if (1..=MAX_CHAT_SEARCH_LIMIT).contains(&value) => Ok(value as usize),
        Some(_) => Err(BridgeError::Invalid(format!(
            "Chat search limit must be between 1 and {MAX_CHAT_SEARCH_LIMIT}"
        ))),
    }
}

/// The whole funnel against one database. `start_model` is only called when
/// the model stage will actually run, so a shallow or confident search never
/// starts a provider.
pub fn search_with(
    db: &Mutex<Connection>,
    params: &SearchChatsParams,
    now: DateTime<Utc>,
    gate: DeepGate,
    start_model: impl FnOnce() -> Result<Box<dyn SearchModel>, String>,
) -> Result<SearchChatsResult, BridgeError> {
    search_with_budget(db, params, now, gate, &Budget::default(), start_model)
}

fn search_with_budget(
    db: &Mutex<Connection>,
    params: &SearchChatsParams,
    now: DateTime<Utc>,
    gate: DeepGate,
    budget: &Budget,
    start_model: impl FnOnce() -> Result<Box<dyn SearchModel>, String>,
) -> Result<SearchChatsResult, BridgeError> {
    let started = Instant::now();
    let limit = resolve_limit(params.limit)?;
    let query = params.query.trim().to_owned();
    let parsed = parse::parse(&query, now);
    let (terms, retrieval) = {
        let db = db.lock().unwrap();
        let (terms, unknown) = parse::rank_terms_counted(&db, &parsed)?;
        // The model sees more candidates than the user does.
        let mut retrieval = retrieve::retrieve(&db, &parsed, &terms, limit.max(SEED_CARDS), now)?;
        retrieval.unknown_terms = unknown;
        (terms, retrieval)
    };
    let words: Vec<String> = terms.iter().map(|term| term.text.clone()).collect();
    let confident = retrieval.confident();
    let has_words = !parsed.terms.is_empty();
    let (deep_available, mut detail) = match (&gate, confident, has_words) {
        (_, true, _) => (false, None),
        (_, false, false) if query.is_empty() => (false, Some("Type what you remember about the chat.".to_owned())),
        (_, false, false) => (false, None),
        (DeepGate::Allowed, false, true) => (true, None),
        (DeepGate::Unavailable(reason), false, true) => (false, Some(reason.clone())),
    };
    let mut result = SearchChatsResult {
        query: query.clone(),
        hits: retrieval.candidates.iter().take(limit).map(|candidate| hit(candidate, None)).collect(),
        stage: ChatSearchStage::Index,
        confident,
        deep_available,
        detail: None,
        terms: words.clone(),
        elapsed_ms: 0,
        model_tokens: 0,
        tool_calls: 0,
    };
    if params.deep && deep_available {
        match start_model() {
            Ok(mut model) => {
                let input = SearchInput {
                    query: &query,
                    parsed: &parsed,
                    terms: &words,
                    seed: &retrieval.candidates,
                    partial: retrieval.unknown_terms > 0
                        || retrieval.candidates.first().is_some_and(|first| !first.covered_all),
                };
                let run = agent::run(db, model.as_mut(), &input, budget, now);
                // Stop the provider before answering, not after.
                drop(model);
                result.tool_calls = run.tool_calls;
                result.model_tokens = u32::try_from(run.model_tokens).unwrap_or(u32::MAX);
                match run.outcome {
                    Outcome::Answered(answered) => {
                        result.stage = ChatSearchStage::Model;
                        result.hits = answered
                            .iter()
                            .take(limit)
                            .map(|(candidate, why)| {
                                let why = (!why.trim().is_empty()).then(|| why.clone());
                                hit(candidate, why)
                            })
                            .collect();
                    }
                    Outcome::Fallback { reason } => {
                        result.stage = ChatSearchStage::IndexFallback;
                        detail = Some(if reason == "budget" {
                            "Deeper search ran out of budget; these are the index's matches.".to_owned()
                        } else {
                            format!("Deeper search stopped: {reason}. These are the index's matches.")
                        });
                        for hit in &mut result.hits {
                            if reason == "budget" {
                                hit.why = "budget".into();
                            }
                        }
                    }
                }
            }
            Err(error) => {
                result.stage = ChatSearchStage::IndexFallback;
                detail = Some(format!("Deeper search could not start: {error}"));
            }
        }
    }
    result.detail = detail;
    result.elapsed_ms = u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX);
    Ok(result)
}

/// Whether this host can run the model stage right now.
pub fn deep_gate(core: &Arc<BridgeCore>, settings: &wire::ChatSearchSettings) -> DeepGate {
    if !settings.deep_search {
        return DeepGate::Unavailable("Deeper search is off in Settings → Composer.".into());
    }
    claude_gate(core)
}

/// How long a Claude availability answer is reused. Search runs on every
/// debounced keystroke and a descriptor spawns child processes (a version
/// probe, a Keychain lookup); asking each time stalled typing and every other
/// call queued behind it.
const GATE_TTL: std::time::Duration = std::time::Duration::from_secs(30);

fn claude_gate(core: &Arc<BridgeCore>) -> DeepGate {
    static CACHE: Mutex<Option<(usize, Instant, DeepGate)>> = Mutex::new(None);
    let owner = Arc::as_ptr(core) as usize;
    if let Some((cached_owner, at, gate)) = CACHE.lock().unwrap().as_ref() {
        if *cached_owner == owner && at.elapsed() < GATE_TTL {
            return gate.clone();
        }
    }
    let gate = match core.adapter_registry.descriptor(live::HARNESS) {
        Some(descriptor) if descriptor.available => DeepGate::Allowed,
        Some(descriptor) => DeepGate::Unavailable(format!(
            "Deeper search runs on Claude Code, which is unavailable: {}",
            descriptor.unavailable_reason.unwrap_or_else(|| "not signed in".into())
        )),
        None => DeepGate::Unavailable(
            "Deeper search runs on Claude Code, the one harness that can answer without tools. Install it to search deeper.".into(),
        ),
    };
    *CACHE.lock().unwrap() = Some((owner, Instant::now(), gate.clone()));
    gate
}

pub fn search(core: &Arc<BridgeCore>, params: &SearchChatsParams) -> Result<SearchChatsResult, BridgeError> {
    let settings = settings::load(&core.db.lock().unwrap())?;
    let gate = deep_gate(core, &settings);
    let model = settings::model(&settings);
    let result = search_with(&core.db, params, Utc::now(), gate, || {
        live::take_or_start(core, &model).map(|model| Box::new(model) as Box<dyn SearchModel>)
    })?;
    // The index is unsure and the user may press Enter next: start the model
    // now, off the request, so Enter pays for turns rather than a cold start.
    if !params.deep && result.deep_available {
        live::prewarm(core, &model);
    }
    Ok(result)
}

/// A plain-text reply for `/find`, index only. The composer opens the
/// sidebar search instead; this answers clients that submit it directly.
pub fn format_reply(result: &SearchChatsResult) -> String {
    if result.hits.is_empty() {
        return format!("No chats match \"{}\".", result.query);
    }
    let mut out = format!("Chats matching \"{}\":\n", result.query);
    for (index, hit) in result.hits.iter().enumerate() {
        out.push_str(&format!(
            "\n{}. {} · {} · {}\n   {}\n",
            index + 1,
            hit.title,
            hit.harness,
            hit.last_active_at.get(..10).unwrap_or(&hit.last_active_at),
            hit.why
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::agent::ModelTurn;
    use super::*;
    use crate::store::{self, append_session_entry};
    use rusqlite::params;
    use serde_json::json;
    use std::cell::Cell;

    struct Fixed(&'static str);

    impl SearchModel for Fixed {
        fn turn(&mut self, _text: &str, _deadline: Instant) -> Result<ModelTurn, String> {
            Ok(ModelTurn { text: self.0.into(), tokens: 42 })
        }
    }

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-09-30T15:00:00Z").unwrap().with_timezone(&Utc)
    }

    fn corpus() -> (tempfile::TempDir, Mutex<Connection>) {
        let dir = tempfile::tempdir().unwrap();
        let db = store::open(&dir.path().join("bridge.db")).unwrap();
        for (id, title, texts) in [
            ("clear-1", "Pelican migration", vec!["pelican migration plan", "pelican migration rollout"]),
            ("tie-a", "Heron sync", vec!["heron sync"]),
            ("tie-b", "Heron sync", vec!["heron sync"]),
        ] {
            db.execute(
                "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,kind,title)
                 VALUES(?1,NULL,'codex','Chat','idle','estimated','direct',?2)",
                params![id, title],
            )
            .unwrap();
            for text in texts {
                append_session_entry(&db, id, None, "user.message", &json!({"text": text}), None, "eligible", None).unwrap();
            }
        }
        (dir, Mutex::new(db))
    }

    fn params(query: &str, deep: bool) -> SearchChatsParams {
        SearchChatsParams { query: query.into(), limit: None, deep }
    }

    #[test]
    fn t1_confident_never_calls_the_model() {
        let (_dir, db) = corpus();
        let calls = Cell::new(0);
        let result = search_with(&db, &params("pelican migration", true), now(), DeepGate::Allowed, || {
            calls.set(calls.get() + 1);
            Ok(Box::new(Fixed("{\"answer\":[]}")))
        })
        .unwrap();
        assert!(result.confident);
        assert_eq!(result.stage, ChatSearchStage::Index);
        assert_eq!(calls.get(), 0, "a confident index must not start a model");
        assert_eq!(result.model_tokens, 0);
        assert_eq!(result.hits[0].session_id, "clear-1");
    }

    #[test]
    fn shallow_request_never_calls_the_model() {
        let (_dir, db) = corpus();
        let calls = Cell::new(0);
        let result = search_with(&db, &params("heron sync", false), now(), DeepGate::Allowed, || {
            calls.set(calls.get() + 1);
            Ok(Box::new(Fixed("{\"answer\":[]}")))
        })
        .unwrap();
        assert!(!result.confident);
        assert!(result.deep_available, "an unsure index offers the deep stage");
        assert_eq!(calls.get(), 0);
        assert_eq!(result.hits.len(), 2);
    }

    #[test]
    fn deep_request_on_an_unsure_index_uses_the_model_answer() {
        let (_dir, db) = corpus();
        let result = search_with(&db, &params("heron sync", true), now(), DeepGate::Allowed, || {
            Ok(Box::new(Fixed("{\"answer\":[{\"id\":\"tie-b\",\"why\":\"the second heron chat\"}]}")))
        })
        .unwrap();
        assert_eq!(result.stage, ChatSearchStage::Model);
        assert_eq!(result.hits.len(), 1);
        assert_eq!(result.hits[0].session_id, "tie-b");
        assert_eq!(result.hits[0].why, "the second heron chat");
        assert_eq!(result.model_tokens, 42);
    }

    #[test]
    fn a_model_that_cannot_start_leaves_the_index_answer() {
        let (_dir, db) = corpus();
        let result = search_with(&db, &params("heron sync", true), now(), DeepGate::Allowed, || {
            Err("Claude is not signed in".into())
        })
        .unwrap();
        assert_eq!(result.stage, ChatSearchStage::IndexFallback);
        assert_eq!(result.hits.len(), 2);
        assert!(result.detail.unwrap().contains("not signed in"));
    }

    #[test]
    fn an_unavailable_gate_is_explained_and_never_starts_a_model() {
        let (_dir, db) = corpus();
        let calls = Cell::new(0);
        let result = search_with(
            &db,
            &params("heron sync", true),
            now(),
            DeepGate::Unavailable("Deeper search is off".into()),
            || {
                calls.set(calls.get() + 1);
                Ok(Box::new(Fixed("{\"answer\":[]}")))
            },
        )
        .unwrap();
        assert_eq!(calls.get(), 0);
        assert!(!result.deep_available);
        assert_eq!(result.detail.as_deref(), Some("Deeper search is off"));
    }

    #[test]
    fn the_result_crosses_the_wire_in_its_contracted_shape() {
        let (_dir, db) = corpus();
        let result = search_with(&db, &params("heron sync", true), now(), DeepGate::Allowed, || Err("no".into())).unwrap();
        let json = serde_json::to_value(&result).unwrap();
        assert_eq!(json["stage"], "index_fallback");
        assert!(json["hits"][0]["sessionId"].is_string());
        assert!(json["hits"][0]["lastActiveAt"].is_string());
        assert!(json["deepAvailable"].is_boolean());
        let back: SearchChatsResult = serde_json::from_value(json).unwrap();
        assert_eq!(back, result);
    }

    #[test]
    fn limit_is_bounded() {
        let (_dir, db) = corpus();
        let bad = SearchChatsParams { query: "heron".into(), limit: Some(9), deep: false };
        assert!(search_with(&db, &bad, now(), DeepGate::Allowed, || Err(String::new())).is_err());
        let empty = search_with(&db, &params("  ", false), now(), DeepGate::Allowed, || Err(String::new())).unwrap();
        assert!(empty.hits.is_empty());
        assert!(empty.detail.is_some());
    }
}
