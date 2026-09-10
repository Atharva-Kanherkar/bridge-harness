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
Claude, Cursor, OpenCode and further providers remain the explicit follow-on
roadmap. Hooks and user plugins remain a separate project.
