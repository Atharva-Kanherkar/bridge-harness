# Issue 39 Contract: Deliberate provider-orphan recovery

- Every Codex/Claude runtime is launched in a dedicated process group and records its leader PID plus an OS-derived identity marker on the owning session before it is considered managed.
- Normal shutdown clears the persisted process claim; supervisor startup audits every remaining claim before accepting new work.
- Startup kills a process group only when the live leader identity exactly matches the persisted marker, avoiding PID-only termination of an unrelated process.
- A matching orphan is terminated before its session is marked recoverably failed; a missing or identity-mismatched leader is not killed but the stale claim is cleared and audited.
- Active orchestrators/chats receive a durable recoverable adapter-failure record. Workers additionally flow through the existing typed failed-result reconciliation so their parent is released and restoration can continue later.
- Mid-turn recovery clears `active_turn_id` and explicitly warns that worktree changes may be partial; it never claims a successful checkpoint.

## Local refund checkpoint

Verify tracked identity persistence/clearing, matching kill versus mismatch refusal, active root failure, typed worker reconciliation, partial-write warning, migration, full frontend/Rust suites, type checks, and `git diff --check` before the issue-only PR.
