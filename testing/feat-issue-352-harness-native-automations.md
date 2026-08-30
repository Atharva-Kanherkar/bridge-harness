# Harness-native automations — Test Contract

## Functional Behavior

- The sidebar row formerly labeled **Automations** is labeled **Marketplace**, uses the Marketplace destination, and is active whenever the Marketplace surface is open.
- Marketplace has two distinct top-level sections: **Catalog** and **Automations**. Catalog retains the existing Agents, Plugins, and Skills experiences and their install/manage behavior.
- Opening Marketplace defaults to Catalog. Selecting Automations renders the existing unified automation list within Marketplace without maintaining a second standalone app destination.
- Every automation and provider state remains attributed to its owning harness: Claude Code, Codex, or Cursor.
- Provider capabilities are returned as protocol data and drive every control:
  - Claude Code: list, create, edit, and delete through `scheduled_tasks.json`; no pause, resume, or run-now capability.
  - Codex: list, pause, resume, and delete through its native automations SQLite store; no Bridge-emulated create, edit, or run-now capability.
  - Cursor and OpenCode: explicit unavailable/unsupported state because neither has a native automation store or API; no mutation controls. Bridge ships four harnesses and the strip names all four.
- Claude create accepts a non-empty prompt and a five-field cron expression whose every field parses (`*`, `n`, `a-b`, any of those with `/step`, plus month and weekday names) against that field's range, writes a native task with a generated ID and millisecond creation timestamp under Claude's lock, and preserves unknown top-level/task data.
- Claude edit changes only the supplied native task's prompt, cron expression, and recurring flag while preserving its ID, creation timestamp, firing metadata, and unknown fields.
- Unsupported provider/action combinations fail explicitly in the backend even if a client bypasses capability-driven UI. Capabilities decide whether a write is offered; a separate provider match decides which native store it lands in, so widening a capability list can never redirect one harness's write into another's file. No path panics.
- Existing Claude/Codex schedules keep their current deletion behavior; Codex schedules keep their current pause/resume behavior.
- Bridge never calculates future occurrences, queues a run, invokes a harness prompt as a substitute for a native run-now API, or owns automation lifecycle/execution state.

## Unit Tests

- `bridge_core::automations` catalog test returns Claude, Codex, and Cursor provider capabilities and keeps native provider attribution.
- Claude create test validates the written native shape, generated identity, file permissions/locking path, and preservation of unknown top-level data.
- Claude edit test updates supported fields while preserving unknown task fields and immutable metadata.
- Invalid Claude drafts and missing edit targets return clear errors without corrupting the native file, and a cron Claude Code cannot parse never reaches disk.
- Capability enforcement rejects create/edit/run-now or pause/resume where the selected provider does not support them.
- Existing Codex pause/resume/delete and Claude delete tests continue to pass.
- `bridge_protocol::messages::automations` serialization tests cover provider capabilities, Claude create/edit payloads, and unknown-field rejection.
- `navigationHistory` tests confirm Marketplace remains one destination and the obsolete Automations view is not representable, enforced by an exhaustive `Record<AppView, true>` that fails `bun run check` if the view returns.
- `AutomationsPanel` tests cover provider capability rendering, supported Claude create/edit controls, Codex pause/resume controls, Cursor and OpenCode unsupported states, absence of run-now controls, and withholding of both chips and controls from a store Bridge cannot read.
- Sidebar/App navigation tests confirm the Marketplace label and route.
- Marketplace tests confirm Catalog and Automations sections coexist and Catalog's nested resource tabs remain Agents, Plugins, and Skills.

## Integration / Functional Tests

- Frontend API methods serialize Claude create/edit payloads to the matching native automation protocol methods and refresh the catalog after success.
- Tauri and daemon dispatch paths expose the same automation catalog and mutation methods.
- Marketplace Catalog switching does not trigger automation mutations or regress catalog mounting.
- Existing schedules loaded from both native stores render together and execute only actions present in their returned capabilities.

## Smoke Tests

- `bun run build` passes.
- `bun run check` passes.
- `bun run test` passes.
- Focused Vitest suites for Marketplace, AutomationsPanel, App/sidebar navigation, and navigationHistory pass.
- Focused Rust tests for `bridge-core` automations and `bridge-protocol` automation serialization pass.

## E2E Tests

N/A — this repository has no desktop E2E harness for mutating real Claude/Codex user stores. Native-store behavior is covered with isolated temporary-home fixtures and UI behavior with Vitest.

## Manual / cURL Tests

- Launch the app, select **Marketplace** in the sidebar, and verify Catalog opens with Agents, Plugins, and Skills intact.
- Switch to **Automations** and verify Claude, Codex, and Cursor capability states are visible.
- With fixture/native schedules present, verify Claude exposes Edit/Delete, Codex exposes Pause or Resume/Delete, and Cursor and OpenCode expose no mutation controls.
- Select Claude creation, submit a five-word non-cron such as `every day at nine am` and verify it is refused by field name rather than written, then submit a valid five-field cron and verify the new entry appears after refresh.
- Verify no provider displays **Run now** because none of the supported native automation surfaces provides a safe run-now API.
- cURL is N/A — automation calls are local Tauri/Bridge protocol operations, not HTTP endpoints.
