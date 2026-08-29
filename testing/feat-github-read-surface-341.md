# feat/github-read-surface-341 — Test Contract

## Functional Behavior

- The generated Bridge protocol registers the read-only `github/status`, `github/prs`, `github/pr`, and `github/checks` methods with matching Rust and TypeScript field names.
- Tauri commands delegate the GitHub read requests to `bridge_core::api` on `spawn_blocking`; they do not invoke model adapters or sidecars.
- In mock mode, the sidebar shows an open-pull-request list for the current repository. Each entry presents its title, branch, author, review state, and CI rollup badge.
- Selecting a pull request opens its detail view with the body, check status/conclusion and log links, plus review comment threads.
- Pull-request bodies and review comments are rendered as literal text: HTML and `<script>` content never creates executable or DOM markup.
- The unavailable GitHub CLI state shows a quiet unavailable hint. The unauthenticated state shows a `gh auth login` remediation. Neither state remains loading indefinitely.

## Unit Tests

- `bridge-protocol` generated-artifact tests prove all four GitHub read methods stay registered and drift is detected.
- `bridge_core::api` tests exercise status, list, detail, and checks reads using the fake-`gh` fixtures established by slice 1.
- `GitHubPanel.test.tsx` covers fixture PR rendering, rollup mapping, selection, and unavailable/unauthenticated states.
- `PullRequestView.test.tsx` covers check rendering, review-thread rendering, and inert handling of script/HTML-looking remote text.

## Integration / Functional Tests

- The frontend API boundary maps every native GitHub read method and has deterministic offline mock data for every fixture state.
- The generated protocol artifacts compile with the native command registry and frontend API mappings.

## Smoke Tests

- `bun run build` completes successfully.
- `bun run test` completes successfully.
- `bun run check` completes successfully.

## E2E Tests

N/A — the surface is covered by mounted jsdom component tests and deterministic Rust fake-`gh` fixtures; no browser E2E harness exists for this repository.

## Manual / cURL Tests

- Run `bun run dev` in mock mode, open a workspace with fixture PRs, and select a PR to confirm its checks and review threads appear.
- Confirm a fixture body or comment containing `<script>alert(1)</script>` appears as text and causes no side effect.
