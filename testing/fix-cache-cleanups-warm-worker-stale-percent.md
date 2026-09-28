# Warm worker prompt reuse and context-pressure resets

## G8: reuse the compiled variable suffix

A compatible worker with a live provider thread already holds the stable prompt.
Its next task sends `CompiledPrompt.variable_suffix` only: objective, acceptance
criteria, evidence and task constraints. It must not resend `<bridge-stable-prompt`.
The compatibility check still requires the same harness, model, prefix hash and
prompt schema. Fresh launches and incompatible-prefix restarts retain the full
compiled instructions.

## G11: retire the previous context gauge at a switch

A committed model or harness switch clears the session's displayed context gauge
and records the current maximum usage-ledger ID in `sessions.context_usage_after_id`.
A launch that binds a different provider thread or resolves a different model
does the same; resuming the same thread and model preserves the existing gauge. Pressure compaction
considers only later ledger rows that report `context_percent`.

Historical ledger rows remain unchanged. A watermark avoids timestamp ties and
does not assume that a requested model equals a rerouted serving model. A switch
that loses its revision guard, or whose transaction rolls back, leaves the old
pressure boundary intact. After a successful switch, a turn without a reported
gauge cannot reuse the old percentage; a new low gauge does not compact and a new
high gauge can compact normally.

## Automated acceptance

- Warm-worker delivery captures the actual next task sent to its live runtime and
  verifies the variable task data is present while the stable envelope is absent.
- The model-switch regression exercises native continuation, a new thread on the
  same harness, and a cross-harness switch, including stale plans and rollback.
- The schema upgrade regression preserves old ledger data and persists the new
  boundary across database reopen.

The live prefix-cache experiment (G12) is outside these deterministic fixes.
