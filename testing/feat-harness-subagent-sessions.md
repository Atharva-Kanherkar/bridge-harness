# feat/harness-subagent-sessions — Test Contract

## Functional Behavior
- Normalize Claude parent_tool_use_id and task lifecycle; suppress background launch placeholders. Ignore non-agent, ambient and skip_transcript tasks.
- Normalize Codex child thread notifications without mutating root stream state. Child turn/status events never change the parent turn.
- Persist each provider child once as a processless harness_subagent session under its chat. Child messages, tools, approvals and status live in its forest. Parent gets one start and one finish pointer, excluded from model context.
- OpenCode tagged events use the same routing. No worker runtime, worktree, policy, PID or separately counted usage is created for children.
- Child Stop routes through the parent runtime to the provider child. Approvals route replies through that runtime. Claude forwards child prose and supports stop_task without stopping siblings.
- Restart marks running children interrupted with parent_process_exited. Send/resume/fork/compact reject processless sessions; archive follows parent; titles/memory extraction skip children.
- Root turn and ordinary Bridge worker behavior remain unchanged.

## Unit Tests
- agent normalization: tagged Claude assistant/user/progress; lifecycle and ignored tasks; background result suppression; Codex root/child isolation and subAgentActivity kinds.
- harness_subagents: idempotent child creation, event routing, lifecycle deduplication, no parent mutation, no child usage ledger, recovery, processless guards.
- sidecar stop_task calls stopTask with the requested id and options enable child text/per-task stopping.

## Integration / Functional Tests
- Replay interleaved root and three child Codex turns: parent stays working until root completion, distinct child forests, exactly one start/finish pointer per child.
- Replay Claude background task lifecycle: launch result suppressed, progress/approval/message attribution retained, notification finishes child.
- Parent and child approval/control routing use one adapter, with no new adapter process.

## Smoke Tests
- bun run build and bun run test must pass before PR creation.
- cargo check workspace verifies all adapter trait implementations and session consumers.

## E2E Tests
- UI journey and screenshots belong to the dependent UI PR, with a separate contract before UI implementation.
- Live provider verification: Direct chat starts children, root remains working, inspect each child forest and stop one sibling independently. Record unavailable provider access honestly.

## Manual / cURL Tests
- Review session rows: kind=harness_subagent, parent_session_id points to chat, provider_session_id matches sender; no worker_runtime or adapter_pid.
- Inspect stored root forest: child prose/tool calls absent; lifecycle pointers context_visibility excluded.
- Restart recovery: working child becomes interrupted with parent_process_exited.
- Attempt send/fork/resume/compact on child: typed processless rejection.
