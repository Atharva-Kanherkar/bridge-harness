# Many chats at once: test contract

With ten or more chats running, chats stalled, streams stopped mid-reply and
the app stopped responding. Sampling the running app and reading the live
data directory (8 GB `bridge.db`, 87,686 sessions) traced it to work done
under the core's one database lock, an app state of 59 MB rebuilt on every
change, and full-database copies every 15 minutes. This contract fixes what
each change must keep and what it must prove.

## Functional behavior and tests

- **Repository stamps leave the lock.** A stored entry's `_bridgeRepoState`
  comes from a per-repository cache (`repo_stamp`). Streaming entries
  (command output, tool starts, status) run no `git` under the lock while the
  stamp is younger than 5 s. Messages and turn ends (`exact_kind`) reuse a
  stamp only when it is younger than 1 s and no tool completion
  (`changes_folder`) or editor save happened in that repository since. The
  frame reader refreshes a stale or changed stamp before it takes the lock.
  Tests: `repo_stamp::tests` (no git under the lock mid-turn; a tool
  completion makes the next reply dirty with no git under the lock when the
  reader refreshed; without a refresh the reply measures; editor saves
  invalidate; chats outside a repository run no git) and the existing
  clean/dirty/diverged test in `bridge-deck`, now with the edit reported by a
  tool completion.
- **The ledger is indexed.** Migration 91 adds `events(kind, entity_id)`.
  Test: the worker pool's terminal-outcome query plan uses the index and no
  longer scans `events`.
- **The app state leaves out background runs.** `store::state` drops the
  hidden kinds (`HIDDEN_KINDS_SQL`: briefing, suggestion, extraction,
  outcome_evaluation, consolidation, chat_search, connector) and keeps every
  kind a person can open, workflow runs included. `state/get_state` answers
  with every session, as before. Test: both reads over a fixture with every
  kind.
- **Starting a chat does not build a state nobody reads.** A resume for a
  send launches through `launch_chat`; `start_chat` builds the state after
  the launch, for its caller.
- **The workspace poll returns the workspace.** `workspaces/refresh_workspace_stats`
  writes only when a count or the branch moved and returns that row, or none
  when the workspace was archived meanwhile. The app patches its workspace
  list and keeps the same state object when nothing moved. Tests: workspaces
  core test, protocol mirror, `workspaceStats.test.ts`.
- **State reloads rest.** `createCoalescedRefresh` takes `maxPauseMs`: after
  a read, the next waits as long as that read took, up to 750 ms for the
  app's `state-changed` reload. Tests: a burst during a slow read is answered
  by one read after the rest; a quick read rests only as long as it took; an
  instant read never rests. The App suites that save settings and expect the
  next reload at once pass unchanged.
- **Snapshots scale with size.** The newest history snapshot may be 15 min
  old per GiB started, at most 6 h. Test: interval for 0 B, 200 MB, 1 GiB,
  the measured 7.9 GB database (2 h) and 500 GiB (6 h).
- **Hot dependencies are optimized in dev builds**: SQLite, `rusqlite`,
  `serde_json`, `sha2`.

## Must not change

- `state/get_state` answers exactly as before. Results that carry the app
  state (`start_chat`, `refresh_workspace` and the other mutations) leave out
  the background runs too, since only the app reads them and it never shows
  them. Protocol 1.51 adds methods; a 1.50 client still pairs.
- The sidebar, chat, Mission Control and every surface that opens a session
  see the same sessions (they already filtered out the hidden kinds).
- Exact repository state reads (`repository_state_for_path`, completion
  checks, worker adoption, forest reads) still measure every time.

## Proof

`perf_bench` (ignored tests in `bridge-core`) on a clone of the live
database's latest history snapshot and four scratch clones of this
repository, before (clean `origin/main` plus the bench) and after, on the
same machine under the same load:

- B1 app state build and serialize, ms and MB.
- B2 worker pool terminal pass, without and with the index.
- B3 twelve chats storing entries in git repositories concurrently: lock
  held and lock wait per entry, wall time, git processes.
- B4 the workspace poll, whole state vs one workspace.
- B5 SHA-256 throughput as a history snapshot pays it.

## Verification

- Regenerate protocol artifacts; protocol and command-surface drift tests pass.
- `bun run build` and `bun run test` green before the PR.
