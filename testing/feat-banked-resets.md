# feat/banked-resets — Test Contract

## Functional Behavior

- Codex snapshots preserve an unknown reset count as unknown, a reported zero as zero, and a positive count with at most a bounded list of sanitized grant details. Background reads request count only; interactive reads request details.
- Claude reset grants appear only when the interactive usage response includes a valid, account-offered program block. Missing or malformed blocks never become a reported zero or an actionable grant. Background collection remains SDK-only by default.
- The usage popover and Usage screen show banked resets, earliest expiry, and a Use reset action only when a credit can be used. A confirmation explains cleared windows and weekly-reset effects, including early use.
- Only an explicit user action can redeem. One click creates one idempotency key; an unconfirmed result preserves the key and triggers a fresh read before retry. Account changes abort redemption. Successful redemption refreshes quota, clears the applicable cooldown, and records an observability event.
- A user chat that reaches a Codex limit can offer an available credit inline. Existing rerouting remains available. Worker and agent paths never redeem.

## Unit Tests

- Codex schema fixtures: count-only, details, capped details, missing count, zero count, and malformed fields.
- Claude program fixtures: offered grant, absent block, malformed block, paused or limit-required grant, and outcome mapping.
- Protocol serialization: older snapshots without reset credits deserialize, unknown remains distinct from zero, and generated types match the wire.
- UI rendering: present, count-only, absent, and expiring credits; disabled reason; confirmation copy and outcome feedback.
- Redemption policy: provider allowlist, account switch, idempotency reuse, and no agent or worker invocation.

## Integration / Functional Tests

- Codex account RPC reads reset details interactively and sends a single consume request per key.
- Successful redemption refreshes provider snapshots and removes a stale cooldown.
- Claude interactive read includes reset status while automatic default refresh uses only the SDK.

## Smoke Tests

- `bun run build` succeeds and generated protocol artifacts are current.
- `bun run test` succeeds.
- A usage popover and Usage screen with mock credits can open the confirmation and display the result.

## E2E Tests

- N/A — a provider account with a live banked reset is required. Do not consume a real reset as part of automated verification.

## Manual / cURL Tests

- In mock mode, inspect credits present, count only, absent, and expiring states.
- With a test provider account, refresh usage, review confirmation, redeem once, and verify updated windows and cooldown. This is an account-affecting manual check and is not run during the PR build.
