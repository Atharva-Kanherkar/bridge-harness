# feat/daemon-ws-listener-credential-policy — Test Contract

Two slices of the cloud groundwork: the opt-in WebSocket transport on `bridged` (#129) and the deployment credential policy (#105).

## Functional Behavior

### WebSocket listener

- Without `--listen`, `bridged` binds only the Unix socket and the health listener.
- `--listen <ip:port>` adds a WebSocket transport carrying the same JSON-RPC contract: one frame per text message, same handshake gate, token, dispatch, limits and event forwarding as the Unix socket.
- A wrong or missing token gets `2001 unauthorized` and the connection closes. The handshake deadline is one absolute budget from TCP accept covering the HTTP upgrade and the protocol handshake: a peer that drips header bytes, stays silent, or floods pings before authenticating is reclaimed at it, not at a per-read timeout.
- The shared connection cap is reserved atomically across both accept loops, so it can never be exceeded.
- An upgrade carrying an `Origin` not exactly on the `--allowed-origin` list is refused with HTTP 403. An upgrade with no `Origin` is admitted to the token check. There is no wildcard; a malformed origin is a startup error.
- A non-loopback `--listen` address needs `--allow-remote-bind`.
- An oversized or binary message ends the connection. Beyond the shared connection cap a peer gets HTTP 503 carrying the `overloaded` body.
- The Unix-socket suite is unchanged and passes untouched.

### Credential policy

- `ExecutionTopology` (`embedded`, `local-daemon`, `remote-runner`) and `CredentialPolicy` (`user-managed`, `api-key-only`, `enterprise-managed`) are independent. The default is embedded + user-managed and changes no behavior and adds no round trip.
- `api-key-only` admits an API key or a cloud-provider account and refuses a subscription login, a signed-out harness, and any shape Bridge does not recognise. `enterprise-managed` admits only a cloud-provider account.
- Codex: before a thread opens, `account/read` is asked and classified (`apiKey`, `chatgpt`, `amazonBedrock`, null, other). A failed read fails closed.
- Claude: the environment the sidecar will inherit is classified before spawn; a deployment that forbids subscriptions neither injects the keychain token nor lets an inherited `CLAUDE_CODE_OAUTH_TOKEN` through.
- Every other adapter is refused under a restrictive policy: its credential source cannot be verified yet.
- A refusal is `credential_policy_violation` (1006) and names the policy and what it requires.
- `health/health` reports `deployment { topology, credentialPolicy }`. `bridged` pins it from `--credential-policy` / `--topology`, else `BRIDGE_CREDENTIAL_POLICY` / `BRIDGE_EXECUTION_TOPOLOGY`; an unparseable policy value fails closed to `api-key-only`.

## Unit Tests

- `remote::tests::*` — origin matching is exact and case/slash-insensitive; lookalikes, other ports and schemes are refused; no `Origin` is left to the token; an empty allowlist admits no browser; wildcards and paths are rejected; non-loopback needs the opt-in.
- `credential_policy::tests::*` — admission per policy and source; Codex `account/read` classification; Claude environment classification; the stable error and its message; environment parsing and fail-closed typo; the adapter gate.
- `codex_adapter::tests::a_subscription_login_is_refused_under_api_key_only_before_a_thread_opens` — against a fake app-server: chatgpt refused, apiKey admitted, signed-out and a failed read fail closed.
- `codex_adapter::tests::the_default_policy_asks_codex_nothing` — the default sends no request.
- `bridge-protocol` state tests — deployment wire names, defaulting from an older daemon's health; `error::tests::runtime_code_values_are_pinned` covers 1006.
- `tests::every_bridge_error_variant_maps_to_its_stable_protocol_code` and `protocol_mirror::result_payloads_mirror_core`.

## Integration Tests

- `bridged/tests/remote.rs` — a real WebSocket client drives a daemon end to end (handshake, call, mutation, event interleave, param validation); the handshake gate holds; origins off the list get 403; oversized and binary messages close; both transports share one daemon and one connection cap, and slots release.
- `remote.rs` also covers the deadline: `a_peer_dripping_http_header_bytes_cannot_outlive_the_handshake_deadline` and `a_peer_flooding_pings_before_authenticating_is_reclaimed_at_the_deadline` (both fail on the per-read-timeout implementation), and `server::tests` race two admissions at `cap - 1`.
- `bridged/tests/deployment.rs` — a pinned deployment is reported on `health/health` and an unverifiable adapter is refused with `credential_policy_violation` before any process spawns.

## Out of scope

- TLS termination, and a hosted server or runner (#108).
- Verifying Claude's selected credential from the SDK's init message; this slice checks the launch environment.
- Gateway-issued credentials under `enterprise-managed`.
