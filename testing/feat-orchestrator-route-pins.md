# feat/orchestrator-route-pins — Test Contract

## Functional Behavior

- A `bridge-delegate` request may carry an optional `harness` and/or `model` pin next to the required `capabilityTier`. The orchestrator sets a pin when the user names a harness or model, or when it has a concrete reason from the routing inventory. With no pin, routing is unchanged.
- A model pin resolves against the installed catalog: exact id, then case-insensitive id or label, then a unique family match (`opus` or `claude opus` finds the Claude Opus entry). A model-only pin selects the harness that owns the model instead of defaulting to Codex.
- A resolved model pin is honored even when its tier differs from `capabilityTier`. The routed request carries the pinned model's real tier, so capability-unit accounting stays honest.
- A harness-only pin picks that harness's default model for the requested tier, then the cheapest higher tier it has.
- The tier route is the fallback. A pin that names nothing installed, or a model that is not selectable, is dropped with a note and the request routes by tier. A pinned candidate that is unavailable, quota or context exhausted, or user-excluded is replaced by what the router would have chosen without the pin, then by the best eligible candidate.
- A below-floor candidate admitted only because it was pinned is never picked by the recommendation, the published policy preference, or retry escalation.
- A pin never overrides a hard gate: the permission ceiling still fails with an actionable error, and a verification pin on the implementer's harness family is replaced by a different-family verifier.
- Every pin resolution, drop, and fallback is recorded in the router decision's explanation, which the `worker.route.selected` event carries.
- The orchestrator's launch-time session context lists installed harnesses and their selectable models by tier, marking each tier default, so it can pick a pin without guessing ids.
- The fleet digest reports each live worker's harness and model, so the orchestrator can see whether a pin was honored.

## Unit Tests

- `learning_router::tests::a_model_only_pin_selects_the_harness_that_owns_it`
- `learning_router::tests::a_model_pin_is_honored_across_tiers`
- `learning_router::tests::model_pins_resolve_aliases_and_labels`
- `learning_router::tests::an_unknown_model_pin_falls_back_to_the_tier_route_with_a_note`
- `learning_router::tests::an_uninstalled_harness_pin_falls_back_to_the_tier_route`
- `learning_router::tests::a_harness_only_pin_uses_that_harness_default_for_the_tier`
- `learning_router::tests::an_unusable_pin_falls_back_to_the_unpinned_route_first`
- `learning_router::tests::a_below_floor_pin_is_never_recommended_without_the_pin`
- `learning_router::tests::a_verification_pin_on_the_implementer_family_is_replaced`
- `learning_router::tests::routing_inventory_lists_selectable_models_by_tier`
- Existing router tests stay green unchanged: user exclusions, permission ceiling, quota substitution, and outcome attribution.

## Prompt Tests

- The orchestrator briefing no longer forbids routing fields. It documents `harness` and `model` as optional pins with the tier as the fallback.
- `prompts::tests::required_marker_lint_agrees_with_the_default_briefing` stays green.

## Smoke Tests

- `bun run build` succeeds.
- `bun run test` succeeds.
