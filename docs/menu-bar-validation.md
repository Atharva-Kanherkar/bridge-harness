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

### Final provider follow-up

`3509451` recognizes the installed Claude CLI's standalone `$` screen-reader
prompt; `132549f` adds Cursor's dashboard account history without inserting it
into the device ledger. The final production build, native Swift checks and
11 focused React tests pass. Ten Claude tests, eight Cursor tests and eight
usage-overview tests pass, including cross-midnight unknown/stale semantics.
All 19 daemon integration tests pass after the final provider changes.
Independent source review found no remaining material issue.

The signed preview was rebuilt and installed with executable SHA-256
`0b76bd2cade6b9f985914b54e606aa0ba4d8d89bfdc163c3e76fd17f85c5de07`.
Staged strict/deep signing verification and byte matching passed before the
atomic replacement; the preceding app remains backed up. Used mode, Overview
and the pasted icon/space/used layout survived relaunch.
No new Bridge crash report was present after installed-app verification.

A manual Refresh in this installed app recovered Claude successfully. The
native Overview showed live 5-hour and Weekly percentages; the detail tab
identified `Source: Claude CLI` and retained local daily token/model/cost
history. CLI-only account identity, plan and reset metadata remain absent.
OpenCode still needs a user-completed Zen workspace sign-in.

Cursor's authenticated live test was rejected by automatic approval review
because the standalone probe would read a stored session credential and send
it to Cursor's dashboard API without explicit credential-use approval. The
user was asked for that approval. Cursor collection is disabled in the final
preview while the answer is pending, so neither startup nor manual refresh
performs that unapproved request. The feature is committed and covered by
fake-response tests; it is not claimed as live-verified.

### CodexBar colors and provider spacing · 2026-09-12

`ad7d696` adapts the reference provider palette, static progress drawing, and
text-only AppKit switcher. Native checks pass, including the real five-button
layout, minimum title room, equal widths, non-overlap, Cursor's position after
Claude, selection dispatch, and stable geometry after selection/visibility
changes. Production-card fixtures were inspected in light/dark appearances;
Cursor's full used lane is mint, Claude is terracotta, Codex is teal, and
OpenCode is blue. The x86_64 macOS 12 type check and production frontend/native
release build pass. No backend or authentication code changed.

The signed preview was installed with executable SHA-256
`a070a0ab32e00d194749bc2ed3bc30a21e551e231bc7ad057948c7aa4ac6bcf1`.
Strict/deep signing verification and staged/installed executable matching pass.
The previous app is preserved as
`Bridge Menu Bar Preview.previous-20260912-140415.app`.
The installed app opens Overview, exposes the new native toggle buttons,
and switches to Claude's detail card without dismissing the menu. No new
Bridge crash report was present. The five-provider spacing/color render uses
fixture data; Cursor remains disabled pending explicit credential-use approval.

### Favorites, horizontal overflow, and Claude scoped limits · 2026-09-12

`3577749` adds persisted top-three favorites and protocol 1.12; `ed8d998`
reads Claude model-specific CLI limits; `504ac0d` adds the native scroll strip.
Default favorites are Codex, Claude, and Cursor, independent of collection
switches. The strip keeps Overview fixed and three equal provider slots within
the existing 350-point width. Long overflow names truncate with tooltips.

Validation at these commits:

- Production frontend build and all 2,058 frontend tests pass, including 15
  focused Menu Bar tests for layout, favorites, and settings persistence.
- All 164 protocol unit tests and two doctests pass, including the older-daemon
  rejection and default-favorites migration checks. Three Menu Bar store tests
  pass for order, validation, and preserving disabled collectors.
- Claude OAuth tests: 3 pass. Claude CLI tests: 11 pass, covering Fable alongside
  other scoped models, incomplete sections, redraws, and bounded capture.
- All 89 desktop library tests pass. The initial sandbox run blocked four
  temporary Unix-socket fixtures; rerunning outside that restriction passed.
- Native Swift checks pass for disconnected-data isolation, 69-provider
  overflow, exact three-slot geometry with a long fourth name, wheel direction,
  clamping, selection reveal, and preserving scroll position across snapshots.
  The x86_64 macOS 12 type check also passes.
- Synthetic production-card renders were inspected in light/dark appearances,
  including the default three tabs, provider 69, and disconnected Cursor.
  These fixtures demonstrate layout and semantics, not live account results.

The reference was CodexBar `928166f`, inspected by a GPT-5.6 Sol subagent before
implementation. The palette and button styling retain that reference; the
fixed-width horizontal scrolling adapts it to the requested Bridge behavior.
Current collectors remain Codex, Claude, Cursor, and OpenCode. Testing 69 tabs
does not claim support for 69 account adapters.

Installed-app checking found a hosted-button issue: the disconnected provider's
Settings link opened the right page but left the native menu tracking above it.
`2c31167` follows CodexBar's explicit `cancelTrackingWithoutAnimation()` before
opening Settings. Native checks and the Intel macOS 12 type check pass again;
the rebuilt installed app dismisses the menu correctly when this link is clicked.

The final signed preview at `/Users/yashaf/Applications/Bridge Menu Bar Preview.app`
has executable SHA-256
`d5006d1fc100028b447d12a3bb0d2db37ff9bdf585a0875102991ee4baab65df`.
Strict/deep signature checks and staged/installed byte matching pass. The prior
preview is retained as `Bridge Menu Bar Preview.previous-20260912-150044.app`.
No new Bridge crash report was present after launch.

Live checks confirmed the default tabs, arrow navigation to OpenCode, selection
without closing the menu, disconnected Cursor with no cached account details,
favorite swapping and saving, and the corrected Settings link. Default favorites
and Show used mode survived relaunch. The final manual Claude refresh recovered
live 5-hour (0% used), Weekly (4% used), and a distinct Weekly · Fable only
(0% used) via Claude CLI. All three appeared together in the installed Overview;
the percentages are an observation, not a permanent account state.
Claude's expired stored OAuth session still makes background refresh stale;
manual Refresh can recover CLI usage without silently repairing credentials.
Cursor collection remains off pending the previously requested authorization.

### Icon display precedence and stable daily history · 2026-09-12

The saved icon/space/used custom layout took precedence over a standard Cost
display while Settings still showed Today's spend. `0779ecee` makes the active
custom mode explicit; selecting a standard display clears its override. The
editor also offers Use standard display and retains the current layout on a
failed save.

`dad314f4`, `e54262cc`, and `136b6ad9` retain each provider's selected history
metric/day in MenuState, eliminate synchronous menu sizing from chart selection,
and defer/coalesce snapshot and provider measurements in the menu tracking loop.
Today and Last 30 days each occupy one compact token/cost row. Selected-day totals
use one additional row; model/pricing details stay in their submenu. Synthetic
production-card renders were inspected, including a selected day, unavailable
cost, and light/dark appearances. These are fixture data, not account readings.

The old installed build lost its vertical scroll controls during a Tokens/Cost
switch while retaining long content. A full process crash was not reproduced;
a process sample was waiting normally in AppKit's menu tracking loop. A separate
macOS resource diagnostic reported 2.1 GB of temporary writes over 33 minutes,
with SQLite aggregation on both provider-snapshot and Codex-snapshot paths.
`e3ba8317` removes that duplicate aggregation and caches the shared history
summary. Invalidation observes both same-connection and external database
changes, local date, and timezone. This conservatively includes unrelated writes.
Current snapshot timestamps and quota freshness/expiry remain uncached. No
authentication, persistence schema, or protocol changes are introduced here.

Validation:

- Production frontend build and all 2,060 frontend tests pass, including 17
  focused Menu Bar tests covering custom/standard precedence and failed saves.
- All 11 usage overview tests pass, including shared cache reuse, pricing and
  ledger changes, external writes, date/timezone invalidation, and quota expiry
  while reusing the same history aggregate.
- All 89 desktop library tests pass. Native Swift checks and the x86_64 macOS 12
  type check pass, including retained history selection, no synchronous resizing
  on metric changes, compact/unavailable/stale summary labels, and expired-day
  selection without substituting month totals.

GPT-5.6 Sol subagents inspected CodexBar `928166f` before implementation and
reviewed the resulting native presentation and cache. The native final review's
stale-token label finding was corrected and checked before packaging.

The rebuilt preview was signed, verified with strict/deep checks, and atomically
installed at `/Users/yashaf/Applications/Bridge Menu Bar Preview.app`. Its
executable SHA-256 is
`76285d0eb067f5fb5425bb4eddd3574f3e4a8787dd2d571defafdb27f4c2d4af`;
the previous app is retained as
`Bridge Menu Bar Preview.previous-20260912-155135.app`.

Installed-app checks through computer use confirmed:

- The existing custom icon/space/used layout is identified as Custom layout.
  Selecting Today's spend clears it, shows Standard display is active, and
  survives a deliberate app relaunch. The capture tool did not expose the
  status-item title itself; that rendering is covered by the native checks.
- Repeated Codex Tokens/Cost toggles retain the open menu and its controls.
  Selecting September 8 shows one compact day total and retains that selection
  in cost mode, across Claude/Codex switches, and after reopening the menu.
  Claude has an independent history selection. No account refresh is triggered
  by those selection actions.
- The new Today/Last 30 days rows are present without the previous inline model
  list. Unknown aggregate cost stays unavailable while known individual days
  remain selectable.

No additional process startup was logged during the toggle/provider tests, and
no fresh Bridge crash report was present. A later explicit main-window close
exited the app through the existing shell behavior; the preview was relaunched
and the saved spend preference checked again. This main-window close behavior
is separate from native menu dismissal. Intermittent computer-use capture errors
required refreshing accessibility state; direct menu interactions then resumed.
Cursor collection remains disabled as before.

### Provider arrows on opposite edges · 2026-09-12

`ebfece9b` places Previous providers at the leading edge and More providers at
the trailing edge: `‹ Overview Codex Claude Cursor ›`. Overview remains fixed
while only the provider list scrolls. In overflow, both arrows have the same
2-point inner gap; three equal 74-point provider slots fit inside the existing
350-point width. When there is no overflow, arrows and their reserved space
remain hidden.

A GPT-5.6 Sol subagent inspected CodexBar `928166f` first. CodexBar itself uses
adaptive rows rather than overflow arrows, so this change adapts the existing
Bridge strip to the requested edge placement. Existing native scrolling,
selection, clamping, three-favorite, and 69-provider checks pass, as does the
x86_64 macOS 12 type check. Synthetic light/dark and high-contrast renders show
the default favorites and the final overflow page with an arrow on each side.

The production build passed and the signed preview was installed with
executable SHA-256
`b5b2da6a96446e86ea6c8f849fbfaa18af2dbba614dc81c598c59e73f7c32d58`.
Strict/deep signature verification and staged/installed byte matching passed;
the previous build is preserved as
`Bridge Menu Bar Preview.previous-20260912-160300.app`.
The exact installed app launched as PID 86759. macOS locked before the native
menu could be reopened, so final installed arrow clicks remain unverified;
spacing evidence comes from the synthetic production-control renders above.

### Fable-only, Grok Bot, guide markers, and padded arrows · 2026-09-12

`8bc9236b` matches CodexBar's model-scoped Claude quota mapper. Enforceable
Fable limits can report `is_active: false`; these are now retained with stable
model identity, deduplication, and a Weekly · Fable only label. Only exact
weekly/scoped entries with a numeric percentage and named model qualify;
all-model totals are not repeated as scoped rows. A payload containing only
Fable's allowance is supported, including reported zero. The existing manual
Claude CLI parser already supports Fable-only sections and remains unchanged.

The same commit adds Cursor's optional Grok Bot quota request, using the existing
Cursor session, a fixed HTTPS origin/path, disabled redirects, sensitive cookie
headers, JSON body, and five-second timeout. The separate allowance uses its own
reset and reported period. Missing, failed, or inapplicable Grok responses do not
discard the ordinary Cursor/Third Party quotas. It does not derive quota from
Grok model token counts or enable a disabled provider.

`bb9070e7` adds fixed visual markers at 50% and 75%, drawn within the existing
Canvas using CodexBar's 5-point punched slot and 1-point neutral stripe. These
are the user's requested positions, not a copy of CodexBar's configured warning
defaults. Both used/remaining modes share these visual positions. Arrow controls
now have equal 6-point outer gutters and 4-point inner gaps, with three 72-point
provider slots. Overview fits its 76-point segment without truncation.

Two GPT-5.6 Sol subagents inspected CodexBar `928166f` before implementation and
reviewed the final provider/native changes. No actionable issue remained.
Validation includes 32 relevant Rust provider tests, 11 shared-snapshot tests,
native Swift checks, and the Intel macOS 12 type check. Native tests verify
marker geometry at both display scales and actual arrow/segment frames. Synthetic
production-card renders show Claude 5-hour/Weekly/Fable only, Cursor's distinct
Grok Bot row, and the updated marker/arrow appearance in light and dark modes.
Those renders use fixtures and are not live account observations.

The production build passed and the signed preview was installed at
`/Users/yashaf/Applications/Bridge Menu Bar Preview.app` with executable SHA-256
`587ad99307affef7637cfd3b720acf2a3f7212b0cd9d0cb6767a8901db514a29`.
Strict/deep signature verification and installed byte matching passed. The
preceding app is retained as `Bridge Menu Bar Preview.previous-20260912-162333.app`.

The installed settings already had Cursor collection enabled when checked in
this turn; that choice was preserved. Live Overview and Cursor details both
showed Total, Cursor, Third Party, and Grok Bot (35% used, 65% left, with its own
reset countdown). These are point-in-time observations, not permanent account
values. The left and right navigation controls both passed a separate-call live
retry without closing the menu; one initial automation sequence dismissed it.
The native accessibility tree exposes the 50/75 guide marker hint, and the
production-control renders provide the spacing and marker visual evidence.

Manual Refresh recovered Claude's live 5-hour (0%) and Weekly (4%) limits via
Claude CLI. This response omitted Fable, so live Fable display is not claimed.
The OAuth scoped mapper and CLI Fable-only cases are covered by tests; no absent
allowance is manufactured. No fresh Bridge crash report or extra process startup
was present after the installed checks.

### Pinned status usage and quieter navigation · 2026-09-12

`77911027` keeps plain status text, custom layouts, and the template meter on the
first favorite while detail tabs continue to switch independently. Settings
display that provider instead of offering a conflicting selection control.
With no favorites, status uses the first enabled provider; a disconnected first
favorite stays explicitly disconnected. The native card hides its scrollbar
while retaining document scrolling, bounded height, and offset clamping.
Provider changes use a 120ms layer opacity fade after the coalesced layout pass;
Reduce Motion disables the fade, and menu geometry is never animated.

A GPT-5.6 Sol subagent inspected CodexBar `928166f` before implementation and
reviewed the diff with no actionable findings. Native checks passed, including
status text/accessibility/custom-layout pinning, actual template-meter pixels,
favorite reorder and disconnected cases, and reaching the final row with hidden
scroll indicators. All 18 Menu Bar frontend tests, the Intel macOS 12 type check,
and the production frontend/native app build passed.

The signed preview was installed at
`/Users/yashaf/Applications/Bridge Menu Bar Preview.app` with executable SHA-256
`481f28bd07ca3e397d5b7dca4c426e6afff1c27ba7a382faebc676443a7c2b83`.
Strict/deep signature checks and staged/installed byte matching passed; the
preceding preview is retained as
`Bridge Menu Bar Preview.previous-20260912-174218.app`.

Live accessibility checks confirmed no native scrollbar control and successive
Codex, Cursor, Claude, and Cursor selections within the same open menu. Cursor's
Cost history choice survived the provider changes; it was then restored to its
original Tokens display. After opening Cursor, Settings still showed Codex as
Favorite 1 and Provider beside the icon. The capture tool could not supply an
image of the transient menu, so visual fade timing and the live scroll offset
remain unverified; deterministic native probes cover scroll geometry and status
rendering. No fresh Bridge/WebKit crash report or extra process startup appeared
during these checks.
