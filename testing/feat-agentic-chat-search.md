# feat/agentic-chat-search — Test Contract

Cross-chat "find that chat" search over the session forest (issue #762). A
funnel: T0 deterministic parse, T1 global FTS5 retrieval, T2 budgeted
tool-free model loop only when T1 is unsure and the caller asked for it.

## Decisions locked before code

- **Wire:** `sessions/search_chats` with `{ query, limit?, deep? }`. `deep`
  omitted or false runs T0+T1 only and never touches a model. `deep: true`
  runs T1, then T2 only when the confidence gate fails. The UI calls T1 on
  type (debounced) and the deep call on Enter, so T1 renders first and the
  deep result is the "second response".
- **Settings:** `config/get_chat_search_settings` and
  `config/save_chat_search_settings`, stored in `configuration_entries`
  (`kind = 'chat_search_settings'`). Fields: `deepSearch: bool` (default on)
  and `model: string?` (default the cheapest Claude model, `haiku`).
- **T2 harness is Claude only.** T2 runs as a hidden, tool-free
  briefing-policy session, and Claude is the only adapter that can enforce
  tool-free (`briefing_policy::adapter_may_brief`). The Settings row says so.
  The session kind is `chat_search`, added to `is_hidden_session_kind`.
- **T2 tools are Bridge-side, not provider tools.** The model asks for a tool
  with a fenced JSON reply, Rust runs it and sends the truncated result as the
  next turn. That keeps every budget in Rust and gives the provider no tools.
- **Wall clock:** 8 s is counted from the first model turn. Provider start is
  not in that budget (a Claude sidecar cold start alone is several seconds);
  it is bounded by the adapter's own start timeout.
- **Usage:** T2 writes one `usage_ledger` row per model turn with
  `source = 'provider.claude'` and `task_family = 'chat_search'`, on the hidden
  session. A literal `chat_search` source would hide the spend from every usage
  surface, since they all read `source LIKE 'provider.%'`.
- **Scope of search:** top-level chats only (`parent_session_id IS NULL`),
  hidden kinds excluded, ended and archived chats included and marked.
- **Digest key:** `chat_digests(id INTEGER PRIMARY KEY, session_id UNIQUE,
  body)` with `chat_digest_fts` as external content over it. Maintenance is an
  indexed upsert, not an UNINDEXED-column scan, and an INTEGER PRIMARY KEY is
  never renumbered by `VACUUM` (a bare `sessions.rowid` can be).
- **Model-bound snippets** are cut from the whole entry body after redaction,
  never from an FTS `snippet()` fragment, which can split a secret into a
  piece no detector recognises.
- **Coverage bonus:** after relaxing AND to OR, chats that matched every term
  get +2/61, since rank fusion alone cannot tell "all terms" from "one term".

## Functional Behavior

### T0 parse (`chat_search::parse`, injected `now`)
- `"yesterday"` → since = start of yesterday (UTC), until = start of today.
- `"today"` → since = start of today, until = now.
- `"3 days ago"` → [now − 4d, now − 2d] (one day of slack each side).
- `"a few days ago"` → [now − 7d, now − 1d].
- `"last week"` → [now − 14d, now]. `"this week"` → [now − 7d, now].
- `"2 weeks ago"` → [now − 21d, now − 7d].
- `"last month"` → previous calendar month. `"in march"` / `"march"` → the most
  recent March that has started (this year if now ≥ March, else last year).
- `"recently"` → [now − 14d, now].
- Harness words `codex`, `claude`, `opencode`, `cursor`, `grok` become a
  harness filter and leave the term list.
- `"quoted strings"` become phrase terms, kept verbatim (still sanitized).
- Stopwords and filler (`the`, `one`, `where`, `we`, `chat`, `about`,
  `something`, …) and consumed time words are removed.
- Remaining terms ranked by document frequency (fts5vocab over entries and
  digests, rarest first); terms absent from both vocabularies are dropped; the
  4 to 6 rarest are kept. The last typed token becomes a prefix term (`tok*`)
  when it has no exact vocabulary entry but has prefix matches.
- FTS syntax in user input (`OR`, `NEAR`, `*`, `"`, `:`, `(`) can never reach
  MATCH unescaped.

### T1 retrieve (`chat_search::retrieve`)
- One MATCH over `session_entry_fts` with no `session_id` filter, joined to
  `sessions` for the harness/time/visibility filter, `ORDER BY bm25 LIMIT 200`.
- One MATCH over `chat_digest_fts`.
- AND first; relax to OR when AND finds fewer than 4 chats.
- Fused score = Σ w/(60 + rank) (entries w = 1, digest w = 2) + multi-hit
  bonus 0.25 × Σ top-3 normalized entry strengths / 61 + recency
  0.1 × exp(−age_days/30) / 61.
- Top `limit` (default 4, max 8) `ChatSearchHit { sessionId, title, harness,
  workspaceId, workspaceTitle, lastActiveAt, matchCount, snippet ≤ 160 chars,
  score, why, archived, ended }`.
- Gate `confident = score[0]/score[1] ≥ 1.5 && matchCount[0] ≥ 2`, or exactly
  one hit.
- Zero terms after parse (e.g. only stopwords, or only a time phrase) with a
  time/harness filter → most recent chats in that window, not an error. Zero
  terms and no filter → empty result with a reason, not an error.

### T2 agent (`chat_search::agent` + `tools`)
- Only when `deep` and not `confident` and the deep-search setting is on.
- First turn shows up to 8 T1 cards (≈60 tokens each), then the query last.
- Tools: `find_chats{terms[], since?, until?, harness?, limit ≤ 8}`,
  `peek_chat{id, term, n ≤ 3}` (200-char windows),
  `chat_outline{id}` (title, first user message 300 chars, latest summary 300
  chars, entry count).
- Budget in Rust: ≤ 3 tool calls, ≤ 2,000 tool-output tokens
  (chars/4), 8 s model wall clock. Exhaustion or model error → T1 hits with
  `why = "budget"` / the error class, `stage = "t1_fallback"`.
- Answer `[{id, why}]` ≤ 4, why ≤ 15 words (truncated server-side). Ids are
  resolved only against ids the model was shown; unknown ids dropped.
- Every model-bound string passes `secret_interception::sanitize`.
- Never creates a visible chat, never appends to any forest, never switches a
  chat's harness.

### UI
- Sidebar search field: title filter as today, plus an "In conversations"
  section fed by T1 (debounced 180 ms, ≥ 2 chars). Cards show title, date,
  harness mark, snippet, and `why` when present; archived/ended marked.
- Enter: if T1 is confident, opens the top hit; otherwise runs the deep search
  and shows "Searching deeper…" until it returns. Click opens a hit.
- Deep search off or unavailable → Enter opens the top T1 hit; the hint row
  says why deeper search is not available.
- `/find <query>` in the composer opens the sidebar search with the query and
  runs the deep search; it never sends a turn.
- Settings → Composer → "Chat search" group: deep search switch + model select.

## Unit Tests

Rust (`bridge-core`):
- `chat_search::parse::tests::*` — every time phrase above with a fixed
  clock; harness words; quoted phrase; stopword strip; FTS operator soup is
  inert; IDF keeps the rarest terms and drops unknown ones; prefix last token.
- `chat_search::retrieve::tests::finds_chats_across_sessions_with_one_query`
- `…::digest_title_match_outranks_single_passing_mention`
- `…::relaxes_to_or_when_and_is_too_narrow`
- `…::time_and_harness_filters_apply_in_sql`
- `…::hidden_kinds_and_workers_are_never_hits`
- `…::archived_and_ended_chats_are_found_and_marked`
- `…::non_indexed_entry_kinds_never_match`
- `…::gate_is_confident_for_a_clear_winner_and_not_for_a_tie`
- `…::digest_tracks_title_first_message_and_latest_summary` (trigger upkeep,
  incl. title rename and session delete)
- `…::search_session_entries_is_unchanged` (per-session recall still scoped)
- `chat_search::agent::tests::t1_confident_never_calls_the_model`
- `…::shallow_request_never_calls_the_model`
- `…::stops_after_three_tool_calls_and_returns_t1_budget`
- `…::tool_output_never_exceeds_two_thousand_tokens`
- `…::wall_clock_exhaustion_returns_t1_budget`
- `…::model_error_returns_t1_fallback`
- `…::hallucinated_ids_are_dropped`
- `…::why_is_truncated_to_fifteen_words`
- `…::model_bound_text_is_redacted`
- `…::first_turn_puts_the_query_last`
- `chat_search::settings` load default / save round-trip / reject blank model.
- `work_briefing_config::tests` — `chat_search` is a hidden kind.
- `slash::tests` — `/find` dispatches to `SlashDispatch::Find` and is bridge-local.
- Protocol: params `deny_unknown_fields`, naming gate, mirror gate, artifacts
  regenerated (`tsgen::checked_in_artifacts_match_the_contract`).
- `bridge-deck`: registry ↔ `generate_handler!` 1:1 and command signatures.

Frontend (Vitest):
- `chatSearch.test.ts` — debounce/stale-response ordering helper, Enter
  decision (open top vs deep).
- `BridgeSidebar.test.tsx` — typing shows "In conversations" hits from the
  mock; Enter on a non-confident result shows "Searching deeper…" then the
  deep hits with `why`; clicking a hit calls `onOpenSession`.
- `ComposerPage` — Chat search group persists deep switch and model.
- `/find` parse helper.

## Integration / Functional Tests
- `api::search_chats` through `protocol_wire` into `wire::SearchChatsResult`
  (the typed round trip that direct-JSON tests skip).
- Daemon dispatch arms for the three methods (compile + registry gate).
- Eval (`chat_search::eval`, runs in `cargo test`): fixture corpus of 200
  chats / 20k entries from `testing/fixtures/chat_search/`, 40 labelled
  queries in three classes; asserts recall@4 ≥ 90% overall for T1 alone
  (T2 needs a live model, so CI cannot assert its contribution), and reports
  per-class recall and the fraction the gate resolves without T2.

## Smoke Tests
- `bun run build` and `bun run test` green.
- `cargo test --manifest-path src-tauri/Cargo.toml --workspace` green,
  including bridge-deck.

## E2E Tests
- Mock mode (`bun run dev`): open sidebar Search, type a word from a mock
  chat's messages, the "In conversations" card appears; Enter opens it.
- Live app (if a Claude login is available): deep search on a vague query
  returns cards with `why`, and one `usage_ledger` row per model turn with
  `task_family = 'chat_search'` appears; no new visible chat.

## Manual / cURL Tests
- Bench (`#[ignore]`, run manually):
  `cargo test -p bridge-core --lib chat_search::eval::bench -- --ignored --nocapture`
  reports T1 p50/p95 at 100k entries (target p95 < 50 ms) and the three
  baselines: (a) titles-only prompt tokens, (b) per-session sequential
  `search_session_entries` ms, (c) dump-all-summaries prompt tokens.
- Daemon probe: `sessions/search_chats {"query":"plugins stall"}` on the real
  socket returns hits without error.
