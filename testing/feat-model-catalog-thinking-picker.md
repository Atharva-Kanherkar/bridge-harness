# main-worker-d56c582d-3051-4a4b-94f2-e897dee37fa8 — Test Contract

## Functional Behavior

- The chat composer model picker lists every selectable model returned by every available harness, grouped by harness, without collapsing distinct releases such as Claude Fable 5 and Claude Fable 5.1.
- Models expose every supported thinking or reasoning-effort choice, and selecting one persists both the model and its effort for the next turn.
- Cursor models flow from runtime adapter discovery into the same picker and degrade to an unavailable provider state when Cursor cannot be probed.
- Provider catalogues can be refreshed at runtime. A refreshed model or display name appears without a Bridge code change or app restart, while a failed refresh preserves a usable cached catalogue and exposes its state.
- Transcript reasoning events pass through the normalized transcript reducer/grouping pipeline and render as a user-visible, accessible, collapsible thought stream.
- The picker supports search across provider, model name, tier, and capability; it uses achromatic Graphite & Paper v2 chrome and Tailwind v4 utilities only.

## Unit Tests

- `ChatModelControl.test.tsx` covers grouped full-catalog rendering, distinct model releases, Cursor visibility, search, badges, effort selection, refresh, empty/unavailable states, and keyboard/accessibility behavior.
- Transcript component/reducer tests cover reasoning event normalization, streaming and completed states, collapsed/expanded presentation, and preservation of harness-neutral behavior.
- Rust adapter/catalog tests cover dynamic discovery normalization, display names, effort variants, Cursor catalogue propagation, cache fallback, and unavailable CLIs.
- Existing model-profile and transcript contract tests remain green.

## Integration / Functional Tests

- Adapter discovery returns descriptors through the sole Tauri API round-trip and the composer renders the resulting catalogue.
- Changing a model or effort updates the selected model profile used when starting a turn.
- Reasoning output emitted by a harness is reduced and rendered through the normalized transcript pipeline.

## Smoke Tests

- Open the composer picker, search for a model, select a model and effort, close and reopen the picker, and confirm the selection remains visible.
- Trigger catalogue refresh with one or more provider CLIs absent; the app remains responsive and marks those providers unavailable.
- Run a reasoning-capable conversation and expand/collapse its thought stream.

## E2E Tests

- N/A — this repository has colocated Vitest and Rust coverage for this desktop flow; native-provider availability varies by developer machine.

## Manual / cURL Tests

- Run `bun run check`, `bun run test`, and `bun run build`; all must exit successfully.
- In the Tauri app, verify all installed-provider models and effort variants, including Cursor when installed, appear in the redesigned picker.
- Inspect changed JSX and confirm no new stylesheet or inline `style` prop was introduced.
