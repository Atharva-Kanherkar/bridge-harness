# Changelog

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
