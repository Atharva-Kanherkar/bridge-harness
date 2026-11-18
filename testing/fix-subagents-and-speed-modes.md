# Harness subagents survive, and fast / ultrafast / ultracode run

Locked before implementation. Evidence is the live `bridge.db` (2026-11-17/18).

## Background subagents are not lost

Live evidence: background Claude agents went silent about two minutes after
the parent turn ended, and the next message cold-started a resumed process
that reported them as "didn't finish before the previous session ended".
`runtime_budget::IdleRuntimes` released the chat's process after `IDLE_TTL`
(120 s) or as soon as more than `MAX_IDLE_RUNTIMES` (2) chats were idle. It
deactivated the reader first, so the dead children were never closed.

- A chat whose harness subagent is still open (`working`, `waiting`,
  `starting`, `ready`) and active in the last 30 minutes is never released.
  A child silent past that window no longer pins the process.
- A chat with a running background task (its own or a child's) is never
  released.
- When the reaper or a replacing launch ends a process, its open harness
  subagents become `interrupted` and its background tasks close, in the same
  step. Nothing stays `working` after its process is gone.
- The end of the parent's turn does not idle a Claude child: Claude reports
  every background agent's end with `task_notification`, so an open child is
  a live background agent.

## One Claude agent is one child session

Live evidence: 118 phantom one-entry "Claude subagent" rows, and 5 agents
whose work and completion landed in two different sessions.

- A continuation (`SendMessage`, resume) of a known `task_id` reuses the
  child created by the original `Agent` call, in the same process and after a
  restart. Its completion closes that child.
- A `task_notification`, `task_progress` or `task_updated` for a task this
  chat never started (a background shell, an orphan report after a restart)
  never creates a child. If an existing child owns the task id, it is updated.
- A background shell started by a subagent is that subagent's activity, not
  the parent's.

## Speed and ultracode

- Claude: a model whose catalog row has `supportsFastMode` offers a `fast`
  speed. Choosing it launches the SDK with `settings.fastMode = true` (the SDK
  opt-in). A model that supports `xhigh` effort also offers the `ultracode`
  effort, launched as `settings.ultracode = true`.
- Codex: each `model/list` `serviceTiers` row is a speed (`priority` is Fast;
  `ultrafast` when listed). The chosen id goes on `thread/start`,
  `thread/resume` and every `turn/start` as `serviceTier`.
- Speed is stored per chat (`sessions.speed`), shown in the chat's model
  control next to effort, and changing it restarts the runtime on the next
  turn the way effort does. An unsupported speed for the model is dropped.

## Verification

- Rust unit tests for each bullet above (`runtime_budget`, `harness_subagents`,
  `agent` normalizer, `codex_adapter`, `claude_adapter`).
- Sidecar `node:test` for the fast-mode and ultracode options.
- Vitest for the speed control.
- Regenerate protocol artifacts; `bun run build` and `bun run test` green.
