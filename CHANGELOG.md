# Changelog

## Unreleased

### 0.5.6 candidate

- Send pasted and uploaded images to Codex, OpenCode, and image-capable Cursor sessions.
- Offer agent installation and provider sign-in during setup, with login recovery that preserves the draft.
- Remove the sidebar reload control.
- Preserve the Doto B artwork through release builds.
- Build Linux Debian, AppImage, and Arch package candidates alongside the signed macOS DMG workflow.

- Show only connected integration activity from the past 24 hours on Work, using source timestamps rather than cache refresh times.
- Remove local checks, approvals, workspace drift, and task-tracker controls from the board. Open it from Settings → Work briefing → Open Work.
- Recognize native GitHub evidence fields and Slack workspace links; keep the browser preview empty instead of showing fabricated work.

## [0.5.5] - 2026-09-08

- Adopt the Span app icon: a mint deck over two off-white supports on a dark tile.
- Regenerate all platform icon exports and check the mint deck and off-white supports at every macOS scale before release.

## [0.5.4] - 2026-09-08

- Integrate main through `65b00ad`, retaining the signed-release, startup, native window, icon, and daemon ownership fixes from 0.5.2 and 0.5.3.
- Show immediate send feedback and improve streaming delivery, transcript updates, and Claude thinking-block completion.
- Bound failed worker-result repair loops and give reused workers a fresh repair budget.
- Improve model-switch handoffs, compaction ownership, provider resume, and context checkpoints.
- Include the updated macOS navigation, settings, model controls, and dark appearance.
- Settle detached model-switch summaries alongside tracked providers during clean shutdown.

## [0.5.3] - 2026-09-08

- Settle provider-process ownership during normal app shutdown so completed or idle chats are not falsely marked failed on the next launch.

## [0.5.2] - 2026-09-08

- Regenerate app icons from the source SVG and verify the visible mark before packaging.
- Report native startup errors in a dialog instead of panicking through macOS launch callbacks; retain bounded diagnostic logs with the executable path and version.
- Let macOS manage its titlebar controls. Use public AppKit material/layer APIs and defer geometry updates outside window callbacks.
- Update Bridge's owned material during resize without repeatedly retaining and releasing Wry's parent view, which reproduced a native deallocation failure.
- Keep other Bridge builds and their active daemons running when a conflicting copy opens.
- Reject overlong local socket paths before runtime startup, with an actionable data-directory error.
- Refresh recommended model profiles when live model discovery replaces their initial aliases.
- Validate signing, entitlements, bundled sidecar dependencies, and notarization before producing a public DMG.

## [0.5.1] - 2026-09-04

### Fixed

- Notarized macOS builds no longer abort on first launch. Hardened Runtime was enabled with an empty entitlements blob, so WKWebView could not JIT and Rust aborted on an uncatchable Objective-C exception in the event loop.
- Window chrome (traffic lights, wallpaper tint, corner radius) now catches Objective-C exceptions instead of aborting the process. On macOS 26 those AppKit calls can throw through tao's run-loop observer, which Rust cannot unwind.

## [0.5.0] - 2026-09-04

First public macOS disk image. Install by opening the DMG and dragging Bridge into Applications.

### Added

- Signed, notarized `.dmg` for macOS 12+ (Apple Silicon host builds).
- Native control room for Codex, Claude Code, OpenCode, and other structured harnesses — messages, reasoning, plans, tools, approvals, and diffs in the desktop UI instead of a provider TUI.
- Git worktree isolation for task workspaces and policy-authorized workers.
- Durable session forest: rewind or fork conversation history without pretending files or provider state were rewound.
- Supervised orchestration with capability tiers, write scopes, budgets, retries, and approvals.
- Checkpointing and resume (hot, provider-native, checkpoint-restored, or fresh).
- Authenticated browser bridge for a user-approved Chrome or Safari tab.
- In-app health, usage, memory, and GitHub surfaces.

### Notes for this cut

- Claude sessions still need Node.js 18+ on the machine. Codex, Claude Code, and OpenCode CLIs stay optional on `PATH`.
- Auto-update and App Store distribution are not in this release.
- The project remains all rights reserved unless a later license says otherwise.
