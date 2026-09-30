# Browser clone stack: landing review

Verdict: **deterministic checks passed; merge authorized by the user**.

The user explicitly requested landing after deterministic checks and will perform
interactive acceptance testing. The packaged agent journey was not completed and
is not claimed as verified.

## Reviewed stack

The cumulative code at `a12526d92f55a85c6ae7644487602db3b52e32a4` includes
754 (process), 755 (guard), 756 (sign-in/tool), 757 (dock/settings), 759
(orchestration), 760 (wire/trigger), 764 (agent request) and 765 (takeover plus
landing fixes). The final fixes are in the stack tip.

## Resolved findings

- Consent binds an immutable request ID, domain, additional hosts, extension path
  and live requester PID. Conflicting requests and stale answers fail closed.
- The requesting capability can poll for a terminal approval answer and receive
  the browser tool in the same turn. Denial and startup failure also terminate
  the wait.
- An always-mounted request inbox polls independently of the dock, including
  unvisited panes and background chats.
- Normal accepted turn completion destroys the clone and both capabilities.
  Cancellation, startup failure, session teardown and expiry also release it.
- Settings persist through the native protocol. The approval card submits the
  displayed sign-in choice and lifetime; sign-in inside is the default.
- Raw images stay in the human dock and are accurately labelled. Agent screenshot
  commands are refused. Accessibility editable values and registered secrets
  are scrubbed, and all page reads/writes pause during sign-in and takeover.
  Paused status exposes no blocked-host metadata; active metadata is scrubbed.
- An explicitly displayed extension directory is loaded only after approval.
  Additional CDN/sign-in hosts likewise require visible, immutable consent.
- Failed startup and overlapping starts have cleanup coverage. One supervisor
  owns the clones, listeners release their owners, and stale expiry cannot
  destroy a replacement clone.
- UI polling rejects stale responses after chat switching and clears sensitive
  drafts. Async input mocks match the real API without suppressing errors.

## Executed deterministic validation

- `bun run build`: passed.
- `bun run check`: passed.
- `bun run test`: exit 0. Frontend: 229 files / 2,878 tests passed. Rust core:
  2,763 passed, zero failures, 11 standard ignored tests. Workspace integration
  suites, protocol/artifact checks and doctests passed. Release scripts,
  sidecars and native menu checks passed as part of the command.
- Focused clone run: 44 matching Rust tests passed. Focused UI: 27 passed.
- Six environment-gated real-browser orchestration scenarios passed on Chrome
  (8.45 s) and Brave (6.91 s). These used isolated profiles, synthetic data and
  sign-in-inside, without reading existing browser profiles or Keychain.
- The application-level synthetic regression exercises real native methods,
  generated request/drive wrappers, immutable consent, same-turn handoff,
  extension loading, takeover, secret scrubbing and normal completion cleanup,
  twice in the same chat. This supports the application path but is not a
  substitute for interactive acceptance.
- An ad-hoc debug packaged application passed authenticated health RPC and
  clean app/daemon shutdown. This is a packaging smoke test, not a completed
  user journey or a release signing/notarization test.
- GitHub checks on the exact tested code head passed, including frontend,
  Rust, sidecars, managed-runtime compatibility, macOS packaged app smoke and
  Linux/Arch release candidates. Earlier dependency heads also have green CI.
- `git diff --check`: passed.

Node 26 validation used `NODE_OPTIONS=--no-experimental-webstorage` to avoid
conflicting with jsdom's storage fixture. Routine tests left `BRIDGE_CLONE_LIVE`
unset. Final tests used an isolated copy-on-write build cache; its copied Swift
module cache was reset after an initial path mismatch. The complete rerun passed.

Local evidence is retained under the ignored
`.generated/clone-review-20260930/` directory: `build.log`, `check.log`,
`full-test-final.log`, `clone-tests-final.log`, `clone-ui-tests.log`,
`live-chrome.log`, `live-brave.log`, `packaged-smoke.log` and
`github-final-checks.json`.

## User acceptance still pending

From the merged app, ask for a browser, verify the global approval card, choose
sign-in inside, take over, hand back and observe the requesting agent continue.
Verify normal completion destroys it; repeat in the same chat and in concurrent
chats. Test an approved local extension and any explicitly requested dependency
hosts. No completed interactive journey or screen recording is claimed here.
