import type { Card } from "./scenes";

export type LoopStep = {
  id: string;
  tab: string;
  title: string;
  text: string;
  cards: Card[];
};

export const loopSteps: LoopStep[] = [
  {
    id: "workspaces",
    tab: "Workspaces",
    title: "A task is a branch, a worktree, and its sessions",
    text: "Point Bridge at a validated local repository and it checks out a branch, opens a worktree, and starts an orchestrator session against it. A direct chat runs without a repository when there is nothing to isolate.",
    cards: [
      {
        title: "New task",
        status: { label: "ready", tone: "success" },
        body: {
          kind: "grid",
          rows: [
            ["repo", "harness"],
            ["branch", "feat/worktree-lifecycle"],
            ["worktree", ".worktrees/task-4c1e"],
            ["base", "main @ 8d0babe"],
          ],
        },
      },
      {
        title: "Sessions",
        body: {
          kind: "checks",
          items: [
            { label: "orchestrator", detail: "claude", tone: "success" },
            { label: "workers", detail: "none yet", tone: "faint" },
          ],
        },
      },
    ],
  },
  {
    id: "delegation",
    tab: "Delegation",
    title: "The orchestrator asks. The policy engine answers",
    text: "For every requested slice the policy decides one of six outcomes: run in the parent, reuse a compatible worker, spawn a new one, queue it, reject it, or request your approval. Capability tier, write scope, isolation, concurrency, depth, retries, and budgets are all inputs.",
    cards: [
      {
        title: "Request · implementation",
        body: {
          kind: "grid",
          rows: [
            ["requested", "src-tauri/bridge-core/**"],
            ["tier", "standard · medium"],
            ["depth", "1 of 2"],
          ],
        },
      },
      {
        title: "Decision",
        status: { label: "spawn worker", tone: "success" },
        body: {
          kind: "checks",
          items: [
            { label: "scope within grant", tone: "success" },
            { label: "isolated worktree required", tone: "success" },
            { label: "concurrency 2 of 3", tone: "success" },
          ],
        },
      },
    ],
  },
  {
    id: "conversation",
    tab: "Conversation",
    title: "One normalized stream, appended not overwritten",
    text: "Each provider's native events become one model: messages, reasoning, plans, tool calls, approvals, file changes, errors, and artifacts. Entries are immutable, so forking the branch to try another direction never destroys the original.",
    cards: [
      {
        title: "Session forest",
        body: {
          kind: "grid",
          rows: [
            ["entries", "142"],
            ["branch", "active"],
            ["forks", "2"],
          ],
        },
      },
      {
        title: "Divergence",
        status: { label: "clean", tone: "success" },
        body: {
          kind: "text",
          text: "HEAD unchanged since the last checkpoint and the working tree is clean. Conversation and files agree.",
        },
      },
    ],
  },
  {
    id: "work",
    tab: "Work board",
    title: "Evidence next to the conversation that produced it",
    text: "Worktree stats, the diff, policy checks, and pending approvals sit in one pane beside the transcript. Nothing about a worker's claim has to be taken on trust.",
    cards: [
      {
        title: "Worktree",
        body: {
          kind: "grid",
          rows: [
            ["path", ".worktrees/worker-2f9a"],
            ["diff", "+38 −12 · 2 files"],
            ["tree", "clean"],
          ],
        },
      },
      {
        title: "worktree_coordinator.rs",
        body: {
          kind: "code",
          lines: ["- self.mark_reclaimed(&branch)?;", "  forest.append(entry)?;", "+ self.mark_reclaimed(&branch)?;"],
        },
      },
    ],
  },
  {
    id: "surfaces",
    tab: "Terminal and browser",
    title: "Shell and browser work, still supervised",
    text: "A workspace terminal keeps ad-hoc commands out of the agent transcript. The browser bridge supervises one tab you approve in your own logged-in browser, and the lease ends when the tab closes or navigation leaves the granted domain.",
    cards: [
      {
        title: "Terminal",
        body: {
          kind: "code",
          lines: ["$ cargo test -p bridge-core worktree::", "  41 passed, 0 failed"],
        },
      },
      {
        title: "Browser lease",
        status: { label: "active", tone: "warning" },
        body: {
          kind: "checks",
          items: [
            { label: "one approved tab", tone: "success" },
            { label: "no cookies exported", tone: "success" },
            { label: "expires on domain change", detail: "30 min", tone: "warning" },
          ],
        },
      },
    ],
  },
  {
    id: "ship",
    tab: "Ship",
    title: "The gate collects evidence. You approve the merge",
    text: "A completion gate can require a verifier from a different harness family, so no worker closes its own task. When the evidence is in, the merge still waits on you.",
    cards: [
      {
        title: "Completion gate",
        status: { label: "passed", tone: "success" },
        body: {
          kind: "checks",
          items: [
            { label: "claude", detail: "implementation · typed", tone: "success" },
            { label: "codex", detail: "verification · typed", tone: "success" },
            { label: "same-family evidence", detail: "rejected", tone: "faint" },
          ],
        },
      },
      {
        title: "Approval",
        status: { label: "waiting for you", tone: "warning" },
        body: {
          kind: "text",
          text: "Merge feat/worktree-lifecycle into main. The decision is recorded in the session forest either way.",
        },
      },
    ],
  },
];
