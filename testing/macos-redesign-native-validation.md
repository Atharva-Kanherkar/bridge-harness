# macOS redesign integration validation

Date: September 7, 2026

The macOS frontend redesign was integrated with `origin/main` at `e39e40c`, producing application revision `775e6b6`. The screen-by-screen design and browser walkthrough are recorded in [the screen audit](../docs/macos-screen-audit.md). This report adds build, native backend, and live provider evidence; it does not treat a browser mock as a working provider.

## Automated checks

| Check | Result |
| --- | --- |
| `bun run build` | Passed TypeScript and production Vite build. Existing large-chunk advisory remains. |
| `bun run test` | Passed: 1,818 frontend tests across 136 files; 2,189 Rust tests, 13 ignored; 44 sidecar tests, 1 skipped. |
| `git diff --check` | Passed. |
| Native debug app bundle | Tauri build passed with the current browser host, daemon, and packaged Claude SDK resources. |

The complete test command ran with Vitest worker bounds of one to four to avoid unnecessary host contention. Assertions and test timeouts were not weakened. The integration merge required no conflict resolutions or application source changes.

## Live native backend checks

The freshly bundled `bridged` daemon used the normal application data directory and its Unix socket protocol. The existing app was closed before replacing the running daemon, and the application databases were backed up. Two disposable Git repositories were connected as QA workspaces. Each started with three failing Bun tests; providers were instructed to change only the source file, leave tests unchanged, and avoid dependency installs, commits, or delegation.

| Scenario | Observed result |
| --- | --- |
| Cursor Auto coding task | Read the slug implementation/tests, edited the implementation, requested permission for `bun test`, and returned a successful three-test result with a TypeScript example and `CURSOR_NATIVE_QA_COMPLETE`. |
| Input while Cursor was working | `submit_input` returned `queuedForPhaseBoundary`. The follow-up was answered after the coding task, listed the three passing tests, and included `CURSOR_FOLLOW_UP_RECEIVED`. |
| Approval lifecycle | The exact fixture test commands received Allow once. Permission resolving, resolved, and settled events were observed, followed by completed tool output. No persistent allow rule was added. |
| Claude Sonnet live request | The runtime's Sonnet model was selected, but authentication failed because its OAuth session had expired and could not refresh. This is a failed live-provider check, not a Claude success. |
| Recovery after provider failure | Switched the same failed chat to Cursor Auto. Cursor fixed the mean implementation, ran three passing tests, and returned the requested Markdown table, TypeScript example, and `CURSOR_RECOVERY_QA_COMPLETE`. |
| Stop and restart | Submitted a harmless 30-second Bun wait, allowed that exact command once, then interrupted the active turn. Session state changed from working with an active turn to ready with no active turn. A new input started a new turn and returned `STOP_RECOVERY_OK`. |
| Independent verification | Reran both fixture suites: six tests passed. Git status in each fixture showed only its intended source file modified; the tests remained unchanged. Both QA sessions ended ready with no active turn. |

These checks exercise the actual provider adapters and the native methods used by the frontend. They do not claim that the corresponding native UI buttons were clicked: the Mac locked during validation, and the app-control tool could not unlock it.

## Release restart regression

The first release bundle built successfully, was locally signed with the existing app entitlements, passed strict bundle signature verification, and was installed with the previous app preserved. Its daemon reloaded all QA response markers and produced `INSTALLED_RELEASE_QA_OK`, correctly recalling the earlier slug test result.

That restart also exposed a pre-existing shutdown defect: `Daemon::shutdown` stopped live providers without clearing their durable process records. On the next launch, the supervisor classified even ready chats as failed provider orphans. Sending a new message recovered the chat, but the failure state was misleading after an orderly quit.

The shutdown path now records provider teardown transactionally: clear the tracked process, stop unfinished sessions and clear their active turn, append the `app_shutdown` event, and reconcile workspace state. Completed, cancelled, stopped, and failed results are preserved. Model replacement keeps its existing continuation path. If persistence fails, the error is reported and the transaction rolls back so startup recovery remains available.

A daemon restart regression reproduced the bug before the fix (ready became failed), then passed after it. It covers ready, working, waiting, completed, and failed chats, verifies that tracking and active-turn fields are cleared, and rejects false orphan events after restart.

The final production frontend build and full suite passed after this fix: **1,818 frontend tests, 2,190 Rust tests (13 ignored), and 44 sidecar tests (1 skipped)**. An earlier rerun exhausted disk space while creating a Rust archive; generated package artifacts were cleared, including an unusable cached CLI executable, before the successful complete rerun. No test assertions were removed or weakened.

## External limits

- Native visual inspection of the rebuilt app remains pending an unlocked desktop. Earlier Paper/Graphite and resize walkthroughs used the browser preview. Native VoiceOver, traffic lights, wallpaper vibrancy, and real browser-host/terminal UI operation are not verified by this run.
- Claude requires account reauthentication before its successful response path can be tested. Cursor Auto supplied the successful live coding, queue, and recovery coverage.
- [GitHub Actions run 34104816275](https://github.com/Atharva-Kanherkar/bridge-harness/actions/runs/34104816275) could not start its jobs. GitHub reported failed recent account payments or an exhausted spending limit. No CI job steps ran; local checks above are the available build/test evidence.
- There were no previously open pull requests at the start of integration or at the final inventory before publishing this change. Their changes were already on `main` and are included in the integration base.

Private application state, raw provider events, credentials, and disposable fixture repositories are not included in this commit.
