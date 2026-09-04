# feat/issue-479-model-catalog — Test Contract

## Functional Behavior

- Every harness catalog is represented by one normalized model shape with explicit availability, compatibility, lifecycle, source, and Standard-promotion metadata. Catalog ordering is deterministic regardless of discovery order.
- Authoritative runtime/API discovery wins when it returns a valid non-empty catalog. Harnesses without discovery use normalized curated fallbacks rather than provider-specific UI exceptions.
- Discovery output is normalized by trimming invalid entries, deduplicating identifiers, and refusing to promote preview, beta, deprecated, inaccessible, or incompatible models.
- Availability is independent from promotion: an available model may be selectable without being the promoted default, and only one eligible model per capability tier is promoted deterministically.
- A profile explicitly selects either `track_standard` or `pinned`. Tracking profiles resolve against the current promoted stable model for their tier without rewriting the stored profile version. Pinned profiles continue to resolve their stored provider/model and never silently move to a new model.
- Existing persisted `pinned = 1` rows migrate to `pinned`; existing `pinned = 0` rows migrate to `track_standard`. New recommended profiles track Standard by default.
- If a pinned model becomes unavailable, existing fallback-purpose behavior remains available; the pin itself is not mutated. If a tracking profile cannot resolve its preferred provider/tier, deterministic catalog fallback behavior applies.
- A successful discovery snapshot is cached atomically as last-known-good data with a bounded entry count, byte size, freshness TTL, and maximum stale age. A refresh failure reuses a valid last-known-good snapshot and reports stale/error diagnostics. Missing, corrupt, oversized, mismatched, or expired cache data falls back to the curated catalog and never replaces a known-good cache with an empty/failed discovery.
- Catalog descriptors expose source/freshness/failure diagnostics so callers can distinguish live discovery, last-known-good data, and curated fallback.
- Managed harness/runtime dependency pins have an automated update path. Compatibility checks verify exact pins, lockfile integrity, compiled constants, and sidecar/package manifests stay synchronized.

## Unit Tests

- `model_catalog::tests::normalization_is_deterministic_and_deduplicated` — unordered/duplicate discovery records produce a stable unique catalog.
- `model_catalog::tests::promotion_requires_stable_available_compatible_models` — preview, deprecated, inaccessible, and incompatible entries remain listed but are never promoted.
- `model_catalog::tests::newer_stable_discovery_can_become_the_standard` — a newly discovered eligible stable model with higher promotion precedence becomes the promoted tier model.
- `model_catalog::tests::failed_discovery_uses_bounded_last_known_good_then_fallback` — fresh/stale-within-bound cache is retained on failure; over-age/corrupt/oversized data uses safe curated fallback and failed discovery never erases the cache.
- `model_catalog::tests::cache_round_trip_is_atomic_and_bounded` — cache serialization enforces source, entry, and byte bounds and round-trips deterministically.
- `model_profiles::tests::tracking_profile_follows_new_standard_without_persisted_rewrite` — a tracking profile resolves a newly promoted model while the stored provider/model/version remain unchanged.
- `model_profiles::tests::pinned_profile_survives_standard_promotion` — an explicit pin continues to resolve its concrete model after promotion.
- `model_profiles::tests::legacy_pinned_boolean_migrates_to_explicit_selection_mode` — schema migration maps legacy rows deterministically.
- Frontend `modelProfiles` tests — available options include non-promoted available models, tracking drafts are explicit, profile comparison/persistence preserves selection mode, and pinned resolution does not drift.
- Managed runtime compatibility tests — built-in constants match exact dependency manifests/lockfiles, including the Claude sidecar package pin.

## Integration / Functional Tests

- Adapter descriptors consume normalized catalogs: curated built-ins and OpenCode runtime discovery use the same promotion policy and wire shape.
- OpenCode refresh success replaces the cached snapshot; refresh failure retains last-known-good selectable models and surfaces stale/error metadata.
- Model-profile save/load round-trips `selectionMode`, including migration of a pre-change database, and session/router resolution observes tracking-vs-pinned semantics.
- Generated Rust/TypeScript protocol artifacts agree on the normalized model/catalog and profile-selection fields.
- CI runs the built-in compatibility report and managed runtime/catalog compatibility test targets when dependency updates are proposed.

## Smoke Tests

- `bun run build` completes with generated protocol types and the Model settings UI.
- `bun run test` passes all frontend, sidecar, and Rust tests.
- A mock catalog exposes selectable non-default models while showing exactly one promoted eligible default per populated tier.
- Saving a pinned profile and refreshing to a new Standard leaves the pin unchanged; saving a tracking profile makes the new Standard effective.

## E2E Tests

- N/A — provider account/network availability is nondeterministic in CI. Runtime discovery, cache failure, profile persistence, and promotion are covered through deterministic adapter/catalog integration tests and fixtures.

## Manual / cURL Tests

- In Settings → Models, choose **Track standard**, save, refresh a catalog containing a newer stable compatible model, and verify the effective model changes without a new stored profile version.
- Switch the same profile to **Pinned model**, select a concrete model, save, refresh again, and verify the selection is unchanged.
- Disconnect discovery/network access after one successful OpenCode refresh; restart and verify models remain selectable with last-known-good/stale diagnostics rather than disappearing.
- N/A for cURL — discovery runs against local harness transports and the desktop command boundary, not a public HTTP endpoint.
