# Background activity visibility: test contract

## Functional Behavior
- Claude task start, progress, background transitions, membership snapshots and terminal notifications produce inspectable normalized activity with stable task identity.
- A background launch acknowledgement is not a successful test result. Actual completion, failure and stop remain distinct.
- A persistent summary at the latest reply lists active commands and background processes, their progress, and whether the model is working. It stays visible when the original command row is offscreen, stops when the process finishes, and is hidden in read-only history.
- Background work remains visibly running after the assistant turn ends; ordinary unfinished tools still stop claiming liveness at turn end.
- Membership snapshots reconcile missed terminal events without inventing success. Runtime exit and restart clear stale background liveness.
- Task progress and available output are visible while a group is collapsed. Select a running call when another parallel call already completed.
- Synthetic task notifications, including Monitor updates, remain inspectable rather than disappearing as user echoes. Housekeeping and ambient watchers do not count as active work.
- All harnesses use the same normalized rendering. Guidance prefers tracked commands and native background tasks over detached shell launchers and explains progress requirements.

## Unit Tests
- Rust: task start/progress/result across turns, foreground-to-background, snapshots in either ordering, omitted tool IDs, ambient exclusion, structured launch acknowledgements, tool progress, notifications and runtime cleanup.
- Transcript: background liveness and status reading; active call selection with overlapping calls.
- Prompt defaults: activity guidance reaches direct, orchestrator and worker sessions.

## Integration / Functional Tests
- Normalize provider-shaped task frames, persist through the session forest, reduce and render the same status and output on reload.
- SDK sidecar forwards task/progress messages after result without filtering.
- Recovery records unknown outcome for tasks from a dead runtime rather than leaving a spinner or reporting success.

## Smoke Tests
- bun run build and bun run test pass in the isolated worktree.

## E2E Tests
- Fresh browser fixture: launch tests, emit progress, end assistant turn, keep tests visible, then show failure or completion with output.

## Manual Tests
- In a fresh Bridge session, run a native background test command and inspect the collapsed activity during and after the assistant reply.
- Stop or restart its runtime and verify the transcript no longer claims that the task is running.
- Detached shell children cannot be discovered reliably from a completed launcher; the shared prompt directs agents to tracked execution or an explicit observable log.
