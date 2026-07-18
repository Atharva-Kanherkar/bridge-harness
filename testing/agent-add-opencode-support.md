# agent/add-opencode-support — Test Contract

## Functional Behavior

- Bridge resolves OpenCode in this order: an explicit executable selected in Harness settings, a Bridge-managed executable location, then the user's `PATH`/standard installation locations.
- An unavailable, non-executable, or OpenCode version older than 1.18.3 is reported in Harness settings with a concrete remediation; Bridge never silently selects a different harness.
- Bridge obtains provider status, provider display names, provider defaults, and model metadata from OpenCode's structured local server API. It does not infer authentication by parsing terminal decoration or maintain a hardcoded provider/model catalog.
- Connected providers from OpenCode's credential store, inherited environment, and OpenCode configuration are shown as connected without exposing credential values.
- A provider API key submitted through Bridge is sent once to OpenCode's authenticated loopback `auth.set` endpoint, is never stored in Bridge configuration or logs, and causes provider/model discovery to refresh.
- Removing provider authentication calls OpenCode's authenticated loopback `auth.remove` endpoint and refreshes provider/model discovery.
- Model identifiers remain fully provider-qualified (`provider/model`). Only models from connected providers are selectable. A configured visible-model allowlist filters the picker; an empty allowlist means all connected models are visible.
- OpenCode Go is handled as the ordinary `opencode-go` provider. No Go-specific model names or subscription branches exist in Bridge.
- Bridge assigns its internal fast/standard/strong routing tiers deterministically over the discovered models while preserving each exact OpenCode model identifier.
- Existing Codex and Claude configuration, model routing, and sessions remain unchanged.

## Unit Tests

- `opencode_adapter::tests::resolves_custom_managed_and_system_executables_in_order` — executable precedence is deterministic and invalid overrides fail closed.
- `opencode_adapter::tests::parses_structured_provider_catalog_without_credentials` — provider/model/default metadata is normalized without secret fields.
- `opencode_adapter::tests::filters_visible_models_and_preserves_qualified_ids` — allowlisting cannot rewrite or admit disconnected-provider models.
- `opencode_adapter::tests::assigns_deterministic_bridge_tiers_to_discovered_models` — all advertised models receive one tier and every populated tier has one default.
- `opencode_adapter::tests::rejects_opencode_versions_with_the_incompatible_context_schema` — versions below 1.18.3 remain unavailable.
- `agent_config::tests::opencode_advanced_config_rejects_invalid_shapes_and_secret_fields` — executable and visible-model settings validate; provider keys cannot enter durable configuration.
- Frontend settings tests verify provider status, key entry redaction, model visibility controls, and executable override behavior.

## Integration / Functional Tests

- A fake OpenCode executable/server fixture returns structured `/provider` data; Bridge exposes only connected provider models through its adapter descriptor.
- Saving an OpenCode Harness configuration updates executable/visibility behavior for subsequent discovery and sessions without restarting Bridge.
- Setting and removing a fixture provider key sends the expected `Auth` payload over an authenticated loopback connection and never returns the key.
- Existing adapter registry and model-profile tests pass with the dynamic OpenCode catalog.

## Smoke Tests

- `bun run build` succeeds.
- `bun run test` succeeds.
- With OpenCode 1.18.3 or newer and an existing OpenCode Go credential, provider discovery reports `opencode-go` connected and returns its current provider-qualified models.
- A Bridge-shaped local server request using a discovered Go model returns an assistant response.

## E2E Tests

- Open Settings → Harnesses → OpenCode; refresh providers; observe the connected Go provider and its models; select visible models and a default; start an OpenCode chat; receive a response.
- Enter a test provider API key, verify connected state appears after refresh, remove it, and verify the provider disconnects. Use a disposable provider credential only; never mutate a production credential during automated verification.

## Manual / cURL Tests

- Start the configured OpenCode executable with random loopback Basic Auth, then verify `GET /provider` returns `all`, `connected`, and `default` without credential material.
- Verify `PUT /auth/{providerID}` accepts `{ "type": "api", "key": "…" }` and `DELETE /auth/{providerID}` removes it using a disposable OpenCode data directory.
- Confirm Bridge's persisted Harness JSON contains executable and visible-model configuration but no API key, token, authorization header, or credential value.
- Confirm the existing local OpenCode Go credential can complete a prompt through the same server endpoints used by Bridge.
