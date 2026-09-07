# fix/issue-455-chat-ux-gaps — Test Contract

Follow-up to the issue #455 chat UX overhaul (merged in PR #463). A read-across
audit of `main` found the feature substantially landed but with a red quality
gate and two residual contract violations. This branch closes those so `bun run
check` and `bun run test` are green and the remaining locked items in
`testing/feat-issue-455-chat-ux.md` hold.

Scope is deliberately narrow — three safe, verifiable fixes. It does not
re-open shipped behavior.

## Functional Behavior

1. **TypeScript gate is green.** `bun run check` (`tsc -b`) reports no errors.
   The `AsideChat.test.tsx` "delivers attachments pasted onto a queued
   follow-up" case types its `onSend` mock with `(text, attachments)` so
   `onSend.mock.calls[1]` is a typed tuple, not `[]`.
2. **Approval card never leaks a raw policy code outside its disclosure**
   (contract §C: "policy code only inside a disclosure"). The remediation
   paragraph shows human prose only; the machine reason enum
   (e.g. `owned_path_provenance_required`) appears solely inside the "Policy"
   `<details>` at the bottom of the card.
3. **Claude compatibility report is honest about file changes.** After the
   #463 normalization, Claude `Edit`/`Write`/`MultiEdit`/`NotebookEdit` emit
   `file_change.*` events, so the built-in compatibility contract advertises
   the `file_changes` capability for Claude, and the golden fixture matches.

## Unit Tests

- `AsideChat` suite (`src/components/AsideChat.test.tsx`) — full file compiles
  and every case passes under vitest, including "delivers attachments pasted
  onto a queued follow-up" (asserts the queued send delivers one attachment
  with `mediaType === "image/png"`).
- `builtin_compatibility` Rust module:
  - `compatibility_report_is_deterministic_and_schema_versioned` — generated
    report equals `testing/fixtures/builtin-compatibility-report-v1.json`
    (regenerated to include `file_changes` for Claude).
  - `descriptor.capabilities == contract.capabilities` assertion stays green.
  - Existing Claude event-fixture drift test stays green (behavior already
    emits `file_change.*`; only the capability advertisement changes).

## Integration / Functional Tests

- N/A beyond the module tests above — no new wire methods or cross-crate
  contracts are introduced.

## Smoke Tests

- `bun run check` exits 0.
- `bunx vitest run src/components/AsideChat.test.tsx` passes.
- `cargo test --manifest-path src-tauri/Cargo.toml -p bridge-core builtin_compatibility::`
  passes.

## E2E Tests

- N/A — no user-journey surface changes; the approval-card edit is a
  presentation-only removal of a redundant raw enum already shown in the
  disclosure.

## Manual / cURL Tests

- Approval card visual check (mock mode, `bun run dev`): open the delegation
  write-scope approval (mock `entry-8b`). The body reads as human remediation
  prose with no monospace `owned_path_provenance_required` token above it; the
  raw enum is reachable only by expanding "Policy".

## Out of scope (documented, not changed)

- Contract §D "session switch keeps the previous forest rendered until the next
  resolves": already mitigated on `main` — `App.tsx` seeds `forest` from a
  per-session cache and `startSerialPoll` fires the first refresh immediately,
  so revisits never flash and first visits show only unavoidable fetch latency.
  Rendering the previous session's forest during that window would surface
  another session's interactive approval cards (a correctness risk), so it is
  intentionally left as-is.
