# feat/chat-loading-and-meter — Test Contract

Branch `feat/chat-loading-and-meter`. Three slices, one PR (PR #594 is merged,
so a fresh branch carries the uncommitted usage follow-ups forward):

1. **Usage follow-through** (uncommitted work from `fix/usage-summary-honesty`):
   auto history scan with bounded batches, partial-total labeling, Codex repair
   removal. Covered by `testing/feat-usage-frontend.md` (regression section)
   and `testing/feat-usage-tracking-backend.md` (Codex §2).
2. **Chat lazy loading**: opening an existing chat shows a minimal skeleton
   while its forest loads — never the new-chat greeting.
3. **CodexBar meter port** (MIT, steipete/CodexBar): pace + adaptive refresh
   math in Rust (`bridge-core/src/meter.rs`) and TypeScript (`src/meter.ts`),
   `meter/get_meter_snapshot` + `meter/refresh_meter` protocol methods, native
   tray companion (`src-tauri/src/meter_tray.rs`), `MeterPopover` UI, and
   `bridge meter --json` CLI.

## Functional Behavior

- Selecting a chat with `forestEntries === undefined` (not yet fetched) and no
  live content renders `ChatHistorySkeleton` (`role=status`,
  `aria-label="Loading conversation"`), not `GreetingEmpty`.
- `forestEntries === []` (fetched, empty) still renders the greeting: a
  genuinely new chat is not "loading".
- Live turns render through the normal transcript path during a forest refetch;
  the skeleton never hides live content.
- Meter pace matches the Rust core case for case: deficit/reserve deltas, ETA,
  last-until-reset, 3% visibility rule with the weekly 1% exception.
- Adaptive delays match the CodexBar table: constrained 30m, recent 2m,
  warm/coding-activity 5m, idle 15m, long-idle 30m; agent-aware activity never
  postpones a tick.
- Tray menu: Show Bridge focuses the window; Refresh usage triggers the shared
  refresh; Quit exits. Tray failure never fails startup.
- `bridge meter --json` prints the registry (live codex/claude + planned
  matrix) and exits 0; other args exit 2.

## Unit Tests

- `src/components/AgentConversation.test.tsx`: skeleton on `undefined`,
  greeting on `[]`, live content through refetch.
- `src/meter.test.ts`: deficit ETA, reserve lasts-until-reset, no-timing →
  undefined, visibility thresholds, adaptive reasons.
- `bridge-core meter::tests` (7): same cases in Rust, plus registry shape and
  workday reshaping.
- `src/components/meter/MeterPopover.test.tsx` (5): live tiles with pace +
  resets, empty state, refresh/close wiring, null registry.
- Protocol: `bridge-protocol` mirror/registry tests cover the two new methods;
  `bridged` dispatch is exhaustive over them.

## Integration / Functional Tests

- `bun run build` green (tsc + vite).
- `cargo check --workspace` green, including `tray-icon` feature and the
  `generate_handler!` ↔ protocol 1:1 test.
- `bridge meter --json` prints valid registry JSON.
- Manual: open an existing chat cold → skeleton, never greeting flash; open
  Usage → Gauge button opens Meter popover; tray left-click opens popover,
  Refresh triggers usage refresh.

## Smoke Tests

- `bunx vitest run src/components/meter src/meter.test.ts src/api.boundary.test.ts
  src/components/AgentConversation.test.tsx` green.
- `cargo test -p bridge-core meter::` green.

## E2E Tests

N/A — Tauri shell; covered by manual smoke above.

## Manual / cURL Tests

- `cargo run -p bridge-client --bin bridge -- meter --json | head -30`
- `cargo run -p bridge-protocol --bin generate-protocol-artifacts` produces no
  diff after protocol edits (artifacts committed).
