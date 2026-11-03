# feat/memory-hardening

Slices 0 to 5 of the memory audit (issue #885). The ledger's guarantees stay:
provenance, validity intervals, conflict groups, refuse-don't-evict, and a
packet frozen at session start in the conversation tail. What changes is what
gets written, how long a proposal waits, what a full scope costs, what the
packet carries, and the fact that an agent can now search and propose memory
instead of writing rules into Notes.

Slice 6 (every-harness extraction on a cheap helper, debounce, daily cap) and
slice 7 (ownership doc, workspace scope, harness-native import as proposals)
are follow-ups. They change model cost and routing and need their own evals.

## Slice 0: evals

- `testing/evals/memory-packet/ledgers.json` holds labeled ledgers. One of
  them reproduces the live failure from #885: long assistant replies saved as
  `user_explicit` preferences (some saved more than once) beside short
  extracted constraints. Each ledger labels the records that must reach the
  packet and the groups of records that say the same thing.
- `memory_evals::packet_eval` runs in CI with no model. It scores every
  ledger for: gold coverage (must-include records selected), constraint
  coverage, duplicate groups with more than one member selected, the longest
  packet line, and total packet size. Gates: gold coverage 100%, constraint
  coverage 100%, exact-duplicate lines 0, every line within the line cap,
  total within `MAX_PACKET_CHARS`. Paraphrase duplicate groups are reported,
  not gated: lexical dedupe cannot see them, and the number says so.
- `memory_evals::live_extraction` (ignored, `BRIDGE_MEMORY_EVAL=1`) replays
  labeled conversations from `testing/evals/memory-extraction/cases.json`
  through the real extraction pipeline on a hidden turn and scores
  precision and recall of proposals against gold, plus forbidden proposals
  (status updates, one-off task details, assistant claims). Prompt changes
  report these numbers.
- `memory_evals::live_tools` (ignored, `BRIDGE_MEMORY_TOOLS_EVAL=1`) starts
  real chats with the Bridge tools attached and scores what the agent did to
  the ledger and to Notes: a durable rule the person states becomes one
  `agent_proposal`; an ordinary task proposes nothing; "write this up as a
  note" creates a note and no memory; "what do you remember about X" calls
  `memory_search` before answering.

## Slice 1: stop the silent spend

- `memory_extraction::enqueue_after_turn` checks the scope budget. A full
  scope enqueues nothing, so no model call happens.
- `claim_due` checks again. A run queued before the scope filled settles
  `cancelled` with detail `scope_full` and never reaches an adapter.
- Extraction telemetry counts `cache_creation_input_tokens` and
  `cache_read_input_tokens` beside `input_tokens` and `output_tokens`.
- The Memory dialog shows when the scope is full, with the held and budget
  numbers and what to do (review proposals, forget pins).

## Slice 2: clean writes

- "Remember this" on an assistant reply never saves the reply. It opens the
  Memory dialog with the text in the draft for the person to cut down and
  pick a kind. A one-click verbatim save is the path that put status
  updates in every packet.
- `memory_ledger::save` is idempotent: when an active record in scope has the
  same normalized body (case, whitespace, and edge punctuation folded), it
  returns that record and writes nothing, so a double click is one pin and a
  full scope does not refuse a pin that already exists.
- `memory_similarity` is the one definition of "same claim" for writes and
  for the packet: `normalize`, `terms`, `jaccard_bps`, `near_duplicate`
  (Jaccard of content terms at or above 80%, with at least three terms each
  side). It is deterministic, symmetric, and reflexive on non-empty text.
- Extraction and agent proposals refuse a near duplicate of an active,
  proposed, or rejected record in scope, counted as `duplicates`. A
  rejected claim does not come back in different words.

## Slice 3: proposal lifecycle

- A proposal older than `PROPOSAL_TTL_DAYS` (30) is swept to `expired`. Its
  interval stays empty (`valid_to = valid_from`), so no as-of read ever
  returns it. It leaves the budget, so a backlog drains instead of blocking
  the scope forever. Active records keep their own `expires_at` rule.
- The sidebar Memory row shows the number of proposals waiting for review,
  refreshed on `memory-changed`. No badge when nothing waits.

## Slice 4: packet quality

- Rank: trust (typed pins first), then kind (constraint, preference,
  decision, fact), then confidence, then recency. Total and reproducible.
- No line exceeds `MAX_PACKET_LINE_CHARS` (320). A longer body is clipped on
  a character boundary with a marker that names `memory_search`, and the
  audit records the clipped text, because that is what was sent.
- A record that is a near duplicate of one already selected is excluded with
  code `near_duplicate`, counted in the Activity tab like the other gate
  codes.
- The packet still drops whole lines against `MAX_PACKET_CHARS`, still
  carries at most one member per conflict group, and is still built once at
  session start (the #528 cache guarantee).

## Slice 5: agent memory tools

- The chat-tools listener (today the notes route) also serves
  `memory_search` to every session and `memory_propose` to top-level chats,
  with the same per-chat HMAC token. The `bridge` MCP server lists and
  forwards them.
- `memory_search {query, limit?}`: FTS over active account records, at most
  20, returned as data with ids, never instructions.
- `memory_propose {body, kind, rationale}`: runs the same gate as extraction
  (body cap, secret interception, kind vocabulary, rationale required and
  bounded, exact and near-duplicate refusal, scope budget) and writes
  `proposed` / `agent_proposal` with the session id. The agent never sets a
  status, scope, provenance, interval, or expiry, and an agent proposal is
  never activated automatically. Manual-only mode refuses it. A session may
  write at most `MAX_AGENT_PROPOSALS_PER_SESSION` (5). A duplicate answers
  with the existing id instead of a new row.
- Tool descriptions carry the when and when-not rubric. `notes_create` says
  it is not for the agent's own memory or rules for future sessions.
- An approved agent proposal reads as "agent memory" in the packet and as an
  accepted suggestion, not a pin, in the Memory dialog.

## Fuzz and mutation

- Property tests (proptest) cover: similarity laws; save idempotency under
  case and whitespace variants; the budget never exceeded by any sequence of
  saves, proposals, approvals, rejections, forgets, and sweeps, with a
  refused write leaving the scope unchanged; swept proposals keep an empty
  interval; packet invariants over random ledgers (line cap, total cap, one
  per conflict group, no exact duplicate lines, deterministic); and
  `gate_and_insert` on arbitrary model text never panics and never writes
  anything but `proposed` rows outside auto-apply.
- A cargo-fuzz target (`src-tauri/bridge-core/fuzz`, nightly) drives
  `gate_and_insert` and the memory tool argument parser with raw bytes.
- `scripts/mutants-memory.sh` runs cargo-mutants over the changed memory
  functions. Surviving mutants are killed with tests or listed in the PR
  with the reason they are equivalent.

## Not in this PR

Relevance ranking against the objective, a model-backed curator for
paraphrase duplicates, a cheap default helper, extraction for Codex,
OpenCode and Cursor chats, workspace scope, a bulk "these look like replies"
cleanup for existing pins, and continuous import of harness-native memory.
