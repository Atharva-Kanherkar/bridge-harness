# Bridge account sign-in (WorkOS AuthKit)

Sign in to Bridge itself: optional, additive, separate from provider accounts.
Bridge is a public OAuth client (PKCE, loopback redirect, no client secret).

## Contract

| Behaviour | Test |
| --- | --- |
| The challenge is S256 of the verifier; the URL carries client id, redirect URI, state and challenge and no secret | `bridge_account::tests::the_challenge_is_s256_of_the_verifier`, `the_authorization_url_carries_…` |
| The callback needs the state this flow issued; a mismatch, a missing code and a denial are rejected | `the_callback_needs_the_state_this_flow_issued`, `the_listener_rejects_a_mismatched_state`, `a_forged_callback_ends_the_flow_…` |
| One callback, then nothing listens; a late one finds the port closed | `the_listener_takes_one_matching_callback_then_stops_listening`, `the_listener_times_out_and_closes`, `a_sign_in_nobody_finishes_times_out_signed_out` |
| Cancel closes the listener and leaves no keychain item, row or token | `cancelling_closes_the_listener_and_leaves_nothing_behind` |
| The exchange sends the verifier and no secret; the profile lands without a restart | `signing_in_exchanges_the_code_with_the_verifier_…` |
| A refused code ends signed out with Bridge's words, not the provider's | `a_code_workos_refuses_leaves_the_app_signed_out_with_a_readable_error` |
| Refresh persists the rotated token; `invalid_grant` signs out once and does not loop | `a_refresh_keeps_the_rotated_token_…`, `an_expired_or_revoked_refresh_token_signs_out_once_…` |
| Offline or a 5xx leaves the signed-in profile showing | `with_workos_unreachable_or_failing_the_signed_in_profile_still_shows` |
| Sign out removes the keychain item and the row, and asks WorkOS to end the session without waiting on it | `signing_out_removes_…`, `signing_out_works_with_workos_unreachable_…` |
| A refresh that lands after a sign-out is dropped | `a_refresh_that_lands_after_a_sign_out_is_dropped` |
| No client id means signed out, not broken | `without_a_client_id_bridge_is_simply_signed_out` |
| A secret key is refused as a client id | `the_client_id_comes_from_the_environment_before_the_build_and_is_never_a_secret_key` |
| No token in the status, the row, or the authorization URL | `signing_in_exchanges_the_code_…` |
| Tokens do not survive `secret_interception::sanitize`, JSON-quoted keys included | `account_tokens_do_not_survive_redaction` |
| Migration 109 applies to an existing database and has no token column | `store::tests::migration_109_…` |
| `bridged`'s keychain read helper refuses the account service | `bridged` bin: `the_keychain_helper_refuses_the_bridge_account_item` |
| The page renders signed out, signing in, signed in, an error, and an unavailable build; search finds it | `src/components/settings/AccountPage.test.tsx` |
| Wire shapes and the schema snapshot | `bridge-protocol` `messages::account` tests and the artifact drift test |

## Not covered by an automated test

- A real AuthKit round trip. It needs a WorkOS environment; run it by hand
  with `BRIDGE_WORKOS_CLIENT_ID` set to a staging client id.
- The real macOS Keychain. Tests use the in-memory store, as every other
  Keychain caller here does.
- A sandboxed worker reading the Keychain item. The read-only seatbelt profile
  is `(allow default)` with writes denied, because Claude Code reads its own
  login from the Keychain inside it, so the profile is not what keeps this
  item private. Two things do: the item's access list names the binary that
  wrote it, so another binary's read raises a system prompt instead of
  succeeding silently; and the one unprompted reader a worker can run, the
  `bridged` read helper, refuses this service (tested above).
