# Menu Bar implementation validation

Checked on 2026-09-10, on Apple silicon with macOS 26. The implementation is
isolated in `codex/native-menu-bar`, based on main at `384997b`. The Codex
quota-window fixes are committed as `13ee07e`.

## Automated checks

- The final full Rust workspace run passed: 2,603 tests, including two
  documentation tests; 13 intentionally ignored. The command was
  `cargo test --manifest-path src-tauri/Cargo.toml --workspace -j 1 -- --test-threads=4`.
  This includes daemon settings persistence/invalid-save rejection, shared
  usage snapshots, stale-daemon handshake rejection and desktop integration.
- Focused Rust checks passed for weekly-only/reversed quota windows, stale and
  unavailable values, real zero, pricing coverage and token aggregation. The
  Grok fake-CLI and large-database tests also pass in the final workspace run.
- The full frontend suite passed: 2,047 tests across 166 files. After automatic
  quota selection was added, all 32 focused settings/API/usage tests passed and
  the production TypeScript/Vite build passed.
- Swift checks passed for the shared Rust/Swift JSON fixture, automatic window
  selection, zero, stale and unpriced metrics, exact grouped token counts, cost
  qualifiers, reset countdowns, icon dimensions, `isTemplate`, transparent
  background and monochrome alpha-mask pixels.
- Release-script tests passed (16 Node and 12 Python); Claude sidecar tests
  passed (50 tests, one authenticated-runtime test skipped).
- The changed Rust files pass formatting checks; `git diff --check` passes.
  Workspace-wide formatting also reports existing differences outside this
  feature; those files were not reformatted.

Bun 1.3.13 exits with `CouldntReadCurrentDirectory` before either requested
script starts in this worktree. The equivalent build and test stages were run
with npm and Cargo. Earlier runs also encountered a shared Cargo artifact collision
with the active main checkout and then exhausted disk space. This worktree now
has its own Cargo target directory; obsolete build libraries were cleared there
before the rerun.

## Packaged Swift milestone

The installed preview is:

`/Users/yashaf/Applications/Bridge Menu Bar Preview.app`

It uses a distinct `dev.bridge.deck.menubar-preview` bundle ID and data
directory. Swift's `NSStatusItem` and `NSMenu` host the independent SwiftUI
presentation inside the Tauri process. The existing Rust backend supplies JSON
over the C ABI. The preview contains the current production frontend bundle.

It is Developer ID signed with hardened runtime. Strict/deep signature
verification passes, and the executable's Mach-O minimum OS is 12.0. Swift
compiles for arm64 with a macOS 12 deployment target; the same source also passed
an x86_64 macOS 12 availability/type check. These are not macOS 12 runtime tests.
There is no trusted timestamp or notarization. The integration preview omits
release sidecars/resources and is not a distributable Bridge release.

A separate daemon accepted the 1.8 handshake and menu preference save/read
calls. A read-only Codex refresh with a minimal GUI-like PATH returned account,
plan, reported limits, recorded tokens, model breakdown and estimated cost. It
created no chat or turn. The temporary daemon and database were removed after
the check.

## Interactive checks

The installed preview was launched and exercised through its native UI:

- Usage → Open usage meter opens the new native menu. Escape dismisses it.
- The menu shows the signed-in account and plan. The live Codex response places
  its only weekly quota in `primary`; the corrected menu labels it Weekly and
  leaves Session unavailable. Reset countdowns match the weekly duration.
- Used and remaining modes show the corresponding percentages. Automatic
  selection and Today's spend can be saved. Exact token counts use separators;
  the native model submenus expose input, output, cache and cost for Today and
  the last 30 days. Unpriced models and mixed totals remain unavailable.
- Refresh completes, and a transient provider failure keeps historical values
  stale instead of manufacturing zero. Account information recovers after a
  later successful refresh.
- Menu Bar Settings opens General → Menu Bar in the main app. The settings
  layout was inspected in light and dark appearances.
- Account, token/model and cost visibility changes affect the native menu. With
  both details disabled, only quota information remains.
- Visibility, provider, display and detail preferences survive an app restart.
  An explicit Open usage meter action still opens a hidden menu. When Codex is
  disabled, the menu explains how to enable it and disables Refresh.
- Test preferences were restored to visible, Codex enabled, remaining quota,
  automatic window selection, all details enabled and a five-minute refresh.
- No matching recent crash report was found for the installed preview.

Native behavior was checked through accessibility state. The settings surface
was also checked in screenshots; this capture path did not expose the native
menu's full visual appearance for a complete appearance audit.

## Remaining before retiring the fallback

1. Check the native menu in light/dark/highlighted appearances, increased
   contrast and reduced transparency, plus full keyboard/focus behavior and
   smaller displays.
2. Exercise an actual supported macOS 12 runtime.
3. Validate the complete release bundle through the normal signing and
   notarization pipeline before shipping it.

The native menu is the default macOS surface. The legacy meter remains
available internally through `BRIDGE_LEGACY_METER=1` until these parity gates
pass; its old shared presentation dependency has not been retired prematurely.
Claude, Cursor, and OpenCode were added in the provider expansion below.
Further providers, hooks, and user plugins remain separate follow-on work.


## Provider expansion · 2026-09-11

Implemented on `codex/menu-bar-providers` in `e15d4c4`, with backend-owned
OpenCode credentials and executable Swift runtime linkage corrected in
`86496c4`. The main conversation and usage-screen presentations are unchanged.
See [provider sources and boundaries](menu-bar-providers.md).

Verified automatically:

- Full core suite: 2,291 passed, 11 intentionally ignored. Two live managed
  agent integration tests remain ignored. Existing device usage imports and
  pricing continue to supply all four provider snapshots.
- Protocol: 161 tests and two documentation tests passed, including grouped
  Rust/Swift fixtures, old-preference migration, session redaction, artifact
  consistency, and rejection of a pre-1.9 daemon.
- Frontend: the full 2,047-test suite passed; after two provider-control tests
  were added, all nine focused settings/API tests and TypeScript checks passed.
- Final app and daemon suites passed after the linkage fix. They cover grouped
  snapshots, provider preferences across daemon restarts, invalid credential
  rejection without disclosure, and the first-party OpenCode login boundary.
- Native Swift: all four provider selections, fallback when disabling a
  provider, account amounts, missing/zero/stale data, reset countdowns, and
  template-icon checks passed. Both arm64 and x86_64 compile/type-check with
  a macOS 12 deployment target. This is not a macOS 12 runtime test.
- Release-script checks: 16 Node and 12 Python tests passed. Claude sidecar:
  50 passed, one authenticated-runtime test skipped.
- Production frontend and the release-mode packaged preview built successfully.

The first sandboxed Rust run could not bind test daemon sockets. The subsequent
unrestricted run passed the core/client suites and exposed a missing Swift
runtime search path in an executable test target. The app build now supplies
`/usr/lib/swift` to all its link targets; the final app/daemon rerun passed.
This is a build configuration change, not an environment-variable workaround.
Bun still fails before starting its scripts with `CouldntReadCurrentDirectory`;
validation uses the equivalent npm, Swift, and Cargo stages.

### Updated preview installed

The implementation worktree and completed preview bundle disappeared immediately
before signing/installing the update. Both implementation commits were already
saved in Git. Sources were restored into `/private/tmp/bridge-menu-bar-providers`.
After the user freed disk space, the production frontend and release-mode app
were rebuilt successfully from the restored source. The fresh bundle was signed
with the existing Developer ID and hardened runtime, then installed at:

`/Users/yashaf/Applications/Bridge Menu Bar Preview.app`

The old preview process was stopped before replacement. The installed bundle
passes strict/deep signature verification, and its executable matches the signed
build artifact byte for byte. The Mach-O minimum OS is 12.0 and its runtime search
path includes `/usr/lib/swift`. The executable SHA-256 is
`f9281c1965eab284d03ff57027404c1b6af12c1c1c8e50c4a6a89ed7b99a7d83`.

The installed preview also includes the app-logo correction: the native menu
uses the exact foreground rectangles from `assets/bridge-icon.svg`, preserving
the span and support proportions and omitting the rounded-square background.
It remains an 18-point monochrome alpha-mask template for macOS recoloring.
The existing native Swift checks passed, an enlarged native rendering was
visually checked against the app mark, and the rebuilt/installed bundle passed
strict/deep signing verification again.

### Provider interaction and menu tracking

During the unlocked session, the installed preview loaded and its native menu
and General → Menu Bar settings were exercised through accessibility:

- All four provider controls and native switcher segments are present. Claude,
  Cursor and OpenCode were enabled in the isolated preview.
- Codex returned its account/plan, weekly quota and reset, recorded tokens and
  model breakdown. Unpriced totals remained unavailable.
- Cursor returned its signed-in account, billing-cycle quota/reset, plan usage,
  allowance and on-demand amount. Local token history remained unavailable,
  explicitly labelled as not imported.
- Claude reported an expired Claude Code session requiring sign-in again.
- OpenCode reported that a Zen workspace connection is needed; absent local
  history stayed unavailable. No new sign-in or credential entry was performed.

The check exposed an open-menu switching bug: the selected segment could change
while the card still showed the preceding provider until menu dismissal. The
user's local CodexBar checkout was then reviewed at `928166f899471bbdcb72210641cdec91324d0154`;
its tracking/default-mode one-shot scheduler was adapted for Bridge's Rust-to-Swift
delivery. Model submenus now refresh for provider changes and read the latest
snapshot when opened. See the [architecture and adaptation notes](menu-bar-codexbar-study.md).

Review also caught that calling `cancelTracking()` on an open child menu would
end the parent menu session. Commit `9cdb247` preserves the tracked child and
defers its structural refresh until the next opening. A native regression
checks the retained menu/row identities, multiple queued provider changes,
and the latest provider and cost visibility when reopened. It passes together
with the existing Swift checks and the x86_64 macOS 12 type check.

The final native checks pass for tracking-mode delivery, default-mode fallback,
exactly-once execution in either order, and delivery from a worker through the
C ABI onto the main thread. Existing fixture, quota/formatting and icon checks
also pass. The source type-checks for x86_64 with a macOS 12 target. Production
frontend and release-mode packaged builds succeeded; the fixed preview was
Developer ID signed, installed, and passed strict/deep signature verification.
The installed executable matches the build artifact; its hash is recorded above.
The restored working source is now in `.worktrees/menu-bar-providers` because
the temporary worktree and caches were removed again.

After the user unlocked the Mac, a temporary one-hour `caffeinate -diu` assertion
kept it awake during the resumed verification. No persistent lock or security
settings were changed. The corrected installed preview passed these live checks:

- With the native menu continuously open, switching Codex → Claude → Cursor →
  OpenCode → Codex updated both the card and its model submenu to the selected
  provider. The previous provider's account and models did not remain visible.
- Opening Model & token breakdown after the switch exposed the selected
  provider's models and each model's exact input, output, cache and total tokens,
  plus its estimated or unavailable cost.
- The native settings action opened General → Menu Bar. All four provider
  switches were enabled, and provider selection agreed with the native menu.
- Cursor was deliberately saved before quitting the preview and installing the
  build containing `9cdb247`. After relaunch, Cursor was still selected and all
  four providers remained enabled. Switching providers and opening nested model
  details passed again in this final signed build. Codex was then restored as
  the selected provider, with all four providers still enabled.
- Connect OpenCode opened the dedicated window and reached the OpenAuth page
  at `auth.opencode.ai`, showing Continue with GitHub and Continue with Google.
  The login page and settings were inspected in screenshots. The login window
  was closed without entering credentials or creating an account.
- No Bridge-named crash report was present in the user's DiagnosticReports
  directory after the final installed-app pass.

The capture tool still does not return an image of the native menu or status
item, so the exact on-screen menu-bar icon appearance is not visually verified.
The native rendered alpha mask was checked separately as described above.
OpenCode's complete sign-in round trip and a current Claude account read still
require authenticated sessions. The normal release signing, notarization, and
macOS 12 runtime gates from the original milestone still apply.

## Native sizing and accessibility · 2026-09-11

A GPT-5.6 Sol research subagent compared the relevant CodexBar implementations
before this slice was chosen. The concrete sources and adaptations are recorded
in [the CodexBar study](menu-bar-codexbar-study.md). Commit `ef7006a` fixes the
viewport retaining its opening height when a tracked card grows or shrinks.
The document and native row now resize together; same-provider updates preserve
and clamp scroll position, and provider changes return to the top. Menus retain
the exact effective appearance, including accessibility attributes. The hosted
row has no fallback title or parallel native highlight. A shared status payload
now supplies visible text, tooltip, and the provider/window/value accessibility
title with explicit stale and unavailable wording.

Validation for this slice:

- Native tests pass for loading → full data → loading using the production
  `MenuState` and `NSHostingView<MenuCard>`, including immediate measurement in
  the event-tracking run loop. Geometry tests cover viewport caps, short/tall
  transitions, scroll clamping, provider resets, and both coordinate directions.
- The exact Aqua, Dark Aqua, and high-contrast appearance objects propagate to
  root and child menus. Status tests cover used/remaining, zero, unavailable,
  stale, estimated cost, and changing the provider. Expired account auth does
  not taint fresh local cost; metric-level stale cost remains labelled.
- Existing wire, quota, countdown, tracking, submenu and icon tests still pass.
  Native source type-checks for x86_64 with a macOS 12 target. The release-mode
  packaged preview built, was Developer ID signed and installed, passed
  strict/deep signature verification, and matches the build binary hash above.
- The installed preview relaunched and switched from the tall Codex card to
  the short OpenCode card and back without dismissing the native menu. Provider
  content and breakdowns matched the selection throughout. Runtime geometry is
  covered by the native tests because the menu capture limitation remains.

The native test script optionally renders synthetic production cards using
CodexBar's `MenuLayoutScreenshotRenderTests` offscreen-window pattern:

```sh
BRIDGE_MENU_BAR_RENDER_DIR=/private/tmp/bridge-menu-card-proof sh scripts/test-menu-bar-native.sh
```

The window is never ordered onscreen, no provider or Keychain is read, and no
system appearance preference is changed. The output covers light, dark and
both high-contrast appearances, with full-height and 280-point capped viewports.
These renders exercise the actual SwiftUI card and AppKit scroll view on a plain
window background. The 12 full, capped-top and capped-bottom images were generated;
light/dark and high-contrast cards were visually inspected with readable text,
no overlaps, and the final coverage row reachable in the 280-point viewport.
They do not represent native menu chrome, translucent glass,
status-item highlighting, VoiceOver speech, or keyboard tracking. Those live
appearance/input checks and the prior release/macOS 12 gates remain open.

## Overview and customization · 2026-09-11

The quota/provider contract and persisted customization settings are committed
as `65131a2`. Native Overview is quota-only; provider details include sparse daily
history and all recorded model rows. Tests cover independent bar direction,
full consumption, fractional percentages near zero/100, unknown and stale
values, Codex duration labels, named quota pools, Cursor's three lanes, token
layout bounds, two-line template rendering and tracked menu lifetime.

The production TypeScript/Vite build passes. The full frontend run passed 2,053
tests in 167 files before the last settings regressions; all 11 focused layout
and settings tests pass after those additions. The full Rust core run passed
2,297 tests with 11 ignored. All 19 daemon integration tests pass after replacing
the obsolete assertion that missing quota creates a Session row. Protocol checks
pass with the versioned interactive-refresh method and source provenance. Swift
checks and the x86_64 macOS 12 type check pass, including the two-line status
image's 22-point height bound, native template tinting, nested menu tracking,
daily selection resizing and rapid serialized preference saves.

Synthetic production-card renders cover Overview, Codex and Cursor in light,
dark and both high-contrast appearances, including capped viewports. Overview,
Codex details and Cursor details were visually inspected; all five tabs fit and
Cursor's 100% used lane is full. These are offscreen fixture renders, not live
account screenshots or proof of native menu glass/highlight appearance. The
packaged and account-specific checks below are tracked separately.

The manual Claude recovery checks pass: 10 provider/CLI tests plus seven usage
snapshot tests. A fake PTY panel delays Weekly by 700 ms to verify the two-second
settle window; other cases cover Unicode, scoped Weekly rows, loading redraws,
prompt refusal, deadlines, output caps and process reaping. A shutdown-fence test
quits during an active capture and verifies the process is gone and further
launches are rejected. These checks do not contact a live account.

### Installed Overview milestone

`d46c3ed` was built in release mode, Developer ID signed and installed at the
existing preview path. Strict/deep verification succeeds outside the sandbox
(the sandbox cannot evaluate the signing trust chain). The executable matches
the signed build, SHA-256
`ce314d2280a64c8cf6dfe23230bd7c78003efe7e47b6795e52de3b4cbc8e40db`.
The preceding app is preserved as a timestamped backup. Final desktop/daemon
checks passed, including 89 desktop tests and 19 daemon integration tests.

The actual installed menu passed these accessibility checks:

- Overview opens first and contains quota/freshness rows for all enabled
  providers, with no daily chart or cost rows.
- Cursor exposes Total, Cursor and Third Party at 100% used/0% left, and its
  detail tab shows $70 plan usage against $70 allowance and $0 on-demand spend.
- Codex exposes the live weekly limit, the provider-reported `gpt-reserve`
  weekly pool and Spark 5-hour/weekly pools. No general 5-hour/session window
  is invented when the account does not report one.
- Selecting a Codex history day changes its exact tokens and model rows while
  the menu stays open. Missing aggregate cost remains unavailable when some
  model usage is unpriced; individual priced models retain their estimates.
- Menu Bar Settings opens the correct settings page. The used status label
  and two-line layout save successfully; the layout's JSON controls are present.

Live verification also identified follow-up work: Claude's manual CLI probe
timed out on the installed CLI's startup presentation, and Cursor has no local
token history. These were not reported as passing account reads. Follow-up
collector changes must be verified and installed before claiming those details.
