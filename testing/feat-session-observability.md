# feat/session-observability — Test Contract

Locked before implementation. Covers issue #446 (search the session forest and
jump to an entry) and the larger observability ask it sits inside: a
full-fidelity durable record of what every harness actually did, an inspectable
JSONL export of it, and a transcript surface that makes failures findable.

## Why these three land together

Bridge already normalizes every harness (Codex, Claude, OpenCode, Cursor, Grok)
into one event vocabulary, and `store::session_event_in_transaction` is the
single door through which a normalized event becomes durable history. That door
currently drops four kinds on the floor — `turn.started`, `turn.completed`,
`usage.updated`, `plan.updated` — so after a reload the forest can no longer say
where a turn began, what it cost, or what the model planned. Everything else in
this contract is downstream of closing that hole: an export of a record with
holes in it is a misleading artifact, and a "where did it fail" UI cannot group
by turn if turn boundaries are not history.

## Non-goals

- No new per-event storage table. The session forest stays the single
  authoritative history; this work widens what reaches it, never duplicates it.
- No semantic/embedding search, no cross-session or cross-repo search. Recall
  stays scoped to one `session_id`, exactly as `session_recall.rs` documents.
- No change to what the rendered conversation shows. Turn/usage/plan entries are
  control records, not chat cards.
- No export of secrets. The export reads the same durable rows the UI reads;
  it introduces no new read path and no new redaction policy.

---

## Functional Behavior

### A. Durable recording (`bridge-core`)

A1. `turn.started`, `turn.completed`, `usage.updated` and `plan.updated` are
    written to `session_entries` with `context_visibility = "hidden"`.

A2. A `hidden` entry is **never** projected into restored model context.
    `context::project_render_entry` already refuses anything that is not
    `eligible` (except `worker.result`); this must stay true with the new kinds
    present.

A3. A `hidden` entry is **never** an FTS recall hit. The
    `session_entries_ai_fts` trigger indexes only
    `context_visibility IN ('eligible','visible')`, and none of the four kinds
    is in `INDEXABLE_KINDS` either.

A4. Streaming frames stay transient and are still never stored: `message.delta`,
    `reasoning.delta`, `tool.progress`, and the `question.settled` control
    signal return `sequence = 0` and write no row.

A5. `reasoning.completed`, `session.error`, `tool.*` terminal events and
    `context.compacted` remain durable exactly as today — this change adds
    kinds, it removes none.

A6. Recording is adapter-agnostic. A normalized event from any harness takes the
    same path, so the assertion is made once against the normalizer output, not
    five times per provider.

### B. JSONL export (`sessions/export_session_transcript`)

B1. Writes one newline-delimited JSON file and returns its path, line count and
    byte size. Nothing is returned inline: a long session is a file, not a
    string on the wire.

B2. Line 1 is a `header` record: schema version, export timestamp, scope, and
    the session's identity (id, harness, model, effort, label, status,
    workspace, created/ended timestamps).

B3. Lines 2..n-1 are `entry` records in ascending `sequence`, one per forest
    entry, each carrying: `sequence`, `entryId`, `parentEntryId`, `kind`,
    `createdAt`, `contextVisibility`, `tokenEstimate`, `providerEventId`, the
    flattened `role`/`status`/`title`/`text`, the full `data` payload, the
    `providerMeta`, a derived `turnIndex`, and `onActiveBranch`.

B4. The final line is a `footer` record: per-kind counts and a
    `sha256` digest over the entry lines, so two exports of the same session are
    comparable and a truncated file is detectable.

B5. `scope: "active_branch"` exports only the entries on the head's active
    branch. `scope: "forest"` exports every entry of the session, including
    abandoned branches. `onActiveBranch` is set correctly in both.

B6. `includeHidden: false` omits the `hidden` control entries; the default is
    `true` (the record is the product).

B7. Absent `destinationPath`, the file lands in
    `<data_dir>/exports/<sessionId>-<timestamp>.jsonl`. A supplied
    `destinationPath` is used verbatim and its parent directory is created.

B8. Every emitted line parses as standalone JSON. A payload containing newlines
    never breaks the line framing.

B9. An unknown `sessionId` is an error, not an empty file.

### C. Observability transcript pane (`src/components/TranscriptPane.tsx`)

C1. Facet chips over the loaded stream, each with a live count: All, Messages,
    Thinking, Tools, Turns, Usage, Approvals, Delegation, Problems. Selecting a
    facet filters the stream; the text filter still applies on top.

C2. **Problems** is the point of the pane. An event is a problem when it is a
    failed tool (`tool.*` with `status` in `failed`/`error`), any `*.error`/
    `error` kind, `compaction.failed`, `session.resume_failed`, a
    `worker.result` whose status is not `completed`, or a `turn.*` that ended
    `failed`. Problems are marked in the row, counted in the chip, and the chip
    is the only place in the pane that uses a non-achromatic color.

C3. The pane reports a problem count even when the Problems facet is not
    selected, so a reader sees that something went wrong without hunting.

C4. Turn grouping: rows carry the index of the turn they fall in, derived by
    walking `turn.started` boundaries in sequence order. Events before the first
    boundary belong to turn 0.

C5. Reasoning rows render their assembled thought text, not a JSON blob.

C6. An **Export JSONL** action calls the new method and, on success, shows the
    written path with a copy-path affordance. On failure it shows the error and
    leaves the pane usable.

C7. Empty state ("no events yet") and no-match state (filter/facet excludes
    everything) are distinct and both stated in words.

### D. Forest search — issue #446

D1. Results paginate: the search returns at most one page, and a
    "Show more" affordance requests the next page. Exhausted results hide the
    affordance.

D2. A hit jumps to and reveals its entry (already true; must remain true).

D3. Visibility rules are respected — non-`eligible`/`visible` entries are not
    hits (guaranteed by the FTS trigger, asserted here).

D4. Empty query, zero-hit and error states are each distinguishable in words.

---

## Unit Tests

### Rust — `bridge-core`

- `store::tests::turn_boundaries_are_durable_but_never_context_eligible` —
  `turn.started`/`turn.completed` store with `sequence > 0` and
  `context_visibility = 'hidden'`.
- `store::tests::usage_and_plan_updates_are_recorded_as_hidden_history` — same
  for `usage.updated` and `plan.updated`, and the payload keeps its token
  counts.
- `store::tests::streaming_frames_are_still_never_stored` — `message.delta`,
  `reasoning.delta`, `tool.progress`, `question.settled` all return sequence 0
  and leave `session_entries` empty.
- `context::tests::hidden_control_entries_are_not_projected_into_context` — a
  forest containing turn/usage entries projects the same context as one without.
- `session_recall::tests::hidden_control_entries_are_not_recall_hits` — a
  `usage.updated` entry whose payload contains the query term returns no hit.
- `transcript_export::tests::header_entries_and_footer_frame_one_session` —
  header first, entries in sequence, footer last; every line parses.
- `transcript_export::tests::active_branch_scope_excludes_abandoned_branches` —
  a forked forest exports fewer lines under `active_branch` than under `forest`,
  and `onActiveBranch` agrees.
- `transcript_export::tests::excluding_hidden_drops_only_control_entries`.
- `transcript_export::tests::payload_newlines_do_not_break_line_framing`.
- `transcript_export::tests::footer_digest_changes_when_an_entry_changes`.
- `transcript_export::tests::unknown_session_is_an_error`.
- `transcript_export::tests::default_destination_lands_under_the_data_dir`.

### Rust — `bridge-protocol`

- `messages::tests::params_types_are_named_after_their_method` (existing, must
  keep passing with `ExportSessionTranscriptParams`).
- `messages::tests::every_method_is_contracted` (existing coverage gate).
- `tsgen::checked_in_artifacts_match_the_contract` (existing drift gate) — the
  regenerated `protocol.ts` and JSON schemas are committed.
- `sessions::tests::export_params_refuse_unknown_fields`.

### Frontend — Vitest

- `TranscriptPane.test.tsx`
  - `facet chips count the stream they filter`
  - `problem events are counted even when the problems facet is not selected`
  - `selecting problems shows only failures`
  - `text filter composes with the selected facet`
  - `rows carry the index of the turn they fall in`
  - `reasoning rows show the thought text`
  - `export writes a file and shows its path`
  - `export failure is shown without breaking the pane`
  - `empty stream and no-match read differently`
- `SessionRecallSearch.test.tsx`
  - `a full page offers to load more`
  - `loading more appends the next page`
  - `a short page does not offer to load more`
  - `existing: a hit jumps to its entry` (must keep passing)
- `src/transcript/*` — existing golden and reducer suites must stay green; the
  four golden fixtures prove the four harnesses still reduce identically.

## Integration / Functional Tests

- `bridge-deck` (`src-tauri`) command-signature gate: the new Tauri command's
  argument types match the wire params struct.
- `bridged` dispatch: the method is reachable over the daemon socket and a
  registry↔handler 1:1 test covers it.
- `protocol_mirror::result_payloads_mirror_core`: the core result struct and the
  wire DTO stay in lockstep.
- Round-trip: export a session written by the store, re-read the file, and
  assert the entry lines reproduce `session_entries` for that session.

## Smoke Tests

- `bun run check` — `tsc -b` and `cargo check --workspace` both clean.
- `bun run test` — sidecar `node:test`, `vitest run`, `cargo test --workspace`.
- `bun run build` — production bundle builds.
- `cargo run -p bridge-protocol --bin generate-protocol-artifacts` from the
  worktree root produces no diff after the change is committed.

## E2E Tests

N/A as automation — Bridge has no driven-desktop E2E harness. Replaced by the
manual pass below, which is the project's normal substitute.

## Manual Tests

Run against a real session in the desktop app (or `bun run dev` for the pane's
mock-data behavior):

1. Open a chat that has run at least one turn with a tool call. Open the
   **Transcript** dock pane.
2. Confirm the facet chips appear with counts and that All ≥ every other facet.
3. Confirm turn numbering increments at each `turn.started`.
4. Force a failure (e.g. ask for a command that exits non-zero). Confirm the
   Problems chip count increments and the row is marked.
5. Click **Export JSONL**. Confirm a path is shown and copyable.
6. Verify the artifact from a shell:

   ```sh
   f=<pasted path>
   head -1 "$f" | jq '.type, .schemaVersion, .session.harness'
   tail -1 "$f" | jq '.type, .counts, .digest'
   # every line is standalone JSON
   while IFS= read -r line; do printf '%s' "$line" | jq -e . >/dev/null || echo "BAD LINE"; done < "$f"
   # the record actually contains what the UI showed
   jq -r 'select(.type=="entry") | .kind' "$f" | sort | uniq -c
   jq -r 'select(.type=="entry" and .kind=="reasoning.completed") | .text' "$f" | head
   ```

7. Press the in-chat search affordance, search a term that appears many times,
   and confirm a page of hits plus a working **Show more**.
8. Search a term that only appears inside a `usage.updated` payload; confirm
   zero hits (visibility rules hold).
