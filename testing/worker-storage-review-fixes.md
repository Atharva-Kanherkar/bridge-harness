# Connector review regression contract

Extends the worker/storage contract with four reviewed failures.

## Functional behavior and tests

- A default or automatically selected worker harness never overrides exclusions.
  Test Shadow and Disabled routing with excluded harness/model, an eligible
  alternative and no alternative. Explicit overrides must not bypass exclusions.
- Git callers asking for a bounded prefix receive that prefix and a truncation
  flag even when total output exceeds 16 MiB. Drain pipes with the existing
  wall-time deadline; whole-output callers still refuse oversized output.
- Archived history uses the shared conversation projection/grouping/rendering.
  Test reasoning, tools, diffs, errors, approvals and delegation entries, including
  a page without user/assistant messages and a tool spanning two fetched pages.
  Historical approval/question controls cannot perform actions.
- An archived root exposes its workers/asides and deeper descendants without
  unarchiving or starting anything. Search can find a root through descendant
  names. Descendant reads remain under their root; only an explicit root unarchive
  changes visibility. Independently archived children stay archived afterwards.

## Verification

- Run focused Rust/Vitest regressions before the full suite.
- Regenerate protocol artifacts and run protocol/command-surface drift checks.
- Run `bun run build` and `bun run test` before updating the PR.
- Record any CI account-level failure separately from local results.
