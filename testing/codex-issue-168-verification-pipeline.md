# codex/issue-168-verification-pipeline — Test Contract

Locks #168, the P3 and final foundation of the marketplace epic #171: "Bridge
Verified" and "update available" become consequences of reproducible evidence
rather than an upstream release event or a manual label.

Builds directly on #164/#166 (#187), which *modelled* `VerificationStatus` and
refused to load an entry that is not `Verified` — while deliberately having no
way to produce that status. This PR is the producer. It also builds on #161's
managed lifecycle (#174/#176) for installing a candidate, and on #163's
`BackendResolver` for the backend a candidate is verified against.

## Scope And Stated Assumptions

- **Evidence is the only way to `Verified`.** The central claim, and it is made
  structural rather than procedural: `Verification::verified` is the sole
  constructor that can produce `VerificationStatus::Verified`, and it takes an
  `&Evidence` it cannot fabricate. `Verification` loses its public field
  literal — a test asserts no code path outside this module can hand-write a
  verified verdict. #164 gated *loading* on the status; this gates the status
  itself on evidence.
- **`evidence_ref` becomes the evidence digest.** #164 called it "an opaque
  reference, deliberately not a path or a URL: this must not become a fetch
  instruction." A content digest keeps that property and adds one: the
  reference can be *checked* against the evidence rather than merely quoted.
  A verdict whose `evidence_ref` does not equal the digest of the evidence
  offered with it is refused at promotion.
- **Detection is an input, never an authority.** A `Candidate` — an agent, a
  version, a recipe, and the approved source that suggested it — is what step 1
  produces. There is no conversion from `Candidate` to `VerifiedEntry`, in
  either direction, asserted structurally the same way #164 asserts it for
  `RegistryAgent`. An upstream release event moves a candidate into the queue
  and nothing further.
- **The suite runs against the #166 control plane, not beside it.** Every check
  drives an `AgentIntegration` through `IntegrationSession` — the same launch,
  turn, permission, interrupt, resume, and shutdown surface a real session
  uses. A suite with its own private path to the runtime would verify a path no
  user ever takes.
- **Flakiness is a property of the suite, not of a check.** A required check
  that does not return the same outcome across repeated runs blocks promotion
  even when its final run passes. Determinism is asserted by running the
  required set `DETERMINISM_RUNS` times and comparing outcomes, because "it
  passed" and "it passes" are different claims and #168 names the second.
- **No new marketplace agent, and no real vendor.** Per #168's own acceptance,
  the pipeline is demonstrated by fake integrations. `claude`, `codex`, and
  `opencode` keep their hand-written adapters; `builtin_compatibility` gains no
  field and `builtin-compatibility-report-v1.json` stays byte-identical.
- **Vendor-authenticated smoke tests are modelled, not executed.** A check may
  declare that it requires vendor credentials; without them it reports
  `Skipped { vendor_auth_unavailable }` and is *not* counted as passing. A
  required check that could be skipped into a pass would make the whole gate
  optional in exactly the environment where it matters least.
- **Evidence carries no secret and no transcript.** No field can hold a token,
  a key, a home path, a vendor configuration value, or provider output beyond a
  bounded, redacted failure reason. Two tests enumerate every field of every
  evidence type, in the same style as #164's catalog enumeration.
- **No RPC method and no CI entry point.** The pipeline is reachable through
  `bridge_core::api`; a `verify-agent` binary and a workflow that holds vendor
  credentials belong with the CI wiring that has somewhere to put a secret.
  Recorded so its absence reads as a decision, matching how #187 recorded the
  catalog's missing RPC method.
- Out of scope: AI-assisted anomaly triage, which #168 explicitly bars from
  being a promotion oracle; the marketplace UI; and adding any specific agent.

## Functional Behavior

### Evidence (#168 steps 2, 6)

- An `Evidence` document is immutable once built and binds a verdict to exactly
  what produced it: `agent`, `version`, `backend`, `backend_kind`, the
  `artifact_digest` the managed lifecycle actually installed, `bridge_version`,
  `platform`, `suite_version`, the ordered `checks` with their outcomes, and
  `produced_at`.
- `Evidence::digest` is a SHA-256 over the canonical JSON encoding. Canonical
  means field order fixed by the struct and no incidental whitespace, so the
  same run on the same inputs produces the same digest — a digest that moved
  with serializer settings would make reproducibility unfalsifiable.
- Evidence is never partially valid: a document missing any required check, or
  carrying a check the current suite version does not define, is refused rather
  than read leniently.

### The conformance suite (#168 steps 4, 5)

- `SUITE_VERSION` is a compiled-in constant. A verdict produced under a
  different suite version is not comparable and is refused at promotion rather
  than silently accepted.
- The required checks cover #168's list: install, launch, initialize, new
  session, multi-turn prompt, normalization totality, permission reporting,
  cancellation, shutdown, claimed resume behaviour, vendor-auth-required
  failure, redaction, update from the prior verified version, rollback,
  uninstall with history retained, platform claims, and capability drift.
- Every check reports `Passed`, `Failed { reason }`, or
  `Skipped { reason }`. Only `Passed` counts toward a gate.
- `normalization` is checked for *totality* against a recorded frame stream —
  every frame normalizes or is explicitly reported unknown — reusing the
  built-in event vocabulary read out of `agent.rs`, exactly as #187's
  `a_fake_integrations_events_are_shaped_like_a_built_in_adapters` does.

### Promotion (#168 steps 7, 8)

- `promote` takes the catalog in force, a `Candidate`, and its `Evidence`, and
  returns either a new `CatalogSnapshot` or a typed `PromotionBlocked`.
- Promotion is blocked when: any required check did not pass; a required check
  was skipped; the run was not deterministic; the evidence digest does not
  match the verdict's `evidence_ref`; the artifact digest does not match what
  the recipe declares; the evidence's bridge version, platform, backend, or
  agent does not match the candidate; the suite version is not the current one;
  the candidate's version is in its own blocked list; or the candidate version
  does not advance the entry it would replace.
- A promoted snapshot advances the generation by exactly one and leaves every
  other entry byte-identical — a promotion that rewrote unrelated entries would
  make the catalog's generation meaningless as a rollback point.
- `roll_back` returns the catalog to the previous generation's entry for one
  agent, recording the reason. Rollback is per-entry, not per-snapshot: a health
  failure in one agent must not withdraw every other agent verified in the same
  snapshot.

## Unit Tests

### Evidence

- `only_evidence_can_produce_a_verified_status` — enumerated structurally:
  `Verification` has no public literal construction, and `verified` is the only
  function returning `VerificationStatus::Verified`.
- `evidence_digest_is_stable_across_runs` — the same inputs produce the same
  digest; changing any single field changes it.
- `a_verdict_whose_evidence_ref_does_not_match_is_refused`.
- `evidence_carries_no_credential` and `evidence_carries_no_transcript` — every
  field of every evidence type enumerated; failure reasons are bounded and
  redacted.

### The suite

- `every_required_check_from_the_issue_is_present` — the required set is
  compared against the list #168 names, so dropping one is a test failure rather
  than a quiet reduction in coverage.
- `a_skipped_required_check_is_not_a_pass` — including the vendor-auth case.
- `a_flaky_required_check_blocks_promotion` — a check that alternates outcomes
  across runs is caught by the determinism gate even when its last run passes.
- `the_suite_drives_the_same_control_plane_a_session_does` — asserted
  structurally: the suite reaches the runtime only through `IntegrationSession`.

### Promotion

- `a_simulated_upstream_release_cannot_reach_users_without_evidence` — #168's
  headline acceptance: a candidate detected from an approved source, with no
  evidence, cannot become a served entry by any public path.
- `a_tampered_artifact_blocks_promotion` — the installed digest differs from the
  recipe's declared integrity.
- `capability_drift_blocks_promotion` — reusing #166's `check_capabilities`
  rather than restating the rule.
- `a_regression_against_the_prior_verified_version_blocks_promotion`.
- `promotion_advances_the_generation_by_one_and_touches_nothing_else`.
- `a_rollback_withdraws_one_entry_and_leaves_the_rest_served`.
- `a_promoted_snapshot_still_passes_catalog_validation` — the output of
  promotion faces exactly the validation a remote snapshot faces, so the
  pipeline cannot mint a document the catalog would refuse.
- `candidates_cannot_become_verified_entries` — no constructor, `From`, or
  conversion, asserted over the module's API.

## Integration / Functional Tests

- End to end over the seam: a candidate is detected, installed through the
  managed lifecycle against a fake fetcher, driven through the full suite on a
  fake integration, produces evidence, promotes into a signed snapshot, and that
  snapshot installs into a `CatalogStore` and serves — the #164, #166, and #168
  seams meeting.
- `cargo test --manifest-path src-tauri/Cargo.toml --workspace` passes.
- `scripts/check-builtin-adapters.sh` stays green and
  `testing/fixtures/builtin-compatibility-report-v1.json` is unchanged.
- Generated schema and TypeScript artifacts regenerate to a zero diff; no method
  or wire DTO changes here.
- `bun run build` and `bun run test` pass.
- `cargo clippy --workspace --all-targets` reports nothing new on touched files.

## Smoke Tests

- Run the pipeline over a fixture candidate and confirm the evidence document
  contains no absolute home path, no environment value, and no provider text
  beyond a bounded redacted reason.
- Promote a fixture candidate, then roll it back, and confirm the catalog serves
  the prior version for that agent and the current version for every other.

## E2E Tests

N/A for desktop E2E: no user-facing surface lands here. The pipeline's UI
arrives with the marketplace screen, and its CI entry point with the workflow
that holds credentials.

## Manual / cURL Tests

- N/A for cURL: this PR adds no method to the socket.
- Manually inspect a produced evidence document and confirm it is readable,
  bounded, and free of anything a reviewer would not want in a public artifact.
