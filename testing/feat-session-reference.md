# feat-session-reference — Test Contract (closes #658)

Stack PR 3 (builds on PR #660 backend + #663 UI). Session IDs become
portable references: resolvable, copyable, and chip-rendered in composers.

## Functional Behavior

1. **`sessions/resolve_reference(id)` protocol method** (fully contracted).
   Takes a session id or a forest entry id and returns a typed descriptor:
   - session ids → `{ kind: "session", sessionId, label, harness, workspaceId,
     parentSessionId, depth, restorationMode, continuationFidelity,
     activeEntryId, latestCheckpointEntryId, updatedAt, authorized: true }`
   - entry ids → `{ kind: "entry", sessionId, entryId, kind, sequence,
     summary (title-derived), createdAt, authorized: true }`
   - an id that exists but the caller may not see, and an id that exists
     nowhere, return the **same-shaped** `{ kind: "unknown", authorized:
     false }` — no existence leak.
   - malformed ids (wrong shape, empty) → typed `invalid` error.
2. **Id format**: the raw internal id is too easy to mistype; a short public
   alias `brio_<8 chars of the uuid>` is what the UI copies and the composer
   recognizes. `resolve_reference` accepts both raw ids and the alias, plus
   the `@session:<alias-or-id>` mention spelling.
3. **Copy affordance**: every sidebar session row and the session header get
   a "Copy chat ID" action that copies the public alias. Forked-session
   breadcrumbs (from #280 PR 2) copy too.
4. **Composer detection**: typing or pasting a `brio_…` token or an
   `@session:…` mention in the ComposerPill is detected, resolved before
   send (debounced), and rendered as a chip (title + last activity +
   restoration mode), not raw text. Unresolvable tokens stay as literal
   text with a subtle warning underline — never a broken chip.
5. **Read-only actions on a resolved reference**: the chip can inject the
   reference's latest checkpoint summary into the current session's composer
   as context ("pull into this chat"), mirroring worker-result evidence
   semantics — the same staleness rules apply (checkpoint label + age shown).
6. **"Referenced elsewhere" indicator**: a session whose id has been resolved
   into another session's composer in this app session shows a faint mark
   (dot in the sidebar row). App-local scope; no cross-window claims.
7. No existence leak, no new authorization path: resolve uses the ownership
   and active-branch checks already implemented for worker-result resolution.

## Unit Tests

**Rust (`bridge-core/src/sessions.rs` additions)**
- `resolve_reference_resolves_a_session_id_to_its_descriptor`
- `resolve_reference_resolves_an_entry_id_to_its_entry_descriptor`
- `resolve_reference_accepts_the_public_alias_and_the_mention_spelling`
- `resolve_reference_does_not_leak_existence` — unknown id vs authorized-
  denied id return identical shapes
- `resolve_reference_rejects_malformed_ids`

**Vitest (colocated)**
- `referenceChip.ts` tests: detection of `brio_`/`@session:` tokens, debounce
  resolve, unresolvable → warning state, chip model shape.
- `ComposerPill` tests: chip renders with title/activity/mode; a resolved
  chip's "pull into this chat" appends the checkpoint summary to the draft.
- App: copy-chat-id on a sidebar row copies the alias; a pasted alias in the
  composer becomes a chip after the mocked resolve.

## Integration / Functional Tests

- Protocol artifacts regenerate cleanly (`resolve-reference-params.json` /
  `resolve-reference-result.json` in `docs/protocol/schemas/`, `protocol.ts`
  maps `sessions/resolve_reference`), the command-signature drift gate stays
  green, and `bridgeApi.resolveReference` binds (mock + test).
- `api::resolve_reference` end-to-end: create chat → append entries → resolve
  session id and an entry id; unknown and unauthorized shapes identical.

## Smoke Tests

- `bun run test` (full vitest + cargo), `bun run check`, `bun run build` all
  green.

## E2E Tests

N/A for the backend/UI slice split — the full journey is covered by the App
integration test above.

## Manual / cURL Tests

Manual pass: copy a chat ID from the sidebar, paste into another chat's
composer, see the chip, pull it in as context.