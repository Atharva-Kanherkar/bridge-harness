# codex/issue-51-usage-health — Test Contract

## Functional Behavior

- The compact usage rail remains visible for both Codex and Claude, including when a provider has not exposed a stable quota API.
- Every displayed metric is explicitly labeled `reported`, `measured`, or `estimated`; provider quota windows parsed from account APIs are labeled `reported`.
- A provider with no known quota shows `Limit unknown` and never displays an invented percentage, total, or reset time.
- Provider detail shows plan/model context when known, quota windows and reset times when reported, and token/context signals when available.
- Context pressure is derived from explicit thresholds and includes an explanation of the triggering threshold.
- Projected exhaustion alerts are produced only when enough measured history exists to support a projection; otherwise no projection is fabricated.
- Usage history entries retain work-unit, harness/provider, model, outcome, source, token/context, and timestamp metadata and appear in newest-first order.
- Invalid percentages are clamped to the 0–100 display range without hiding their provenance.

## Unit Tests

- `extractUsageSnapshot` labels parsed provider metrics as reported and preserves plan/model metadata.
- `extractUsageSnapshot` returns no fabricated quota windows when provider limits are absent.
- `contextPressure` returns explainable healthy, elevated, high, and critical states at the locked thresholds.
- `projectUsageExhaustion` requires adequate history and returns an explainable alert only for projected exhaustion within the configured horizon.
- `buildUsageHistory` ties records to work units, harnesses, models, outcomes, and sources in newest-first order.
- Existing reset-formatting, window-labeling, and latest-snapshot tests continue to pass.
- Usage UI rendering tests verify source labels, unknown limits, pressure explanations, and work-unit history.

## Integration / Functional Tests

- `App` continues to consume native `account-usage` events and renders provider entries even when a provider returns no quota windows.
- The detailed usage panel combines account quota snapshots with session-ledger history without inventing missing values.
- `bun run build` completes successfully with strict TypeScript and Vite.
- `bun run test` completes successfully for the sidecar, frontend, and Rust crates.

## Smoke Tests

- With no provider usage response, the top-right rail still renders Codex and Claude as `unknown`.
- With a reported quota snapshot, the rail shows the highest reported window percentage and its provenance.
- Opening the usage panel exposes reset information, context health, and recent work units without obscuring the main conversation.

## E2E Tests

N/A — live provider account APIs require authenticated Codex and Claude installations and are not deterministic in CI. The native event-to-React boundary is covered by integration behavior and manual smoke testing.

## Manual / cURL Tests

1. Run `bun run dev` and open `http://127.0.0.1:1420`.
2. Confirm the compact top-right rail always includes Codex and Claude.
3. Hover or focus the rail to open details; confirm unavailable quota fields say `Limit unknown` and carry a source label.
4. In the Tauri app with authenticated providers, run work on multiple models and confirm reported quota windows, reset text, context pressure explanations, and newest-first work-unit history.
5. Confirm no UI copy claims a percentage, reset, plan, or projected exhaustion when that value is absent from provider or measured data.
