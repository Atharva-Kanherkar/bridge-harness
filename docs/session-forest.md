# Session forest

Bridge stores conversation history as immutable entries in `session_entries`. Each entry belongs to one session, has an insertion-order `sequence`, and optionally points to a parent entry. `session_heads.active_entry_id` selects the active leaf.

This is [durable local history](local-history.md), not a tamper-proof or replicated evidence ledger.

```text
root → user → assistant → checkpoint
                └──────→ alternate user → assistant  (active)
```

Moving the head changes only the active conversation branch. It never rewinds a worktree, commit, or filesystem change. Appending after moving the head creates a new branch and leaves the previous entries intact.

Every controller append stamps the entry with the repository `HEAD` and a deterministic hash of the full porcelain dirty state. Forest snapshots compare the selected entry's stamp with the current worktree. A mismatch is surfaced as conversation/file divergence; legacy unstamped entries and non-repository sessions remain explicitly unknown. This controller-owned stamp is stripped before entries are projected into agent context.

## Stored entry shape

- `id`: immutable entry identity.
- `session_id`: owning adapter session.
- `parent_entry_id`: previous entry on this branch.
- `sequence`: deterministic insertion order across every branch in the session.
- `semantic_schema_version`: explicit persisted-event contract version. New writes use v2; projection supports v2 and N-1 v1, while unknown future versions fail closed.
- `kind`: provider-neutral semantic kind such as `user.message`, `tool.completed`, `checkpoint`, or `worker.result`.
- `payload`: semantic content plus provider metadata when needed for inspection.
- `context_visibility`: whether the projector may include the entry in restored context.

The React UI requests a forest snapshot and walks from the active leaf to the root. Raw provider notifications stay inspectable but collapsed. The legacy linear `agent_events` table is removed at schema version 6; normalized adapter events append directly to the forest.

How that projection is drawn, and what a reader is entitled to see identically whichever agent produced the turn, is [`transcript-behavior-contract.md`](transcript-behavior-contract.md): thinking presentation, grouping invariants, stream-state meanings, and the rule that a harness id is display and never behavior. It also explains why a frame the forest refuses to persist (anything ending in `.delta`) has to be assembled into a durable one by the adapter rather than left to the live window.

## Session recall

FTS5 indexes conversational `session_entries` (`user.message`, `assistant.message`, `worker.result`, plus checkpoint/compaction/branch summaries). There are two ways to search it, and they never share a code path for scoping.

**Recall in one chat.** `sessions/search_session_entries` is always `session_id = ?`. Direct chats with a NULL workspace still do not share a bucket: each chat has its own id. No LLM and no account-memory lookup. `/recall <query>` is Bridge-handled (it never auto-switches harness) and prints hits as a local assistant card. The session toolbar search box is the same API. Hits stay in this session even after the chat has ended.

**Finding a chat.** `sessions/search_chats` is the one place Bridge searches across chats, and it returns chats, never entry bodies. It is a funnel (`bridge-core/src/chat_search/`):

- **T0, parse.** Time phrases ("yesterday", "3 days ago", "last week", "in march") become a window against an injected clock; harness words become a filter; quoted text becomes a phrase. Stopwords go, and the rest are ranked by document frequency from two `fts5vocab` tables. The rarest six are kept, unknown words of four or more letters retry as a prefix, and words the index has never seen are counted.
- **T1, index.** One MATCH over `session_entry_fts` with no session filter (its rows already carry `session_id`, so going global needed no reindex) and one over `chat_digest_fts`, a per-chat digest of title, first user message and latest summary. AND first, relaxed to OR below four chats. The two lists are fused per chat by reciprocal rank, with small bonuses for several matching entries, for covering every term, and for recency. Top-level visible chats only; ended and archived chats are included and marked.
- **The gate.** The index calls itself confident only when every typed word is known, the top chat matched all of them, and it leads the next by 1.5x with at least two matches (or is the only hit). A confident search never starts a model.
- **T2, model.** Only on a `deep` request that the gate did not settle, with deep search on and Claude available. A hidden `chat_search` session runs under an empty briefing scope, so the provider has no tools. The model sees about 60-token cards and may ask Bridge for three lookups (`find_chats`, `peek_chat`, `chat_outline`); Bridge runs them, truncates them, and sends them back. The loop enforces three lookups, 2,000 lookup tokens and eight seconds of model time, and falls back to the index's answer on any overrun or error. Answers may only name chats the model was shown. Model-bound text is redacted with `secret_interception` on whole bodies before any window is cut. Usage is recorded on the hidden session as `provider.claude` with `task_family = 'chat_search'`; nothing is written to any forest.

The sidebar search shows T1 as you type; Enter opens a confident winner or runs T2. An unsure typed query pre-starts one tool-free Claude session in the background, without submitting a model turn. Enter consumes that session once; an unused session expires after three minutes. Tool-free launch replaces the coding preset and sends no tool definitions. Provider startup is outside the eight-second model budget and remains part of the reported elapsed time; quota errors are surfaced with the index fallback. `/find <query>` opens the same search from the composer, and its Bridge-handled fallback for direct clients is index-only.

This is not the memory ledger and not router learning. Account memory is a separately keyed product named `account:local`; it can come from an explicit pin, a reviewed extraction, or the separately controlled automatic-extraction mode. See [memory-ledger.md](./memory-ledger.md).

## Three independent trees

Bridge deliberately keeps these separate:

1. Workspace tree: repository → task worktree → optional worker worktree.
2. Agent tree: orchestrator → policy-authorized workers.

Worker-result entry IDs are also durable evidence references. A later sibling worker receives the exact validated typed payload resolved from the parent's active branch rather than relying on an orchestrator paraphrase; active-branch membership and session ownership are checked on every resolution.
3. Conversation tree: immutable entries → active branch.

An operation on one tree does not imply a matching operation on another.
