# feat/bounded-evaluator — Test Contract

Issues #214 (P0 evaluator truthfulness, Phase D) and #215 (bug 5, Phase 3). Locked
before implementation.

## The shape of the problem

The learning stack was designed around a bounded model evaluator and shipped without
one. `record_deferred_model_evaluations` finds every outcome whose `success_state` is
`unknown`, picks an evaluator profile from a different provider family than the one
that did the work, and then writes a `routing_evaluations` row with status
`pending_bounded_model_eval` and no score. Nothing ever executes it. The learning
report hardcodes `evaluated_spend_microusd = 0` and `evaluated_tokens = 0`, and the
row it wrote sits in the table forever.

Downstream, `record_outcome` assigns `confidence_bps` from three constants: test-backed,
unknown-acceptance, no-signal. Most real delegations have no automated test, so most
land on the middle constant, and the learning job's confidence gating grades almost
everything the same. `routing_policy::load_evidence` already prefers a completed
`model_based` evaluation's score and confidence over the outcome's own — it has simply
never found one.

This slice writes the executor. Nothing about who may run, what a policy may promote,
or what the router is allowed to select changes.

## Functional Behavior

### A queued evaluation is a run, not a note

- A decision with `success_state = 'unknown'` and an eligible evaluator profile enqueues
  one evaluation run in status `queued`, and its `routing_evaluations` row carries the
  same status rather than `pending_bounded_model_eval`.
- The lifecycle is exactly `not_requested`, `queued`, `running`, `completed`, `failed`,
  `skipped`. A run leaves `queued` only by being claimed, and leaves `running` only by
  settling. Settling twice is refused and the first settlement stands.
- With no eligible evaluator profile the row is `not_requested`, never `queued`. Turning
  evaluation off settles queued rows `skipped` rather than executing them.
- A run is claimed under a lease with an expiry. An expired lease is reclaimable; a live
  one is not. Two concurrent claims yield at most one claimed run.
- A decision already carrying a completed independent-verifier evaluation is not queued.
  Reused existing evidence stays reused.

### The evaluator never sees a transcript

- Evidence is built deterministically from the decision and its outcome: the acceptance
  criteria, per-check statuses, files-changed count, runtime, retries, override signal,
  reported cost and tokens, and the bounded artifact ids the deterministic evaluation
  already recorded. Session entries are not read.
- Nothing the worker said about its own work reaches the bundle, and neither does the
  acting harness or model name. A self-report and a model identity both move a judge,
  so the judged artifact carries only what Bridge and the check executors recorded.
  Cross-family selection still needs the acting family; the bundle does not.
- The prompt is ordered so the untrusted half cannot become instruction: the acceptance
  criteria first, then the evidence as one JSON-encoded object, then the rubric and the
  answer contract last.
- The evidence is capped, and its SHA-256 is recorded on the run so a verdict can be
  tied to exactly the bytes that produced it.
- The same decision produces byte-identical evidence on repeat builds.

### The judge is bounded, tool-free, and cross-family

- The evaluator profile chosen for a decision is never from the same provider family
  that produced the outcome. A workspace whose only profile matches the acting provider
  produces `not_requested`, not a same-family judge.
- The run executes in a hidden session that no surface lists, compiled with an empty
  tool scope so every tool call is denied, with one turn, no repair, and a wall-clock
  cap. Exceeding it settles the run `failed`.
- Observed tokens and spend are recorded on the run and summed into the learning
  report. The report's evaluated spend and tokens are the observed values, never zero
  when a run executed.

### The gate owns the verdict

- The model answers with a fenced block tagged `bridge-outcome-verdict` containing a
  JSON object with exactly the fields `criteria` and `confidenceBps`. `criteria` holds
  every id in the closed rubric set exactly once, each an object with exactly `id`,
  `verdict`, and `evidence`, where `verdict` is `pass`, `fail`, or
  `insufficient_evidence` and `evidence` is a short span quoted from the bundle.
- The model never states a score. The gate derives `score_bps` from the pass and fail
  verdicts alone; criteria answered `insufficient_evidence` leave the denominator
  rather than counting against the work, and cap the recorded confidence in proportion
  to how much of the rubric was actually decided.
- The last fenced block wins, so an earlier one echoed out of the evidence cannot.
  An unknown id, a duplicate id, a missing id, an unknown verdict, an extra field, a
  missing block, a confidence outside 0-10000, or an evidence span over its cap is a
  protocol deviation: the run settles `failed` and writes no score.
- A verdict in which every criterion is `insufficient_evidence` settles `skipped` and
  writes no score. Neither a failed run nor an undecidable one is ever recorded as a
  zero, because a zero is a judgement about the work and neither of those is.
- A settled `completed` run writes `score_bps` and `confidence_bps` onto the
  `model_based` row for that decision with status `completed`, and records the
  evaluator identity, the evidence digest, and that tool access was none.
- A verdict never changes an outcome's `success_state` or `acceptance_state`. It is
  evidence for confidence, not a re-decision about what happened.

### Nothing pending counts as evidence

- `routing_policy::load_evidence` picks up a completed model evaluation's score and
  confidence, and ignores `queued`, `running`, `failed`, `skipped`, and
  `not_requested` rows entirely.
- A policy cannot be promoted on the strength of a run that has not completed. A
  learning run whose evaluations are all still queued reports the same replay and
  promotion outcome it would have reported with no evaluator configured at all.

### Confidence stops being three constants

- With a completed model evaluation for a decision, the confidence the learner uses is
  the evaluated confidence. Without one, the existing constants apply unchanged, so a
  workspace with evaluation disabled behaves exactly as it does today.
- A test-backed outcome keeps its test-backed confidence: a judge does not get to lower
  the confidence of a result that has a passing or failing test.

## Determinism

No test performs a model call. The executor is written against an `EvaluationModel`
trait that tests implement with fixed strings, matching `ExtractionModel`. Leases,
claims, and settlement are asserted through explicit timestamps passed in, never by
sleeping.

Bridge's own half is deterministic: the same decision builds the same bytes, and the
same verdict derives the same score. The model's half is not. Whatever sampling
parameters a run was given are recorded beside its verdict, and a score is treated as
a measurement with error rather than a fact about the work.

## Out of Scope

- Any change to eligibility gates, sandboxing, budgets, pins, or exclusions.
- Routing evaluation runs through the learning router. This slice runs them on the
  configured evaluator profile, as extraction already does.
- Memory consolidation, TTL, and conflict adjudication (#214 Phase D remainder).
- Every UI surface. The wire methods this adds are read and settings only.
