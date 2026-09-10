# Menu Bar implementation validation

Checked on 2026-09-10, on Apple silicon with macOS 26. The implementation is
isolated in `codex/native-menu-bar`, based on main at `384997b`.

## Completed

- The full Rust workspace suite passed: 2,600 tests, 13 intentionally ignored.
  Subsequent focused checks passed after the final changes: usage semantics,
  159 protocol tests plus two documentation tests, and the new daemon socket
  test for settings persistence, invalid-save rejection and unavailable usage.
- `cargo check --workspace` passed. The latest native code also built through
  Tauri into a macOS app bundle.
- Swift checks passed for the shared Rust/Swift JSON fixture, zero, stale and
  unpriced metrics, cost qualifiers, reset countdowns, icon dimensions,
  `isTemplate`, transparent background and monochrome alpha-mask pixels.
- Swift compiled with a macOS 12 deployment target on arm64. The same source
  passed a macOS 12 x86_64 availability/type check. This is not a macOS 12
  runtime test.
- A real, isolated daemon accepted the 1.8 handshake and menu preference
  save/read calls. With a minimal GUI-like PATH, a read-only Codex refresh
  completed in 2.79 seconds and returned account, plan, session quota/reset,
  measured tokens, a model breakdown and estimated cost. The provider returned
  no weekly quota; its value and reset stayed unavailable. No chat or turn was
  created. The temporary daemon and database were removed after the check.
- An earlier focused frontend run passed 31 tests across the settings page,
  existing Settings screen, search, usage adapter and API boundary. Production
  frontend bundling passed at that point. Final TypeScript compilation also
  passed after later error-handling and polling fixes.
- Release-script tests passed (16 Node and 12 Python); Claude sidecar tests
  passed (50 tests, one authenticated-runtime test skipped).

## Packaged Swift milestone

The prototype uses a distinct `dev.bridge.deck.menubar-preview` bundle ID and
data directory. It hosts the Swift status item and menu inside the Tauri
process, with the existing Rust backend supplying JSON over the C ABI.

The installed prototype opened its native menu, displayed recorded token/model
and cost data, and dismissed with Escape. Its initial GUI-only Codex discovery
failure led to the centralized installed-app runtime fallback; the later daemon
check above verified that fix. The live account display in the updated native
menu still needs a final interactive check.

The latest native validation bundle is saved at:

`/Users/yashaf/Library/Caches/bridge-menu-bar-validation-20260910/Bridge Menu Bar Preview.app`

It is Developer ID signed with hardened runtime. Strict/deep signature
verification passes, and its Mach-O minimum OS is 12.0. It has no trusted
timestamp or notarization. This small integration prototype deliberately omits
release sidecars/resources and retains the last successfully bundled frontend;
it is not a complete distributable Bridge release.

## Remaining before cutover

The Mac locked during interactive verification. The frontend build/test rerun
also stalled before executing tests, during an esbuild filesystem read; macOS
logged an access check at the same point. Those stalled processes were stopped.
Neither the full frontend suite nor a fresh final production bundle is claimed
as passing.

After the Mac is available:

1. Complete the normal frontend build and full test command, including the new
   usage-read-failure regression test, and package the current frontend.
2. Verify the updated native account/plan, session and weekly availability,
   countdowns, used/remaining/spend modes, model submenus, and Refresh completion.
3. Verify native Settings navigation, saved visibility/provider/detail choices,
   disabled-menu explicit opening, and persistence across app restart.
4. Check light/dark/highlighted appearances, accessibility contrast, reduced
   transparency, keyboard navigation, focus and smaller displays.
5. Exercise an actual supported macOS 12 runtime before removing the fallback.

The legacy meter remains available through `BRIDGE_LEGACY_METER=1`. Its UI and
old shared presentation dependency have not been retired prematurely. Claude,
Cursor, OpenCode and further providers remain the explicit follow-on roadmap.
