# fix/github-surface-scale - Test Contract

Bugs under fix: (1) repositories with roughly 20+ open pull requests fail to
load because a single `gh pr list --limit 100` requesting computed GraphQL
fields (`statusCheckRollup`, `mergeable`, `reviewDecision`) exceeds GitHub's
server-side query budget and returns 502/504; (2) a slow or hung `gh` call
wedges one of the shell's four daemon connections and blind round-robin then
queues unrelated invokes behind it for up to the 300-second client timeout,
which reads as an app-wide hang; (3) the pane refetches the entire PR list
every 8 seconds while checks run; (4) the changes tab renders every patch line
of every file eagerly, freezing the webview on large PRs.

## Functional Behavior

- `github/prs` loads the pull-request list on repositories of any size: the
  list itself is fetched with cheap fields only, and the expensive computed
  fields (review decision, mergeability, merge-state, check rollup) are an
  enrichment pass bounded to the newest 25 pull requests.
- When the enrichment pass fails (GitHub 502/504 or timeout), the list still
  returns: unenriched pull requests read review decision `none`, mergeability
  `unknown`, and an empty check rollup.
- Every `gh` invocation is bounded by a hard timeout; on expiry the child
  process is killed and the call fails with a `CommandFailed` error naming the
  timeout instead of blocking a daemon connection forever.
- Availability probing (`gh auth status`) is bounded by the same timeout.
- The desktop shell routes `github` domain invokes over dedicated daemon
  connections; non-GitHub invokes never share a connection with a GitHub call,
  so a slow `gh` read cannot stall sessions, health, or the work board.
- Within a partition the shell prefers an idle connection over blind
  round-robin, falling back to rotation only when every lane is busy.
- While a selected PR has queued or in-progress checks, the 8-second poll
  refreshes only that PR's checks; the full list refetch happens only on
  `github/checks_changed` and `github/ci_finished` events (server-side change
  detection), not on the timer.
- The changes tab renders file patches collapsed when the PR touches more than
  6 files; each file expands on demand. Oversized patches render a bounded
  number of lines with a link out to GitHub for the remainder.

## Unit Tests

- `list_prs` issues a cheap-field list call plus a bounded rich-field call and
  merges rollups by PR number (fake `gh` served both shapes).
- `list_prs` returns the full list with default review/merge/check state when
  the rich-field call exits nonzero.
- A raw PR summary without the expensive fields deserializes to defaults
  (review decision none, mergeability unknown, empty rollup).
- `run_gh` kills a hung `gh` and returns `CommandFailed` naming the timeout
  (fake `gh` sleeping beyond a test-shortened deadline).
- The proxy pins `github` domain methods to the reserved lanes and keeps other
  domains off them (selection logic asserted without sockets).
- Idle-preference: with one lane busy, the next call lands on an idle lane.
- Pane: the checks-poll interval invokes `githubChecks` only (no
  `githubPullRequests` on the timer tick).
- Pane: patches are collapsed above the file threshold and expand per file;
  a patch beyond the line cap renders the cap with an outbound link.

## Integration / Functional Tests

- Existing github surface, dispatch, and daemon routing suites stay green with
  the two-call list shape (fixture invocation counts updated deliberately).

## Smoke Tests

- `bun run check`, `bun run test`, and `bun run build` complete successfully.

## E2E Tests

N/A - deterministic Rust and jsdom tests cover the split fetch, the timeout,
and lane selection; the repository has no browser E2E harness for the native
GitHub surface.

## Manual / cURL Tests

- Open the pane against a repository with 100+ open PRs (e.g. a large OSS
  repo): the list renders; check badges appear on the newest ~25.
- With the GitHub pane loading a large repository, switch panels: sessions and
  work board remain responsive.
- Open a PR with a very large diff: the changes tab stays interactive.
