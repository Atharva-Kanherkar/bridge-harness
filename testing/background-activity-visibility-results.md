# Background activity visibility: investigation and verification

The supplied session showed a detached test launcher and a Monitor acknowledgement,
then assistant replies while the tests continued. Both the launcher and the SDK
background task needed their own truthful state.

## Root causes

- The Claude sidecar already forwards SDK messages after `result`. Its Rust
  normalizer discarded task starts, updates, progress, membership snapshots,
  terminal notifications, tool heartbeats, and typed synthetic task notices.
- A background launch acknowledgement became `completed`, which could look like
  a passed test before the real process finished.
- The UI scoped every running indicator to the current assistant turn and had no
  persistent process list. Older calls could revive during a later turn, while a
  silent background process could fall outside loaded history.
- Transient heartbeats can have event id zero and no output text. Row memoization
  did not see duration or progress changes in those frames.
- Foreground checkpointing suppressed asynchronous process updates along with
  maintenance tool calls, so an existing process could lose its completion.
- A shell that uses `nohup`, output redirection and `&` can outlive its launcher
  without exposing a process lifecycle to Bridge. Native task progress is also
  different from a live stdout stream. Bridge cannot manufacture missing output.

## Changes

Native tasks and executions now retain their own lifecycle. The snapshot includes
up to 256 active process entries independently of the history window, with output
previews capped at 8 KiB per field. Runtime stop and startup recovery record an
unknown outcome for unfinished tasks instead of success. Corrupt history remains
readable. The transcript uses one current-activity panel across all live chat
surfaces, with separate model, running-process, preparing-call and passive-watcher
states. Launchers and their tasks count as one logical command. The shared host
prompt requests tracked execution, observable logs and verified exit status.
Newer live outcomes, output and heartbeats update a process before the next
snapshot arrives. Cleanup participates in its caller's lifecycle transaction,
including rollback. Existing processes remain observable during checkpointing;
maintenance tool calls stay hidden. A task becoming hidden closes any previous
visible running claim without inventing a successful outcome.

The SDK wire shapes were checked against the installed pinned TypeScript types
and the [official SDK reference](https://code.claude.com/docs/en/agent-sdk/typescript).
The browser checks use provider-neutral event fixtures rendered through the real
conversation component; they do not claim to run four live provider accounts.

## Verification

- The new transcript regressions failed before the fixes: background work lost its
  running state after a reply, and parallel work named the last completed call.
- 12 browser checks passed: four harness labels, model/background distinction,
  completion/failure/unknown outcomes, offscreen origin, mobile width, absent
  output and available streamed output.
- SDK sidecar forwarding, native lifecycle, persistence, history-window recovery,
  restart recovery, corrupt history and prompt compilation have regression tests.
- Review added regressions for transaction rollback, compaction with concurrent
  work, process exits during checkpointing, stale snapshots, transient output,
  resumed tasks and tasks whose visibility changes before they finish.
- The activity query was checked with `EXPLAIN QUERY PLAN`: latest processes
  drive indexed `(session_id, sequence)` lookups. A 200-row reproduction improved
  from about 201 ms to 2 ms after removing a repeated coroutine scan. The existing
  148,123,560-byte pathological-history fixture passed in 16.8 seconds and
  produced a 19,152,682-byte snapshot (7.7 times smaller).
- Final build and full-suite results are recorded after completion below.

The test environment uses `NODE_OPTIONS=--no-experimental-webstorage` so Node 26
leaves jsdom in charge of localStorage. Rust artifacts reuse the main checkout's
ignored target cache through `CARGO_TARGET_DIR`; user data is not used by tests.

[Background process while the model is idle](../docs/design/background-activity/background-process.png)

[Available output on a narrow viewport](../docs/design/background-activity/mobile-output.png)

## Reproduce the browser checks

Run `bunx vite --host 127.0.0.1 --port 1432` from the task checkout and open
`http://127.0.0.1:1432/testing/fixtures/background-activity.html` in a fresh browser.
The fixture uses the actual conversation component and theme. In the console call
`showActivity(mode, harness)`, where mode is `running`, `background`, `completed`,
`failed`, `unknown`, `many`, or `stream`. Harness may be `claude`, `codex`,
`opencode`, or `cursor`. For `many`, scroll the original command out of view; the
current-activity panel must stay visible. At 420 pixels, verify no horizontal
page overflow. Expand the process to inspect the supplied output or its explicit
absence. The model mark appears only while the model is working.

## Final results

- `bun run build`: passed.
- `NODE_OPTIONS=--no-experimental-webstorage bun run test`: passed with the
  shared Cargo target cache, including all 226 frontend files / 2,856 tests and
  the Rust workspace's unit, integration and protocol checks.
- The final task-resumption correction was followed by all 9 native task and
  Monitor tests: passed. Resuming a task clears the previous run's report.
- Focused transcript, rendering and provider-independence checks: 209 passed.
- 12 fresh-browser fixture checks: passed.
- `git diff --check`: passed.

One concurrent full-suite attempt hit the existing short-timeout helper test
under load. The final full run completed without that failure. No helper behavior
or timeout was changed.

The changes are on `fix/background-activity-visibility` in the isolated task
checkout. The user's original working tree was left intact; the running desktop
application was not relaunched or replaced.
