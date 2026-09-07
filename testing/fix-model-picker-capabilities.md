# Model picker capabilities — Test Contract

## Functional Behavior
- Chat and orchestrator pickers show model names without internal routing tier badges.
- Successful provider discovery supplies the selectable catalogue; fallback-only IDs are not appended. Separate real releases remain distinct.
- Only the selected model’s advertised thinking levels are offered. Unknown or empty capabilities never invent a ladder; automatic model selection uses the adapter default.
- Provider-supported values including max and ultra pass through chat transports without the delegation effort enum restricting them.
- Invalid effort is rejected before teardown or mutation. Model switches clear incompatible saved effort. Effort changes take effect on the next provider start, including an already warm session.
- Claude discovery uses the same installed SDK as launch, has a finite deadline, and avoids loading project integrations. Repeated refreshes do not overlap per adapter.

## Unit Tests
- Catalogue regression tests cover live-only models, distinct releases and supported effort propagation.
- Picker tests cover no tier labels, absent capability data, default selection, disabled controls and refresh rejection.
- Protocol tests round-trip max and ultra. Core tests verify supported and invalid effort validation.

## Integration / Functional Tests
- Run sidecar tests and the repository build and test commands.
- Verify effort-only updates restart warm providers and incompatible effort cannot reach adapter launch.

## Smoke Tests
- Inspect actual installed SDK catalogue without submitting a model turn where available.

## E2E Tests
- Manual desktop: open chat and orchestrator pickers; select a model and supported thinking level, send a turn, then switch model. Names are distinct and history remains available.

## Manual / cURL Tests
- Desktop interaction requires the running Bridge build; report any checks that cannot be performed.
- Required before PR: bun run build; bun run test.
