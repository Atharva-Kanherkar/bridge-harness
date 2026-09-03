# fix/github-pane-hang — Test Contract

## Functional Behavior

- Opening the GitHub pane keeps the complete basic list of up to 100 open pull requests.
- Expensive CI, review, and mergeability enrichment is limited to the five newest pull requests so large repositories do not make the pane appear frozen.
- Opening a pull request still loads its complete detail and checks; the list optimization must not reduce detail accuracy.
- Background CI polling keeps its previous 25-pull-request coverage; the five-row UI enrichment budget must not reduce completion notifications.
- GitHub command failures and timeouts keep their existing typed error behavior.

## Unit Tests

- `list_prs_enrichment_is_bounded_for_interactive_loading` — the basic query requests 100 pull requests while the rich query requests exactly five.
- `polling_prs_keep_the_previous_notification_coverage` — the background polling query keeps rich CI state for 25 pull requests independently of the interactive list.
- Existing `github_surface` tests continue to cover rich-field parsing, graceful degradation, caching, and command timeout behavior.

## Integration / Functional Tests

- `cargo test -p bridge-core github_surface` passes.
- `bun run build` passes.
- `bun run test` passes.

## Smoke Tests

- Read the GitHub surface for the Bridge workspace through the running daemon and verify pull requests still load.
- Read the GitHub surface for the larger `rimo-backend` workspace and verify the cold pull-request request completes materially faster than the 8.64-second baseline.

## E2E Tests

- Open a repository-backed chat in Bridge, open the GitHub dock pane, and verify the pull-request list becomes interactive without a long frozen-looking wait.

## Manual / cURL Tests

- N/A — Bridge is a Tauri desktop app and this path uses the local `gh` CLI through the Bridge daemon, not HTTP or cURL.
