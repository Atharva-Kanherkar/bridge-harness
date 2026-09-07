# feat/rich-content-rendering — Test Contract

## Functional Behavior

- Mermaid fenced blocks render inline and retain a readable source fallback when rendering fails.
- Inline and display math render with KaTeX without interpreting ordinary price text as math.
- HTML fenced blocks render in an iframe with an empty sandbox and no script or same-origin capability.
- Orchestrator and worker prompts advertise the supported rich-content formats.
- Reconciliation with `main` preserves newer runtime and protocol behavior outside this feature.

## Unit Tests

- Markdown renderer tests cover block detection, valid and invalid math, price false positives, and iframe sandboxing.
- Rust prompt tests cover rendering guidance in orchestrator and worker briefings.

## Integration / Functional Tests

- `bun run build` completes successfully.
- `bun run test` completes successfully, including frontend and Rust suites.

## Smoke Tests

- The production frontend bundle builds with KaTeX assets and lazy-loaded Mermaid support.

## E2E Tests

N/A — no automated desktop E2E harness is configured for this change.

## Manual / cURL Tests

N/A — this is an in-app renderer change with no HTTP endpoint.
