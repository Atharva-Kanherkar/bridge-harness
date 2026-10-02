# fix/snapshot-string-cap — Test Contract

## The Defect

`trim_snapshot_strings` in `src-tauri/bridge-core/src/store.rs` applies a flat
4 KiB guillotine (`SNAPSHOT_STRING_BYTES`) to **every** string in **every**
snapshot payload, unconditionally — regardless of how large the session
actually is. A 4.6 KiB assistant message in a twenty-entry chat loses its tail
to `… 578 more bytes not shown`, even though that chat's entire snapshot is a
few hundred kilobytes: four orders of magnitude under the 64 MB frame ceiling
the cap was written to protect.

The dropped bytes are still durable in SQLite (`session_entries` is the
untrimmed read), but nothing in the UI can reach them, so the rendered
transcript silently misrepresents history. Sources lists, code blocks, and long
tool output all lose their ends.

## The Fix

Turn the cap into a **budget**. Trimming stays available for the pathological
session it was written for and stops firing on ordinary ones.

1. `session_entry_window` measures the window's stored payload bytes before
   deciding anything.
2. Under `SNAPSHOT_PAYLOAD_BUDGET_BYTES` (24 MiB, against a 64 MB frame), every
   payload travels **exactly as stored** and `trimmed_payloads` is 0.
3. Over budget, the window picks the **largest** per-string cap from a
   descending ladder — 1 MiB, 256 KiB, 64 KiB, 16 KiB, 4 KiB — whose projected
   size fits the budget, and trims with that one. The 4 KiB floor is today's
   behaviour, so no session is ever trimmed harder than it is now.

Unchanged by design: the `dataUri` exemption for pasted images, char-boundary
safety, field-structure preservation, and `SNAPSHOT_ENTRY_WINDOW` (an
entry-count bound the UI already reports honestly through `returned`/`total`).

## Functional Behavior

- A session whose window fits the budget → payloads byte-identical to storage;
  `trimmed_payloads == 0`; no `more bytes not shown` marker anywhere.
- The reported message: a ~4.6 KiB single string now round-trips whole.
- A session whose window exceeds the budget → trimmed to the loosest ladder rung
  that fits; `trimmed_payloads` counts the entries actually shortened; the
  snapshot fits in one client frame.
- Trimming preserves JSON structure: `title`, `text`, and nested `data` fields
  stay present and stay strings.
- `data:image/...` URIs are never trimmed at any cap.
- Truncation is never silent — a shortened string carries the marker.
- `session_entries` (compaction, context projection) remains untrimmed always.

## Unit Tests

In `src-tauri/bridge-core/src/store.rs`:

- `trimming_a_payload_never_splits_a_character` — multi-byte char straddling the
  cap walks back to a boundary; no U+FFFD. (existing, re-pointed at explicit cap)
- `a_string_under_the_cap_is_never_touched` — no marker, no mutation.
- `the_trim_ladder_picks_the_loosest_cap_that_fits` — projected-size selection
  returns the largest rung under budget, and the 4 KiB floor when none fit.
- `the_snapshot_window_leaves_an_ordinary_session_untrimmed` — the regression
  test for this bug: a 4.6 KiB string survives whole, `trimmed_payloads == 0`.
- `the_snapshot_window_trims_oversized_payload_strings_in_place` — structure
  survives trimming when the budget is actually exceeded. (existing, reshaped)
- `the_snapshot_window_preserves_durable_image_data_uris` — image stays
  decodable after reload. (existing, reshaped)

## Integration / Functional Tests

- `a_snapshot_of_a_pathological_session_stays_inside_the_client_frame_limit`
  (existing) must still pass unchanged: 2,000 fat `command.started` entries,
  >64 MB untrimmed, <64 MB after, at least 5x smaller. This is the guard that
  proves the budget did not become a licence to blow the frame.
- `session_forest_snapshot_with_repository_state` round-trip keeps
  `entryWindow.trimmedPayloads` accurate.

## Smoke Tests

- `cargo test --manifest-path src-tauri/Cargo.toml -p bridge-core store::` green.
- `bun run check` green (no protocol or type drift — this change adds no wire
  fields).

## E2E Tests

N/A — no protocol surface changes, so `src/protocol/generated/protocol.ts` and
`docs/protocol/schemas/` are untouched and the frontend needs no edit. Manual
verification below covers the user-visible path.

## Manual Tests

1. Build and open the app against a real data directory.
2. Open a chat containing a message longer than 4 KiB (the reported one: an
   answer ending in a `Sources:` list).
3. Reload / reopen the chat.
4. Expected: the message renders in full, with no `… N more bytes not shown`
   suffix, and the sources list is complete.

Direct database check, no UI required:

```bash
sqlite3 ~/Library/Application\ Support/bridge/bridge.db \
  "SELECT length(payload) FROM session_entries ORDER BY length(payload) DESC LIMIT 5;"
```

Any payload between 4 KiB and the budget must now reach the UI whole.
