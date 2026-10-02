# fix/browser-google-signin: test contract

Locked before implementation against the reported Google browser rejection and
the existing throwaway-profile, approved-domain, and takeover contracts.

## Functional Behavior

- Production clones use installed, headed Chrome with their existing isolated
  RAM-disk profile and debugging pipe. Chromium's pipe-enabled automation bit
  must not make `navigator.webdriver` true. No `--enable-automation`, test-type
  switch, headless user-agent override, or JavaScript navigator spoofing is added.
- Headless mode remains an explicit test option. Profile destruction, private
  debugging transport, domain consent, and cookie import scope remain intact.
- A Google page showing "This browser or app may not be secure" produces a fixed,
  readable sign-in notice in browser tool responses and the Browser pane. Match
  only accounts.google.com; unrelated pages with the same text do not qualify.
- The notice explains user sign-in, Take over, Hand back, and the fallback of
  signing in in normal Chrome then approving a fresh browser with google.com
  explicitly included. Never promise that Google will accept an automated login.
- Paused status and refused page calls explain that the person must finish and
  choose Hand back, and that status can be polled. Paused agent calls must never
  inspect the page, expose screenshots, or retrieve cached results.
- Hand back clears paused status and restores only previously approved actions.
  Tool instructions must describe imported sessions as potentially challenged,
  rather than assert that cookie import guarantees authentication.

## Unit Tests

- browser_clone: launch flags preserve the pipe and temporary profile, keep
  production headed, and reject automation switches before spawning.
- clone_browser_tool: paused status/errors carry recovery instructions, perform
  no page access, and resume without stale sign-in notices or cached results.
- CloneSurface.test.tsx: Google recovery guidance appears while acting or taken
  over, ordinary waiting/takeover guidance and controls remain usable.
- The wire-to-UI mapping preserves an optional native sign-in notice.

## Integration / Functional Tests

- Use the generated helper against a synthetic guarded clone to verify paused
  JSON and hand-back behavior through the real HTTP boundary.
- Detect a synthetic Google rejection through CDP and return the same fixed
  message through tool status, page results, and the native snapshot source.
- Existing lifecycle, cookie consent, guard, takeover, and replacement tests pass.

## Smoke Tests

- bun run build and bun run test pass before opening the PR.
- cargo check --manifest-path src-tauri/Cargo.toml --workspace passes.

## E2E Tests

- BRIDGE_CLONE_LIVE=1: run real installed Chrome with the patched production
  launch flags; assert webdriver is falsy and the UA contains no HeadlessChrome
  before and after navigation. Verify Chrome process flags and RAM-disk cleanup.
- Exercise the Google rejection predicate in real Chrome with synthetic inputs,
  including whitespace, a spoofed hostname, an ordinary sign-in page, and the
  insecure-browser message. Never import a real user cookie store in tests.
- Live Google verification requires explicit google.com approval. Record the
  exact observed result on patched code separately from synthetic tests and from
  any check using the currently running app. If account challenges or human
  interaction prevent completing sign-in, document the limitation honestly.

## Manual / cURL Tests

- Request chrome.google.com with explicitly approved chromewebstore.google.com,
  google.com, accounts.google.com, googleusercontent.com, and gstatic.com.
  Navigate to https://chrome.google.com/webstore/devconsole and record whether
  Google rejects the patched browser. Authentication and 2FA belong to the user.
- On a rejection, inspect the Browser pane and call status: both explain the
  recovery path. Take over, then verify that agent calls are refused with readable
  Hand back instructions. Hand back and verify that approved calls resume.
- Review launch and recovery evidence in the PR; do not count reaching Google's
  initial login form as successful authentication.
