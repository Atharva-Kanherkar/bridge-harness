# Browser clone stack: landing review

Verdict: **changes required; do not merge the stack yet**.

## Reviewed scope

| PR | Reviewed head | Scope | Disposition |
|---|---|---|---|
| 754 | 3eeb58f465a09e299173c8c379f885373fff16c2 | Process, RAM disk, crash recovery | Foundation tests pass; application acceptance depends on the later slices. |
| 755 | ef3b6d70bbf674a1060f4d18c21c914dcc22b9ae | Request scanner and proxy | Synthetic checks pass. Network containment and real-site dependencies still need application proof. |
| 756 | d351d542163665701f9c0420ecccc3fdac0dff0f | Sign-in import and agent tool | Listener ownership and revocation fixed in the stack tip. Image confidentiality remains unresolved. |
| 757 | de6af3c79f860a2e55de2f5901c07b2d5ba61586 | Dock and settings | Production settings remain disconnected. Mock screenshots establish layout only. |
| 759 | 421a8b3184979f0abd930cb0342985a22169cf7e | Orchestration | Startup cleanup, supervisor ownership and overlapping starts fixed in the stack tip. Extension loading remains an internal method. |
| 760 | 437a612077877c10239c99ac716e1a37f700de4f | Wire methods and manual start | API compiles. The failed frontend CI run passed on rerun; this is not an application journey proof. |
| 764 | 981079672de87f96c5e672f62d95f00664b3cf21 | Agent request and approval | Approval delivery and consent binding remain blockers. |
| 765 | 939b2f1, plus landing fixes | Takeover and expiry | Cleanup, chat switching and CI test error fixed. Normal task completion is still missing. |

## Confirmed remaining blockers

1. **Approval does not deliver the new capability to the requesting agent.**
   `src-tauri/bridge-core/src/live_turn.rs:10745` injects clone context during
   input delivery. `api::resolve_clone_request` only builds and returns a
   snapshot. The request tool returns an awaiting-approval response, with no
   continuation, status capability, or notification delivering the drive tool.
   A direct orchestrator test explicitly calls the tool afterwards and does not
   prove this application handoff.

2. **The answer is not bound to the request the person saw.**
   `clone_browser_tool::execute` overwrites the pending domain. The wire answer
   contains only session ID and a boolean; `approve_request` consumes whichever
   domain is pending at that moment. Add an immutable request identity and
   validate it atomically against the displayed request and requesting runtime.

3. **Fresh chats do not automatically surface approval.**
   `src/components/SessionDock.tsx:224` mounts visited panes only. Clone request
   polling and attention reporting live inside `CloneSurface`. There is no
   independent request event/notification for an unvisited clone pane or other
   chats. The approval lifecycle must be visible without prior dock navigation.

4. **Normal task completion does not destroy the clone.**
   Teardown is wired to clear, cancellation and session stop, and to the lease.
   The normal `turn_completed` path does not release the browser, and the agent
   command surface has no completion/release operation. Define task ownership
   and prove completion cleanup without prematurely discarding a pending
   approval or human takeover.

5. **The production settings and sign-in choice are unfinished.**
   `src/api.ts:1446` returns `connected: false` under Tauri and rejects saves.
   Approval hardcodes Chrome, import and the default lease. The default
   sign-in-inside choice therefore cannot govern an agent-approved clone.

6. **Frames are advertised as redacted without image redaction.**
   `CloneOrchestrator::frame` directly returns `Page.captureScreenshot` data.
   Scrubbing JSON strings cannot mask sensitive pixels in PNG data. The wire
   reports zero redacted regions while the UI calls the image redacted. Agent
   reads also remain enabled during takeover. Implement and test the required
   confidentiality boundary before claiming redaction or complete pausing.

7. **The extension-testing journey is not exposed.**
   `load_extension` is an internal orchestration method. Neither the agent's
   restricted command surface nor the clone wire methods expose an approved
   extension-loading flow. The test calls that internal method directly.

8. **Real application evidence is missing.**
   Existing live tests operate directly on the supervisor/orchestrator and
   explicitly invoke the later steps. They do not demonstrate a running agent
   receiving approval and completing the requested task through the packaged
   app. No new authenticated account or existing browser profile was accessed
   in this review.

## Fixes included at the stack tip

- Async input mocks now match the production API; the CI unhandled exception
  is fixed without suppressing errors.
- Polling follows the selected chat and rejects stale reads/actions after a
  chat switch. Sensitive drafts reset on switching chats.
- Approval rejects a missing runtime and binds the tool to the real runtime
  PID instead of zero. Immutable request consent remains a separate blocker.
- One supervisor owns the live clones and crash ledger.
- Failed startup cleans up before a clone enters the active map.
- Pending requests, wrappers and cached results are revoked on destroy, even
  when a browser was never created.
- The listener no longer retains its owner forever; dropping the orchestrator
  also destroys its active clones.
- Per-chat operations serialize overlapping starts, teardown, takeover and
  page actions, while separate chats retain separate locks.
- The expiry sweep checks the captured clone identity, so a stale expiry cannot
  destroy a replacement clone.

## Validation limits

`BRIDGE_CLONE_LIVE` is unset. Environment-gated live tests returning early are
not fresh live-browser evidence. The new lifecycle tests use a synthetic pipe
child and temporary directory; they never import real sessions.

Node 26's experimental global storage conflicts with the Vitest/jsdom storage
fixture. `NODE_OPTIONS=--no-experimental-webstorage` restores the intended test
environment; all four updater tests pass without modifying or skipping them.

The shared build cache was changed by concurrent builds during one doctest run.
Final validation uses a separate copy-on-write cache to avoid artifact races.
See the PR's validation section for final command results.

## Final executed results

- `bun run build`: passed.
- `bun run test`: exit 0 with the environment and isolated cache described above.
- Frontend: 228 files and 2,874 tests passed; zero failures or unhandled errors.
- Core: 2,759 passed, zero failures, 11 standard ignored tests.
- Other workspace test binaries, integration suites and doctests: passed.
- Release-script, sidecar and native menu checks: passed as part of the full command.
- The focused final lifecycle run passed 40 matching tests, including the synthetic cleanup and overlapping-start regressions.

Passing these checks does not satisfy the unresolved application acceptance criteria above.
