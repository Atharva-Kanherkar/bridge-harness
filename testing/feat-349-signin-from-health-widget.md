# feat/349-signin-from-health-widget — Test Contract

Source: [issue #349](https://github.com/Atharva-Kanherkar/bridge-harness/issues/349).
Locked before implementation. This file is the definition of "done" for this branch.

## Functional Behavior

### Auth state on the health payload (backend)

- Each usage provider (`claude`, `codex`, `opencode`) exposes an auth state derived
  from its own credential store, surfaced per-adapter on the health payload:
  - `signed_in` — the vendor's credential store holds a usable credential.
  - `signed_out` — the CLI is installed but no usable credential exists.
  - `unknown` — presence cannot be determined (probe error), never guessed.
- Install status stays orthogonal: a provider whose CLI binary cannot be resolved is
  reported through the existing availability path (`available: false` +
  `unavailable_reason`), and the UI renders it as *not installed*, never as signed out.
- Probe sources — metadata/presence checks only. Bridge never opens, reads, or
  parses credential file contents, and never invokes anything that prints secret
  values:
  - Claude: `~/.claude/.credentials.json` exists (non-empty), or macOS Keychain
    generic password service `Claude Code-credentials` exists (status bits only,
    entry value never requested).
  - Codex: `~/.codex/auth.json` exists with size > 0 (`fs::metadata` only).
  - OpenCode: its auth store exists with size > 0 under its data dir
    (XDG-aware; `fs::metadata` only). Probe failure degrades to `unknown`, not
    `signed_out`.

### Start a provider login from Bridge (backend)

- A new protocol method starts a provider's own login flow:
  - Input: `{ provider }`.
  - Behavior: runs the vendor's documented login entry point inside Bridge's existing
    PTY/terminal infrastructure (the dock terminal pane hosts it). Browser-handoff
    flows (Codex/Claude OAuth) are started by the vendor command itself — Bridge only
    launches the process.
  - Output: the terminal pane hosting the flow, so the UI can surface it inline.
  - Completion: when the vendor process exits, health re-reads and the auth state
    updates. Bridge does not parse or intercept the credential at any point.
- No field to type a token into exists anywhere in this feature (same doctrine as
  `ManagedAgentsPanel`).

### Usage widget reflects auth state (frontend)

- Collapsed widget row per provider, in priority order:
  1. CLI not installed → ring visibly inert + label reports missing install.
  2. Signed out → ring visibly inert + reads "Not signed in" (never "Limit unknown").
  3. Signed in but no stable quota → unchanged "unknown"/"Limit unknown" behavior
     (quota-less must remain distinguishable from signed-out).
  4. Signed in with quota → unchanged percentage display.
- Detail panel (`ProviderDetail`) mirrors the same states and adds a sign-in control
  on a signed-out provider's row. Activating it starts the vendor flow inline via the
  backend method above rather than instructing the user to open Settings or a
  terminal.
- When the flow completes, the widget re-reads health without an app restart and the
  ring populates (usage/quota/context for that provider appear).
- **Listen before launch:** the UI subscribes to the login PTY's output and exit
  events *before* starting the vendor process, so no early output (OAuth URL,
  first prompt) can be lost to a subscription race.
- Settings → Harnesses keeps working unchanged. This feature is an additional entry
  point, not a move.

## Unit Tests

Rust (`src-tauri/`):

- `auth_probe_reports_signed_in_when_credential_store_present` — temp HOME with each
  vendor's store populated → `signed_in` (Claude file variant; keychain variant
  covered where testable, otherwise gated).
- `auth_probe_reports_signed_out_when_cli_present_but_store_absent` — temp HOME,
  empty stores → `signed_out`.
- `auth_probe_reports_unknown_when_store_unreadable` — unparsable JSON → `unknown`,
  not `signed_out`.
- `missing_binary_is_not_reported_as_signed_out` — unresolved binary → adapter
  unavailable; auth probe either skipped or reported independently of "signed out".
- Protocol: round-trip test for the extended health/adapter types with camelCase wire
  names (extend the existing pattern in `bridge-protocol/src/messages/state.rs`).
- Generated frontend protocol artifact regenerated and consistent
  (`cargo run --manifest-path src-tauri/Cargo.toml -p bridge-protocol --bin generate-protocol-artifacts`).

TypeScript (`src/`):

- `UsageWidget` renders "Not signed in" for a signed-out provider with no snapshot —
  and never the string "Limit unknown" for that case.
- `UsageWidget` still renders "Limit unknown" for a signed-in provider with no
  snapshot (the two remain distinguishable).
- `UsageWidget` renders a not-installed state that is distinct from signed out.
- Sign-in control appears only on signed-out providers' rows and invokes the start
  callback with the right provider id.
- Ring inertness: signed-out/not-installed providers render no progress arc.

## Integration / Functional Tests

- Health payload flows adapters into `UsageWidget` via `App.tsx` (prop pass-through;
  widget must render correctly with `adapters` absent too — backwards compatible).
- New login method is registered in the protocol method table and dispatched end-to-end
  (`bridged` dispatch + `api.rs`) without breaking existing methods.

## Smoke Tests

- `bun run build` green.
- `bun run test` (vitest) green including new cases.
- `cargo test` green for touched crates.
- App boots to the main view with the widget present.

## E2E Tests

N/A — desktop app; manual verification below covers the journey.

## Manual Tests

1. `bun run tauri dev` with a machine where one provider is signed out → collapsed
   widget shows "Not signed in" + inert ring for it; panel shows the sign-in control.
2. Click sign-in on that provider → dock terminal pane opens running the vendor flow;
   complete it; widget re-reads health without restart and fills the ring.
3. Provider with CLI absent (temporarily shadowed PATH) → reported as not installed,
   pointing at the install path, never as signed out.
4. Confirm no credential value appears anywhere in logs during both flows.

## Acceptance criteria traceability (from the issue)

- [ ] Signed-out labelled in collapsed widget AND panel; never conflated with
      quota-less. → §Unit Tests (TS) items 1–2.
- [ ] Claude sign-in completable from the health widget without Settings and without
      leaving the app. → §Functional Behavior (login flow) + Manual Test 2.
- [ ] Usage/quota/context populate after completion with no restart. → §Functional
      Behavior (completion) + Manual Test 2.
- [ ] No credential entered into, stored by, or logged by Bridge. → §Functional
      Behavior probes + Manual Test 4.
- [ ] Missing CLI reported as missing, not signed out, pointing at install path. →
      §Functional Behavior (install status) + Manual Test 3.
