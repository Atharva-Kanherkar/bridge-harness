# Provider limit detection and cooldown contract (issue #572, slice 2)

- A provider usage limit is recognised from the adapter's own `error` frame, not from the worker's prose. Codex ("You've hit your usage limit"), Claude (`rate_limit_error`, "usage limit reached") and OpenCode (429) all produce the same typed signal.
- Detection is independent of session kind and result status: a chat, a working worker, a failed worker and a worker that already settled `protocol_invalid` all write the cooldown.
- The cooldown ends when the provider says it ends. A parsed reset hint sets `cooldown_until`; with no hint, a floor much longer than one turn applies. A hint already in the past is ignored in favour of the floor.
- Compaction against a harness in cooldown is refused with the reset time, not retried. The repeated "Error running remote compact task" turns are the failure this prevents.
- The cooldown row and its `router.harness_quota_exhausted` event carry the harness, the signal, and the reset time.

Verification: replay real Codex, Claude and OpenCode limit frames through the live handler and assert a cooldown row with no `assistant.message` and no worker result present; assert reset-hint parsing including past hints and absent hints; assert the failed-lifecycle worker path writes it where slice 1's early return used to skip it; assert compaction refusal during cooldown. Run `bun run build` and `bun run test` before opening the PR.
