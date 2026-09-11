# Attachments, onboarding, and release candidates

This contract records the requested scope during final review; initial implementation preceded this document.

## Functional behavior
- Remove the bottom sidebar Refresh button; automatic refresh remains.
- Clipboard and file-picker images reach Codex as image data URLs, OpenCode as file parts, and Cursor as ACP image blocks when its handshake advertises image support.
- Preserve text, application context, image ordering, drafts on failure, and durable attachment history.
- Install and sign-in controls are reachable from first-run setup and harness settings. Failed authentication can open the provider's login flow without discarding the draft.
- Login uses the selected provider's executable, including managed installations. Browser authentication remains the provider's own flow.
- Prepare v0.5.6 candidates without merging or publishing from this PR.
- Preserve the Doto B icon through generation and release validation.

## Unit and integration tests
- Native image shapes and ordering in Codex, OpenCode, and the shared ACP client.
- Existing attachment persistence, queueing, unsupported-provider rejection, and Claude content-block tests.
- Composer file selection preserves the draft and delivers selected files.
- Auth recovery targets only supported providers and explicit authentication errors.
- Managed agents expose sign-in next to installed runtimes.
- Icon tests reject blank, transparent, and colored artwork.
- `bun run build` and `bun run test` must pass before opening the PR.

## Smoke and E2E checks
- Linux CI builds Debian and AppImage candidates and repackages the checked Debian artifact into an Arch package.
- macOS manual CI builds the PR revision with existing signing, notarization, and exact-bundle verification gates.
- Candidate artifacts do not establish desktop stability by themselves: verify launch, new workspace, paste/upload, login, chat, restart/history on macOS and Linux (X11 and Wayland) before publishing.
- Live paid-provider image recognition and OAuth completion require the user's provider account; report separately from protocol tests.

## Manual review
- Start a fresh profile, install an agent, choose Sign in, complete the browser flow, choose defaults, and send a message.
- Paste and upload two images with text, then an image alone. Check transcript after restart.
- Trigger expired login, confirm the draft remains, complete login, and retry.
- Download PR workflow artifacts; do not use a release tag until reviewed.

## Linux portability follow-up
- Installed Debian and extracted AppImage runtimes resolve both sidecars from `usr/lib/Bridge`, independent of build-machine paths.
- The Rust CI job installs the Node sidecar dependencies its terminal tests exercise.
- Linux read-only workers remain fail-closed; a verified Linux isolation implementation is a stable-release blocker. No unconfined fallback is permitted.

- Linux packaging targets Ubuntu 24.04+ and current Arch. Debian and Arch dependencies include Node.js 18+ and npm; installation is checked through the native package manager.
