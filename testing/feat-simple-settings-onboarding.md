# feat/simple-settings-onboarding — Test Contract

## Functional Behavior
- Settings has four primary destinations: General, Coding agents, Permissions, and Data & storage. Existing deep links continue to reach their controls under the correct destination.
- General groups appearance, typing suggestions, menu bar, and updates. Coding agents is the single installation, sign-in, and configuration surface.
- Model preferences, task limits, saved setups, and internal instructions remain accessible through contextual details and Advanced, without extra settings rail entries. Built-in role presets are hidden from the ordinary saved-setup list.
- Marketplace contains Apps and Skills. Scheduled tasks, archived chats, saved setups, and briefing configuration are reachable from the features they support.
- User-facing wording explains what a setting does and where it applies. Browser copies live under Permissions. Existing security and ownership rules remain intact.
- Fresh installation shows onboarding before the application interface. Existing users with Bridge data or completed setup are not forced through onboarding again.
- Onboarding asks which coding agents to use, distinguishes an existing installation from a separate Bridge-managed copy, and shows missing agents honestly.
- Only supported managed agents offer installation. Unsupported installation/sign-in paths give concrete manual guidance, never a fake working button.
- Selected missing agents can be installed, repaired, or deselected. Unselected agents do not block setup or receive model profiles.
- Every selected agent must be installed, available, and report signed-in before setup can finish. Unknown sign-in status is explicitly unresolved. No skip/continue-without-agent path bypasses readiness.
- Closing or cancelling login does not imply success. Refresh availability after install/login and recheck readiness at final submission. Errors remain visible and retryable.
- Setup uses recommended models from selected, ready agents and persists their enabled state. Model customization is available later, not required during first run.

## Unit Tests
- Settings section mapping preserves all legacy deep links and exposes exactly four rail destinations.
- Onboarding readiness covers signed-in, signed-out, unknown, missing, broken, and unavailable agents.
- Selection filters model recommendations and blocks empty selection.
- Onboarding completion persistence retains the existing-user migration behavior.

## Integration / Functional Tests
- Settings navigation and search open the correct contextual pages; drafts survive navigation; reset retains its confirmation.
- Marketplace defaults to Apps, includes Skills, and does not mount a second agent installer or scheduled-task UI.
- Onboarding tests cover existing external and managed installations, installing a missing agent, login cancellation, unknown authentication, selection changes, detection/save failure, and successful completion.
- Fresh App does not render the workspace until onboarding completes.

## Smoke Tests
- `bun run build` passes.
- `bun run test` passes, including frontend, sidecars, native menu bar, release scripts, and Rust workspace tests.

## E2E Tests
- Browser smoke: four settings destinations, contextual navigation, Marketplace Apps/Skills, and first-run choose/connect journey against the browser mock API.
- Real installation and provider OAuth cannot run unattended against a user's accounts; do not claim those were completed. Cover the boundary with integration tests and reviewer steps below.

## Manual / cURL Tests
- Launch a fresh Tauri profile. Select an installed agent and a missing supported agent. Confirm installation-source labels and that Bridge never offers to remove the user's existing installation.
- Install the missing agent, sign in using its provider flow, and refresh. Confirm the workspace remains hidden until every selected agent reports signed-in and available.
- Cancel sign-in, simulate unknown sign-in status, or fail an installation. Confirm setup stays open with actionable guidance; deselecting the unfinished agent allows another ready selection to proceed.
- Enter Bridge and confirm only selected agents are enabled and model profiles use them. Restart and confirm onboarding stays completed.
- Check General, Coding agents, Permissions, and Data & storage; follow old usage/permissions/menu-bar links; find scheduled tasks and archived chats from the sidebar and saved setups near new chat.
