# Issue 12 — Session Tree, Restoration, Budgets, and Worker UI Test Contract

This contract locks GitHub issue #12 against the merged matte UI baseline from PR #23 and the umbrella contract in `testing/issue-1-subscription-native-session-forest.md`.

## Functional Behavior

- Typed frontend/backend APIs expose a session's immutable entries, active leaf, branch leaves, restoration mode, worker leases, queued requests, usage/budget state, capability tier, and actual runtime model from SQLite without requiring a live provider process.
- Conversation projection uses only the selected active branch, preserves stable entry identity across branch switches, and renders compaction, checkpoint, and branch-summary cards. Raw provider events remain inspectable but collapsed.
- Session-tree navigation distinguishes the active path from alternate leaves and supports conversation-only rewind/fork. The UI explicitly warns that changing conversation history does not rewind or modify files.
- Restoration badges honestly distinguish hot, native, checkpoint restored, and fresh sessions; stopped sessions are not presented as equivalent.
- Context pressure and per-turn capability budget are visible. Model names are runtime detail; capability tiers remain the routing semantic and users cannot bypass policy by choosing arbitrary workers/models.
- Worker drill-down exposes role, lifecycle state, owned paths, write mode, lease status, actual model, result details, and inspectable spawn/resume/queue/reject/compact/kill reasons.
- Queue/conflict states explain why work is waiting and identify path ownership conflicts.
- Mock state includes forks, compaction/checkpoint cards, restoration modes, a queued conflict, budgets, and worker results.

## Unit Tests

- Active-branch projection includes the correct fork point and excludes inactive descendants.
- Stable entry keys do not change when switching leaves.
- Compaction, checkpoint, and branch-summary entries map to dedicated cards; raw provider payloads are collapsed/inspectable.
- Restoration label mapping covers all four modes without conflation.
- Mock-state tests cover queued worker explanation, overlapping paths, budget display, and result drill-down.
- Conversation rewind changes only `session_heads.active_entry_id`; session entries and workspace files/Git state remain unchanged.

## Integration / Functional Tests

- A SQLite-only snapshot loads the complete session tree and observability panels with no adapter runtime.
- Manual compact invokes the issue #10 controller API and session rewind invokes the forest head API.
- Every worker spawn/resume/queue/reject/compact/kill reason stored in forest/audit state has a visible inspectable representation.

## Verification

- `bun run test`
- `bun run check`
- `bun run build`
- Native Tauri app launched locally against the merged PR #23 UI baseline.
- `git diff --check origin/main...HEAD`

## Manual / E2E Notes

- In the running desktop app, select an orchestrator and alternate branch; verify the warning before rewind, restoration badge, pressure/budget meters, worker ownership/conflict explanation, result drill-down, and collapsed raw-event inspector.
