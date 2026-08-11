# codex/issue-170-managed-agents-ui — Test Contract

## Scope And Stated Assumptions

- The UI calls only the `agents` domain from #169. No installation logic, no
  ownership logic, no marketplace logic, and no credential handling lives in
  React. Every question the card answers is answered by a field in the RPC
  response.
- **Removal is offered if and only if `status.removable` is true.** That field is
  the API's answer to "is this Bridge's to remove", so the UI never infers
  removability from a state string. A user-owned path therefore cannot show a
  Remove action even if a future state string is added that the UI does not know.
- Two things the issue's UX asks for cannot be rendered honestly yet, so they are
  not faked:
  - **No progress bar.** `install`/`repair`/`uninstall` run to completion and
    return a result; #169 deliberately ships no progress stream and no operation
    id. An in-flight operation shows an indeterminate busy state, not an invented
    percentage.
  - **No cancel button.** There is no cancel on the RPC surface. A button that
    silently does nothing is worse than its absence.
  Both become real when the operations move to a background job, which is a new
  result shape rather than a UI change.
- The `running` state is rendered and tested, but the backend does not emit it yet
  (`processId` is declared and unpopulated pending the #175 coordinator). The UI
  is correct in advance; the gap is the backend's.
- Vendor guidance is displayed verbatim when the RPC supplies `vendorMessage`, and
  the UI adds no API-key form, OAuth flow, login dashboard, or logout action.

## Functional Behavior

- One card per built-in integration, showing exactly one source state, its label,
  and the version and executable path when the RPC supplies them.
- State to actions:
  - `not_installed` → primary **Install**; no Remove.
  - `external` (backing `external`, `explicit`, or `bundled`) → primary **Start**;
    secondary **Install Bridge-managed copy**; **no Remove**, and the card names
    the source so a working user runtime is never presented as Bridge-managed.
  - `installed` and `ready` → primary **Start**; secondary **Remove**.
  - `repairable` → primary **Repair**; **Remove** offered because Bridge owns the
    drifted payload.
  - `running` → active state shown; **Remove disabled** with the reason given.
- Removing asks for confirmation first, and the confirmation names the exact
  managed payload — agent label, version, and the executable path — so a user is
  never asked to approve "remove this" without being told what "this" is.
- A failed operation surfaces the error's message in the card, associated with
  that card, and leaves the card usable.
- After a completed operation the card re-reads authoritative state from the RPC
  result rather than assuming what the new state should be.

## Unit Tests

- `renders_one_card_per_built_in_agent` — three cards, each labelled from the RPC.
- `each_state_offers_its_contracted_actions` — the six states each render their
  primary and secondary actions and no others.
- `a_user_managed_runtime_never_offers_removal` — for `external`, `explicit`, and
  `bundled` backings there is no Remove control at all, and the card names the
  source.
- `removal_is_driven_by_the_removable_field_not_the_state_string` — a status with
  an unrecognized state string and `removable: false` still offers no Remove; with
  `removable: true` it does.
- `a_running_agent_disables_removal_and_says_why` — the Remove control is disabled
  and carries an accessible explanation.
- `removing_requires_confirmation_naming_the_payload` — the confirmation contains
  the agent label, the version, and the executable path, and dismissing it calls
  nothing.
- `confirming_removal_calls_the_rpc_once_and_applies_the_returned_status` — the
  card's new state comes from the result, not from an optimistic guess.
- `an_in_flight_operation_shows_busy_without_progress_or_cancel` — an
  indeterminate busy state, no percentage, and no cancel control.
- `a_failed_operation_shows_its_message_and_leaves_the_card_usable` — the error is
  associated with the card and the actions are usable again.
- `vendor_guidance_is_shown_verbatim_and_adds_no_credential_controls` — the vendor
  message appears unmodified and the card renders no input, no password field, and
  no login or logout control.
- `the_panel_never_renders_a_credential_control` — a scan of the rendered markup
  across every state finds no `input`, no `type="password"`, and no
  login/logout/API-key affordance.

## Integration / Functional Tests

- `bun run test` passes, including the new component tests.
- `bun run build` passes; TypeScript compiles against the generated protocol
  types with no local re-declaration of an RPC shape.
- `cargo test --manifest-path src-tauri/Cargo.toml --workspace` passes.
- `scripts/check-builtin-adapters.sh` remains green: #162 unchanged.

## Smoke Tests

- With no managed payload and a runtime on PATH, the card shows the user-managed
  source and offers no Remove.
- Install from the card, confirm the state becomes ready, then Remove through the
  confirmation and confirm the card returns to not-installed.
- Confirm conversation history is still readable after a removal.

## E2E Tests

Deferred: the full clean-environment Install → Start → Remove → history → Reinstall
walk needs a real vendor fetch per agent and a running app, so it is a manual
acceptance pass rather than automated CI. The per-state behaviour is covered by
component tests above and the RPC layer by #169's suite.

## Manual / cURL Tests

- N/A for cURL: the UI speaks to the daemon through the existing transport.
- Manually confirm that removing a managed payload leaves vendor configuration,
  API keys, login sessions, and any global or custom installation untouched, and
  that reinstall is immediately available.
- Manually confirm keyboard-only operation: every action reachable by Tab, the
  confirmation trapping focus, and focus returning to the invoking control when
  it closes.
