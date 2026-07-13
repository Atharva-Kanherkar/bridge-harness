# Issue 37 Contract: Measured cross-harness continuation fidelity

- Every session exposes one controller-owned continuation marker: `native`, `projected_at_boundary`, or `projected_mid_turn`.
- Native/hot resume records `native`; projected starts classify against durable parent state, with checkpoint/compaction, worker-result verification, or a completed parent turn treated as a phase boundary.
- A cross-harness request received mid-turn is queued while the parent remains active, then becomes dispatchable at the boundary. Races remain visible as `projected_mid_turn` rather than being mislabeled.
- The Deck surfaces both projected continuation states and gives mid-turn projection the stronger degraded warning.
- Historical child sessions migrate conservatively from restoration state without rewriting conversation history.

## Local refund checkpoint

Verify classification, cross-harness defer/release behavior, native and projected persistence, migration, Deck rendering, full frontend/Rust suites, type checks, and `git diff --check` before the issue-only PR.
