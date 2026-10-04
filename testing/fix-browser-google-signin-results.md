# Google browser sign-in validation

## Research

Chromium's [runtime feature initialization](https://chromium.googlesource.com/chromium/src/+/refs/heads/main/content/child/runtime_features.cc)
enables AutomationControlled for `--remote-debugging-pipe`, as well as headless
and automation switches. Explicit Blink feature disabling runs afterward. This
explains why Bridge's headed, pipe-controlled browser reports webdriver=true
even though Bridge already omits `--enable-automation`.

[Google's supported-browser guidance](https://support.google.com/accounts/answer/7675428?hl=en)
states that software-controlled browsers can be refused. Clearing this one
browser signal does not establish that Google will accept every account login.

[Chrome's debugging guidance](https://developer.chrome.com/blog/remote-debugging-port)
requires an isolated profile for current Chrome debugging. Bridge retains its
RAM-disk profile and private debugging pipe, with its existing destruction and
explicit domain-consent rules.

## Live evidence on 2026-10-03

- Before the patch, the approved browser returned webdriver=true and
  headless=false. It exited during navigation, so that run did not establish
  the outcome of a Google account challenge.
- On installed Chrome/154.0.8037.95, the patched, headed, guarded browser returned
  webdriver=false before and after localhost navigation. Its process flags
  contained no headless, automation, test-type, or TCP debugging switch. Destroy
  removed the process and RAM-disk mount.
- The Google rejection predicate ran inside real Chrome against synthetic
  inputs. It recognized Google's English insecure-browser message, including
  case and whitespace differences, and rejected unrelated and spoofed hosts.
- A temporary probe built against the patched production CloneOrchestrator
  opened a blank RAM-disk profile with the explicitly approved Google domain
  list. No real cookie store was imported. Navigating to the Web Store developer
  console reached accounts.google.com with an email/password login form,
  browserRejected=false, and webdriver=false. Takeover status returned the new
  readable Hand back instructions. The probe and its RAM profile were destroyed.

**Completed Google account authentication remains unverified.** Reaching the
initial sign-in form does not test the later account challenge or 2FA. The live
probe exercised patched native browser code; the full patched desktop app's
authenticated Google journey still requires a person to complete sign-in.

## Coverage

- Launch invariants and explicit headless test opt-in.
- Generated helper requests, paused responses, zero page access during takeover,
  cached-result removal, and restoration of previously approved actions.
- Fixed Google recovery notices through tool responses and native clone-state
  serialization, with notices disappearing when the rejection disappears.
- Optional native notice mapping and visible Browser pane recovery controls.
- Existing cookie consent, shared sessions, guard, replacement, and lifecycle
  checks.

## Reproducing the account check

1. Run the patched desktop app with `bun run tauri dev`.
2. Request chrome.google.com and explicitly approve chromewebstore.google.com,
   google.com, accounts.google.com, googleusercontent.com, and gstatic.com.
3. Navigate to https://chrome.google.com/webstore/devconsole. Complete account
   interaction yourself using Take over, then choose Hand back.
4. Record whether the authenticated developer console opens or Google shows its
   insecure-browser rejection. On rejection, the Browser pane and tool results
   explain signing in in normal Chrome and approving a fresh google.com-scoped
   browser request.

The dedicated live launch and rejection tests use `BRIDGE_CLONE_LIVE=1` and never
import a real user cookie store. On Node 26.3.0, run the test script with
`NODE_OPTIONS=--no-experimental-webstorage bun run test` so jsdom supplies Web
Storage to the existing updater tests.

## Final checks on the current main base

The branch was rebased onto 7e65af72 before these final checks.

| Check | Result |
| --- | --- |
| bun run build | Pass |
| NODE_OPTIONS=--no-experimental-webstorage bun run test | Pass: 2999 frontend tests; 3250 Rust tests across the workspace; release, sidecar, and native menu checks also pass |
| cargo check --manifest-path src-tauri/Cargo.toml --workspace --locked | Pass |
| BRIDGE_CLONE_LIVE=1 cargo test --manifest-path src-tauri/Cargo.toml -p bridge-core clone_ -- --test-threads=1 | Pass: 62 focused checks, including all 12 live Chrome cases |
| Completed Google account authentication in the patched desktop app | Unverified; see the live evidence and account-check steps above |
