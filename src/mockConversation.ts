import type { AgentEvent } from "./types";

// Static event stream used purely as a design preview while a session has no
// real events yet — lets the conversation UI be reviewed without a live agent.

let seq = 0;
const ev = (kind: string, overrides: Partial<AgentEvent> = {}): AgentEvent => {
  seq += 1;
  return {
    id: seq, sessionId: "preview", sequence: seq, protocolVersion: 1, kind,
    itemId: `mock-${seq}`, role: null, status: "completed", title: null, text: null,
    data: {}, providerMeta: {}, createdAt: "now", ...overrides,
  };
};

export const MOCK_CONVERSATION: AgentEvent[] = [
  ev("message.completed", { role: "user", text: "Take issue #8 — hot-resume the adapter protocol after a restart, then open the PR." }),

  ev("reasoning.completed", {
    text: "The resume path has two hazards: the fast path re-acquires the store mutex it already holds, and an old reader thread can observe EOF after its replacement starts. A launch-generation check on the persisted `started_at` handles the second; the first needs the lock scoped before returning state.",
  }),

  ev("tool.completed", { title: "Read lib.rs", data: { type: "readFile", path: "src-tauri/src/lib.rs" } }),
  ev("file_change.completed", { title: "lib.rs", data: { path: "src-tauri/src/lib.rs", additions: 24, deletions: 3 } }),
  ev("command.completed", { title: "bun run test", data: { type: "commandExecution", command: "bun run test", durationMs: 3000, aggregatedOutput: "bun test v1.1.34\n\n 92 pass\n 0 fail\n 153 expect() calls\nRan 92 tests across 14 files. [3.02s]" } }),
  ev("file_change.completed", { title: "store.rs", data: { path: "src-tauri/src/store.rs", additions: 10, deletions: 2 } }),
  ev("command.completed", { title: "git diff --stat", data: { type: "commandExecution", command: "git diff --stat && git diff -w -- src-tauri/src/lib.rs | rg '^@@'", durationMs: 400, aggregatedOutput: " src-tauri/src/lib.rs   | 27 ++++++++++++---\n src-tauri/src/store.rs | 12 ++++--\n 2 files changed, 34 insertions(+), 5 deletions(-)" } }),

  ev("message.completed", {
    role: "assistant",
    text: [
      "The checkpoint review found one real blocker: the hot-resume fast path held the database mutex and attempted to lock it again while returning state, which would deadlock. I fixed that before recording the step as passed — this is exactly the kind of cumulative integration defect the `review-checkpoint` gate is meant to catch.",
      "",
      "The restart-specific race is also closed: stale readers now check the launch generation before tearing down a newer runtime.",
      "",
      "```rust",
      "fn resume_session(&self, id: SessionId) -> Result<Resumed> {",
      "    let generation = self.store.launch_generation()?;",
      "    let state = self.store.lock_scoped(|db| db.session_state(id))?;",
      "    if state.started_at.generation() != generation {",
      "        return Err(Error::StaleReader); // old thread — never tear down the new runtime",
      "    }",
      "    Ok(Resumed::from(state))",
      "}",
      "```",
      "",
      "What the suite now covers:",
      "- **Hot resume** — 92 Rust tests plus 61 frontend tests pass; the four live provider tests stay ignored by default",
      "- **Restart race** — an old reader observing EOF can no longer mark the resumed session stopped",
      "- **Diff hygiene** — the `review-checkpoint` gate ran against the locked contract before commit",
    ].join("\n"),
  }),

  ev("delegation.spawned", { title: "Verify WAL backup rejection on busy DB", data: { model: "claude-sonnet", modelLabel: "Sonnet", effort: "high" } }),
  ev("delegation.result", { title: "Worker finished", text: "Reproduced the busy-WAL case, confirmed the backup is rejected with `SQLITE_BUSY` and the retry path backs off correctly. One flaky assertion tightened in store step 4.", data: { delivered: true, model: "claude-sonnet", modelLabel: "Sonnet" } }),

  ev("approval.requested", {
    title: "Push branch and open PR",
    text: "Bridge wants to push the branch and create the pull request for issue #8.",
    status: "pending",
    data: { command: "git push -u origin codex/issue-8-adapter-resume && gh pr create --fill", cwd: "~/Projects/bridge-harness" },
  }),

  ev("reasoning.delta", { status: "streaming", text: "Preparing final checkpoint before PR merge — running the acceptance audit on every restoration label and fallback path…" }),
];
