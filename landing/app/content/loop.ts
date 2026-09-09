import type { Entry } from "./scenes";

export type LoopStep = {
  id: string;
  tab: string;
  title: string;
  text: string;
  entries: Entry[];
};

export const loopSteps: LoopStep[] = [
  {
    id: "workspaces",
    tab: "Workspaces",
    title: "A task is a branch, a worktree, and its sessions",
    text: "Point Bridge at a validated local repository and it checks out a branch, opens a worktree, and starts an orchestrator session against it. A direct chat runs without a repository when there is nothing to isolate.",
    entries: [
      {
        kind: "notice",
        edge: "info",
        title: "Task opened",
        status: "ready",
        text: "A branch, a worktree, and an orchestrator session, created together.",
        caption: "Worktree",
        code: ".worktrees/task-4c1e  ·  feat/worktree-lifecycle  ·  main @ 8d0babe",
      },
      { kind: "rail", label: "Sessions", status: "1 active", text: "Orchestrator on Claude, no workers yet" },
    ],
  },
  {
    id: "delegation",
    tab: "Delegation",
    title: "The orchestrator asks. The policy engine answers",
    text: "For every requested slice the policy decides one of six outcomes: run in the parent, reuse a compatible worker, spawn a new one, queue it, reject it, or request your approval. Capability tier, write scope, isolation, concurrency, depth, retries, and budgets are all inputs.",
    entries: [
      {
        kind: "rail",
        label: "Requested",
        status: "standard · medium",
        text: "Write access to src-tauri/bridge-core/**, at depth 1 of 2",
      },
      {
        kind: "notice",
        edge: "success",
        title: "Spawn an isolated worker",
        status: "authorized",
        text: "Scope sits inside the existing grant, concurrency is 2 of 3, and the overlap forces a worktree of its own.",
      },
    ],
  },
  {
    id: "conversation",
    tab: "Conversation",
    title: "One normalized stream, appended not overwritten",
    text: "Each provider's native events become one model: messages, reasoning, plans, tool calls, approvals, file changes, errors, and artifacts. Entries are immutable, so forking the branch to try another direction never destroys the original.",
    entries: [
      { kind: "rail", label: "Session forest", status: "142 entries", text: "One active branch, two forks kept" },
      {
        kind: "notice",
        edge: "success",
        title: "No divergence",
        status: "clean",
        text: "HEAD is unchanged since the last checkpoint and the working tree is clean, so the conversation and the files agree.",
      },
    ],
  },
  {
    id: "work",
    tab: "Work board",
    title: "Evidence next to the conversation that produced it",
    text: "Worktree stats, the diff, policy checks, and pending approvals sit in one pane beside the transcript. Nothing about a worker's claim has to be taken on trust.",
    entries: [
      {
        kind: "notice",
        edge: "info",
        title: "2 files changed",
        status: "+38 −12",
        text: "The diff, the policy checks, and the pending approval all read from the same worktree.",
        caption: "worktree_coordinator.rs",
        code: "- self.mark_reclaimed(&branch)?;\n  forest.append(entry)?;\n+ self.mark_reclaimed(&branch)?;",
      },
      { kind: "rail", label: "Worktree", status: "clean", text: ".worktrees/worker-2f9a" },
    ],
  },
  {
    id: "surfaces",
    tab: "Terminal and browser",
    title: "Shell and browser work, still supervised",
    text: "A workspace terminal keeps ad-hoc commands out of the agent transcript. The browser bridge supervises one tab you approve in your own logged-in browser, and the lease ends when the tab closes or navigation leaves the granted domain.",
    entries: [
      {
        kind: "notice",
        edge: "info",
        title: "Workspace terminal",
        status: "exit 0",
        text: "Ad-hoc shell work stays out of the agent transcript.",
        code: "$ cargo test -p bridge-core worktree::\n  41 passed, 0 failed",
      },
      {
        kind: "notice",
        edge: "warning",
        title: "Browser lease active",
        status: "30 min",
        text: "One tab you approved, in your own logged-in browser. No profile is copied and no cookies are exported. The lease ends when the tab closes or navigation leaves the granted domain.",
      },
    ],
  },
  {
    id: "ship",
    tab: "Ship",
    title: "The gate collects evidence. You approve the merge",
    text: "A completion gate can require a verifier from a different harness family, so no worker closes its own task. When the evidence is in, the merge still waits on you.",
    entries: [
      {
        kind: "notice",
        edge: "success",
        title: "Completion gate passed",
        status: "cross-family",
        text: "Claude implemented and Codex verified, both as typed results. Same-family evidence was rejected by policy.",
      },
      {
        kind: "notice",
        edge: "warning",
        title: "Merge needs your approval",
        status: "waiting for you",
        text: "Merge feat/worktree-lifecycle into main. The decision is recorded in the session forest either way.",
        actions: ["Not now", "Approve merge"],
      },
    ],
  },
];
