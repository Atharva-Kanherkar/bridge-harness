# fix/router-debts

The learning-router debts locked out of the memory epic, one commit-sized fix
and one regression test per bug. Replay fixtures re-run after every change
that moves live routing numbers; the policy-replay contract already forbids a
fabricated improvement, and this stack must not create one.

## Dead fingerprint keys

- Policies stop publishing per-fingerprint preferred candidates; only role
  family keys generalize, so only they are published. The fingerprint stays
  for deterministic canary bucketing and evidence grouping.
- Existing stored weights are migrated: non-family preferred keys are dropped.
- Test: a promoted policy's preferredCandidates contains only role families.

## Time decay

- Online history weights outcomes by age with an exponential half-life;
  offline evidence gains the same recency cutoff beside its row cap. Fresh
  evidence outweighs stale volume: 200 old samples lose to 5 fresh ones.
- Test: stale_history_decays_out_of_the_prediction.

## Lateral escalation

- After a failure, escalation tries a same-tier different-family candidate
  before spending a tier. Still deterministic, still terminal after strong,
  still advisory to policy. If no production caller lands in this stack, the
  function does not survive as a prettier unused helper.
- Test: same_tier_family_switch_is_tried_before_tier_up.

## Honest confidence

- The three buckets get names. Unknown-acceptance stops passing the learning
  gate: eligibility requires confidence above the unknown-acceptance bucket,
  keyed to the named constant — no new number is invented. The evaluator
  executor remains the only future source of a real score.
- Test: unknown_acceptance_does_not_pass_the_learning_gate.

## Catalog normalization

- Decisions store a catalog hash; the catalog body lives once per version in
  a catalogs table. The decision blob no longer embeds the snapshot, and
  evidence loading stops deserializing it. Old rows keep their inline
  snapshots as history; new rows never write one.
- Test: decision_rows_store_a_catalog_hash_not_blobs, with a size assertion.

## Named, tunable constants

- The load-bearing learning constants (autonomy threshold, prior weight,
  decay half-life, evidence window, confidence floor) become named values in
  one place with documented defaults, read per workspace from a tunables row
  when present. Defaults are unchanged; validation rejects nonsense ranges.

## Input honesty

- The minimum-pass field keeps draft state and commits on blur; an emptied
  field cannot commit a zero floor. LearningRunSummary renders a real
  "No learning runs yet" empty state instead of a lone button.

## Out of scope

Fingerprint clustering, the evaluator executor, router-routed memory jobs,
sideways escalation UI, splitting the router dialog, and any policy-weight
change beyond dropping dead keys.
