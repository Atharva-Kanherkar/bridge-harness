# fix/agent-onboarding-auth-detection — Test Contract

## Functional Behavior

- A fresh Bridge installation enters a dedicated onboarding flow even when no agent runtime is currently available.
- Onboarding auto-detects supported runtimes already installed by the user and clearly distinguishes them from Bridge-managed installs.
- The agent step offers Codex, Claude Code, Cursor, and OpenCode independently; installing one agent does not install or select the others.
- A runtime that is installed and signed in is shown as ready and never offers a redundant **Sign in** action.
- A runtime that is installed but signed out offers an explicit **Sign in** action.
- Completing a provider login refreshes both runtime and adapter/auth state without restarting Bridge, so the sign-in action disappears when the provider is detected as signed in.
- The sign-in surface presents a calm browser-first progress experience. Raw terminal output remains available only as troubleshooting detail, and an OAuth URL emitted by the vendor is exposed as a safe external link.
- Users may continue without installing an agent. That choice is remembered locally and Bridge remains usable for project/navigation workflows.
- When at least one usable, non-signed-out adapter exists, onboarding can save recommended model defaults and complete normally.
- A desktop-launched Bridge detects npm-installed Codex when the launcher is found in a standard fallback directory but requires `node` from that same hydrated PATH.
- Provider login commands receive the same hydrated PATH as normal provider launches.

## Unit Tests

- `binary::tests::version_probe_uses_the_hydrated_gui_path` — an executable whose shebang resolves through a fallback directory reports its version.
- `binary::tests::fallbacks_include_common_user_package_manager_bins` — Bun, Volta, pnpm, asdf, and mise locations are candidates in a GUI environment.
- Model/onboarding state helpers distinguish fresh setup, dismissed optional setup, signed-in agents, signed-out agents, and unknown auth without guessing.
- Provider login presentation helpers extract only safe HTTP(S) URLs and strip terminal control noise from visible troubleshooting text.
- Managed agent UI tests assert **Sign in** appears only for `signed_out`, **Signed in** appears for `signed_in`, and login completion requests an auth refresh.

## Integration / Functional Tests

- The onboarding component renders all supported agents from the managed-agent API and labels an external Codex install as detected.
- With no detected agents, onboarding still renders install choices and allows **Continue without an agent**.
- With a signed-in detected agent, onboarding advances to recommended models without requesting another login.
- Installing or completing login invalidates/refetches health so adapter availability and auth state converge in the open UI.
- The existing model setup save path remains the sole way recommended profiles are persisted.

## Smoke Tests

- `bun run build` succeeds.
- `bun run test` succeeds.
- `bun run check:builtin-adapters` succeeds.
- The Harnesses settings page still supports install, repair, managed removal, and external-runtime ownership messaging.

## E2E Tests

- Manual desktop journey: start Bridge with a minimal GUI PATH and Homebrew npm Codex installed; onboarding detects Codex and reports the existing install.
- Manual desktop journey: signed-out Codex opens the polished sign-in flow; after browser authorization and CLI exit, Codex reads **Signed in** without restarting Bridge.
- Manual desktop journey: a user with a valid existing `~/.codex/auth.json` is not prompted to sign in.
- Manual desktop journey: a user with no agent may skip onboarding and open the normal workspace surface.

## Manual / cURL Tests

- N/A — this change uses local Tauri RPC and provider-owned CLI authentication, not an HTTP API.
- Review the onboarding and sign-in surfaces in both light and dark themes at narrow and wide desktop sizes.
