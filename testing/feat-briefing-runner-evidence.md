# feat/briefing-runner-evidence — Test Contract

Issue #207, slice 4 of 7 under epic #203. Locked before implementation.

Slices 1–3 are merged: the wire contracts and migration-24 tables, the briefing
authority and its conformance suite, and the facts UI. This slice produces
*validated* briefing runs. Reconciling task state into the board is slice 5, so
nothing here writes `work_tasks`.

## The shape of the problem

Everything a briefing run reads is untrusted, and the thing reading it is a language
model. So the run has two jobs beyond producing output: **prove where each claim came
from**, and **refuse output it cannot prove**.

Two rules fall out of that, and most of this contract is their consequences:

1. **Provenance is Bridge-derived, never model-authored.** The model may say *which*
   evidence backs a task, by reference. It may not say what that evidence is, where it
   came from, when it was observed, or what URL it points at.
2. **An evidence reference is only real if this run earned it.** A successful call, to
   a reviewed tool, in *this* run. A reference to another run's evidence, to a failed
   call, or to nothing at all invalidates the payload.

## Functional Behavior

### Briefing configuration is optional and outside the purpose set

- `ProfilePurpose` keeps its nine variants. Briefing configuration references a
  provider, model, and effort directly and is `Option`al — there is no tenth required
  purpose, because profile validation demands exactly one profile per purpose and a
  briefing that nobody configured must not make the profile set invalid.
- Absent configuration is `not_configured`, which is a state, not an error.
- Configuration naming an unknown provider, or a provider that
  `briefing_policy::certify_briefing` refuses, fails explicitly with a stable code.
  There is no fallback to another provider — a briefing on a provider the user did not
  choose is a different run.

### Only reviewed connectors reach the model

- A connector instance is offered only when it has (a) at least one reviewed exact
  tool identity, (b) a recorded tool-definition digest matching what the provider
  presents now, and (c) a deterministic evidence resolver for its family.
- A tool-definition digest that differs from the reviewed one makes that tool
  ineligible until re-reviewed. Drift narrows authority; it never widens it.
- A connector family with no resolver is `Ineligible` and never offered, however many
  tools it has.

### Evidence is earned, and its provenance is Bridge's

- An `evidenceRef` exists only for a call that (a) was allowed by the policy, (b)
  belongs to this run, and (c) returned successfully.
- **The provider is told each reference as its call returns.** A brief names evidence by
  reference, so a model that had to guess the reference format would be authoring
  provenance by the back door — and a test that predicted the format would be testing the
  format rather than the run. The answer is therefore asked for *after* the calls.
- For each such call Bridge derives and stores: the canonical resource id, the
  connector instance and account identity, the observed time, the tool-definition
  digest, a digest of the result, and a safe target.
- The canonical id comes from **one** field per family, with no fallback to a generic
  `id`. Two different fields are two different grains of identity, so a fallback could
  mint two ids for one resource and slice 5 would treat it as two things. A result
  without that field earns nothing rather than an id nobody can match again.
- A target is only an external link when Bridge resolved it *and* matched its host
  against the connector's allowlist. Anything else is a Bridge-local target or none.
- Raw connector payloads are never stored. Only digests and Bridge-derived identity.

### Source coverage is six separate facts

`Ineligible`, `Eligible`, `Consulted`, `Succeeded`, `Failed`, `AuthRequired` are
tracked independently, one row per connector instance per run. "Available" is not
"was read", and a run that consulted three sources and succeeded on one must be
readable as exactly that.

### The parser is strict, and repairs exactly once

- Exactly **one** fenced ```` ```bridge-work-brief ```` block. Zero is
  `schema_invalid`; two or more is `schema_invalid` — picking one would be guessing.
- The payload is one JSON object with a `version` the build knows. Unknown fields are
  rejected, not ignored.
- Ranks are `1..=n` with no gap and no duplicate. At most **12** tasks.
- `title` and `why` are bounded; `confidenceBps` is `0..=10000`.
- Every `evidenceRef` resolves to evidence this run earned. A wrong-run, failed-call,
  duplicate, or unknown reference invalidates the payload.
- On failure, **one** bounded repair turn is attempted, and only one. The bound is read
  from `MAX_REPAIR_ATTEMPTS` by the function that enforces it, so the constant governs
  rather than describes — a constant the code ignores is a claim, not a limit. If the repair
  also fails, the run ends `failed` with a stable non-sensitive code, and the
  previously committed board is left exactly as it was.

### The briefing session is hidden

- The run happens in a workspace-less session with `kind = 'briefing'`.
- It is excluded from the sidebar, Mission Control, default session selection, and
  normal composer routing — by an explicit predicate, not by happening to sort last.
- Run usage and source coverage are readable without selecting that session, because
  the diagnostic transcript is for **Inspect run** and nothing else.

## Unit Tests

`work_brief_parser`:

- `exactly_one_fence_is_required` — zero and two both fail.
- `an_unfenced_payload_is_not_accepted`
- `unknown_fields_are_rejected_rather_than_ignored`
- `an_unknown_schema_version_fails_closed`
- `ranks_must_be_dense_and_unique` — gap, duplicate, zero, negative.
- `at_most_twelve_tasks`
- `oversized_title_or_why_is_rejected`
- `confidence_outside_basis_points_is_rejected`
- `every_evidence_reference_must_resolve`
- `a_reference_to_another_runs_evidence_is_rejected`
- `a_reference_to_a_failed_call_is_rejected`
- `a_duplicate_reference_within_one_task_is_rejected`
- `a_valid_payload_parses_with_every_field_bridge_derived`
- `the_model_cannot_author_a_target_or_an_observed_time` — supplying them is an
  unknown field.
- `exactly_one_repair_is_attempted`
- `a_failed_repair_preserves_the_previous_board_and_records_a_code`
- `a_failure_code_is_stable_and_carries_no_payload_text`

`work_connectors`:

- `a_connector_without_a_resolver_is_ineligible`
- `a_changed_tool_definition_digest_makes_the_tool_ineligible`
- `only_reviewed_exact_tools_are_offered`
- `a_target_outside_the_connector_allowlist_is_refused`
- `the_resolver_never_invents_a_session_target` — a Bridge-local target is not
  something a connector result can talk Bridge into. (The original name assumed a
  resolver path that produces one; none exists until slice 5, so asserting it here would
  have been a test of nothing.)
- `a_target_keeps_its_kind_across_the_json_the_store_writes`
- `the_canonical_resource_id_is_derived_not_taken`

`work_evidence` ledger:

- `only_a_successful_allowlisted_call_earns_a_reference`
- `a_failed_call_earns_nothing`
- `a_denied_call_earns_nothing`
- `references_are_unique_within_a_run`
- `the_ledger_stores_digests_and_never_a_payload`
- `evidence_is_scoped_to_its_run`

Source coverage:

- `the_six_states_are_tracked_independently`
- `consulted_is_not_succeeded`
- `an_auth_required_source_is_not_a_failure`
- `coverage_is_one_row_per_connector_per_run`

Briefing session and configuration:

- `briefing_configuration_is_optional_and_adds_no_profile_purpose`
- `an_unconfigured_briefing_is_not_configured_not_an_error`
- `an_uncertified_provider_fails_with_a_code_and_no_fallback`
- `a_briefing_session_is_workspace_less_and_kind_briefing`
- `keeps a briefing session out of every surface that filters through here` — asserted
  on the shared predicate, which the rail, Mission Control and default selection all read
  through, plus a wiring test that checks the **props** rather than string-matching the
  filter's presence.
- `run_usage_and_coverage_are_readable_without_selecting_the_session`

Adversarial:

- `a_connector_result_asking_for_another_tool_is_not_followed`
- `a_connector_result_claiming_its_own_provenance_is_ignored`
- `a_model_fabricated_evidence_reference_fails_the_payload`

## Integration / Functional Tests

- A whole run over a stubbed provider: eligible sources offered, one consulted and
  succeeded, one failed, a valid payload parsed, evidence stored, coverage recorded,
  and the run marked `succeeded` — with `work_tasks` still empty, because committing
  tasks is slice 5.
- The same run with an invalid payload: one repair, then `failed` with a code, and
  no evidence or coverage lost.
- `work_fact_cache` and the facts board are untouched by any of it.

## Smoke Tests

- `cargo test --manifest-path src-tauri/Cargo.toml --workspace` green.
- `npx vitest run` green.
- `bun run build` green.
- `npm test --prefix sidecar/claude-agent` green.
- Regenerating protocol artifacts leaves the tree clean if no wire type changed; if
  one did, the regenerated artifacts are committed in the same step.

## E2E Tests

N/A — there is no briefing UI in this slice and no browser-driven harness in this
repo. The integration test above is the closest equivalent and is run in full.

## Manual / cURL Tests

No HTTP surface.

```bash
cargo test --manifest-path src-tauri/Cargo.toml -p bridge-core work_brief -- --nocapture
```

```bash
cargo test --manifest-path src-tauri/Cargo.toml -p bridge-core work_evidence work_connectors
```
