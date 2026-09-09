export type Tone = "success" | "warning" | "info" | "destructive" | "faint";

export type SidebarSession = {
  title: string;
  status: string;
  tone: Tone;
  time: string;
  selected?: boolean;
};

export type SidebarProject = {
  name: string;
  sessions: SidebarSession[];
  open?: boolean;
};

export type Entry =
  | { kind: "user"; text: string }
  | { kind: "assistant"; text: string }
  | { kind: "rail"; label: string; status?: string; text: string }
  | {
      kind: "notice";
      edge: Tone;
      title: string;
      status?: string;
      text: string;
      caption?: string;
      code?: string;
      actions?: string[];
    }
  | { kind: "collapsed"; label: string };

export type DockFile = { dir: string; name: string; added: number; removed: number };

export type Dock = {
  heading: string;
  origin: string;
  added: number;
  removed: number;
  files: DockFile[];
};

export type Scene = {
  id: string;
  tab: string;
  blurb: string;
  title: string;
  subtitle: string;
  projects: SidebarProject[];
  entries: Entry[];
  dock: Dock;
};

const supervisorProjects: SidebarProject[] = [
  {
    name: "Worktree lifecycle inventory",
    open: true,
    sessions: [
      { title: "Orchestrator", status: "working", tone: "success", time: "2m", selected: true },
      { title: "Implementer", status: "isolated", tone: "success", time: "1m" },
      { title: "Verifier", status: "read-only", tone: "warning", time: "18s" },
    ],
  },
  {
    name: "Usage ledger rollups",
    open: true,
    sessions: [{ title: "Orchestrator", status: "working", tone: "success", time: "6m" }],
  },
  {
    name: "Refresh-token rotation",
    sessions: [{ title: "Orchestrator", status: "merged", tone: "faint", time: "2d" }],
  },
];

export const scenes: Scene[] = [
  {
    id: "orchestrate",
    tab: "Orchestrate a fleet",
    blurb:
      "One orchestrator plans and routes. Workers on Codex, Claude Code, and OpenCode take bounded slices of the task and hand back typed results.",
    title: "Orchestrator",
    subtitle: "Worktree lifecycle inventory",
    projects: supervisorProjects,
    entries: [
      {
        kind: "user",
        text: "Reclaimed branches still show as active in the sidebar. Trace it and fix it, but I want a second harness to verify.",
      },
      {
        kind: "assistant",
        text: "The coordinator marks a branch reclaimed before the forest entry lands, so the sidebar reads one stale tick. Delegating the fix to an isolated worker.",
      },
      { kind: "collapsed", label: "Read worktree_coordinator.rs" },
      {
        kind: "rail",
        label: "Delegated",
        status: "running",
        text: "Implementer on Claude, standard tier, isolated worktree",
      },
      {
        kind: "notice",
        edge: "success",
        title: "Worker result",
        status: "tests passed",
        text: "Reordered reclaim after the forest append and added a regression test.",
        caption: "2 changed files · cargo test -p bridge-core worktree::",
      },
      { kind: "rail", label: "Checkpoint", status: "Saved", text: "Reclaim ordering and its regression test" },
    ],
    dock: {
      heading: "2 files changed",
      origin: "feat/worktree-lifecycle · uncommitted vs HEAD (8d0babe)",
      added: 38,
      removed: 12,
      files: [
        { dir: "src-tauri/bridge-core/src/", name: "worktree_coordinator.rs", added: 12, removed: 9 },
        { dir: "src-tauri/bridge-core/tests/", name: "worktree_reclaim.rs", added: 26, removed: 3 },
      ],
    },
  },
  {
    id: "policy",
    tab: "Policy owns the gates",
    blurb:
      "Capability tier, write scope, worktree isolation, concurrency, depth, retries, budgets, approvals. The policy engine decides. Routing and learning only rank what it already allows.",
    title: "Orchestrator",
    subtitle: "Worktree lifecycle inventory",
    projects: supervisorProjects,
    entries: [
      { kind: "user", text: "Let the worker touch the sidebar too, and push it to main once the tests pass." },
      {
        kind: "assistant",
        text: "Two requests, two gates. Widening the write scope needs a new grant, and merging to main is approval-only at this tier. Nothing moves until you decide.",
      },
      {
        kind: "notice",
        edge: "warning",
        title: "These paths were not pre-approved by you.",
        status: "waiting for you",
        text: "The worker proposed them itself, so Bridge needs a one-time authorization.",
        caption: "Write scope",
        code: "src/components/BridgeSidebar.tsx",
        actions: ["Deny", "Authorize once"],
      },
      {
        kind: "notice",
        edge: "warning",
        title: "Merge to main needs your approval",
        status: "waiting for you",
        text: "Evidence is complete, but this capability tier cannot merge on its own.",
        actions: ["Not now", "Approve merge"],
      },
      { kind: "rail", label: "Recorded", status: "Saved", text: "Both decisions are durable either way" },
    ],
    dock: {
      heading: "2 files changed",
      origin: "feat/worktree-lifecycle · uncommitted vs HEAD (8d0babe)",
      added: 38,
      removed: 12,
      files: [
        { dir: "src-tauri/bridge-core/src/", name: "worktree_coordinator.rs", added: 12, removed: 9 },
        { dir: "src-tauri/bridge-core/tests/", name: "worktree_reclaim.rs", added: 26, removed: 3 },
      ],
    },
  },
  {
    id: "worktrees",
    tab: "Isolated worktrees",
    blurb:
      "Every task gets its own branch and worktree. Workers whose write scopes overlap get their own too, so parallel work never shares a dirty tree.",
    title: "Orchestrator",
    subtitle: "Usage ledger rollups",
    projects: [
      { ...supervisorProjects[0], sessions: supervisorProjects[0].sessions.map(s => ({ ...s, selected: false })) },
      {
        name: "Usage ledger rollups",
        open: true,
        sessions: [
          { title: "Orchestrator", status: "working", tone: "success", time: "6m", selected: true },
          { title: "Implementer", status: "isolated", tone: "success", time: "3m" },
        ],
      },
      supervisorProjects[2],
    ],
    entries: [
      { kind: "user", text: "Run the usage rollup refactor in parallel with the lifecycle fix." },
      {
        kind: "assistant",
        text: "Both touch bridge-core, so the second worker gets its own worktree instead of sharing one. Scopes stay disjoint and Git state stays honest.",
      },
      {
        kind: "notice",
        edge: "info",
        title: "Worktree created",
        status: "ready",
        text: "A second isolated checkout, branched from the same base commit.",
        caption: "Path",
        code: ".worktrees/worker-b330  ·  feat/usage-tracking",
      },
      { kind: "collapsed", label: "Subagent finished" },
      {
        kind: "notice",
        edge: "success",
        title: "No divergence",
        status: "clean",
        text: "HEAD unchanged since the last checkpoint and the working tree is clean, so the conversation and the files agree.",
      },
    ],
    dock: {
      heading: "3 files changed",
      origin: "feat/usage-tracking · uncommitted vs HEAD (8d0babe)",
      added: 96,
      removed: 14,
      files: [
        { dir: "src-tauri/bridge-core/src/usage/", name: "rollup.rs", added: 61, removed: 4 },
        { dir: "src-tauri/bridge-core/src/usage/", name: "ledger.rs", added: 18, removed: 10 },
        { dir: "src-tauri/bridge-core/tests/", name: "usage_rollup.rs", added: 17, removed: 0 },
      ],
    },
  },
  {
    id: "verify",
    tab: "Verify across harnesses",
    blurb:
      "A worker's own word is not evidence. Completion gates demand typed results and, when you ask for it, a verifier from a different harness family.",
    title: "Orchestrator",
    subtitle: "Worktree lifecycle inventory",
    projects: supervisorProjects,
    entries: [
      { kind: "user", text: "Don't merge until a different harness has verified it." },
      {
        kind: "assistant",
        text: "Completion gate set to cross-family evidence. A Codex verifier is running read-only in the implementation worktree, so Claude's own report does not count.",
      },
      {
        kind: "notice",
        edge: "info",
        title: "Verifying",
        status: "2 of 3 required checks passed",
        text: "Verification record is saved locally and has not been committed.",
        caption: "Proof and checks",
        code: "Revision 307729bf075c · clean",
      },
      {
        kind: "notice",
        edge: "success",
        title: "Completion gate passed",
        status: "cross-family",
        text: "Evidence from two harness families is recorded as typed results. Same-family evidence was rejected by policy.",
      },
      { kind: "rail", label: "Checkpoint", status: "Saved", text: "Verified boundary at turn 14" },
    ],
    dock: {
      heading: "2 files changed",
      origin: "feat/worktree-lifecycle · uncommitted vs HEAD (8d0babe)",
      added: 38,
      removed: 12,
      files: [
        { dir: "src-tauri/bridge-core/src/", name: "worktree_coordinator.rs", added: 12, removed: 9 },
        { dir: "src-tauri/bridge-core/tests/", name: "worktree_reclaim.rs", added: 26, removed: 3 },
      ],
    },
  },
  {
    id: "usage",
    tab: "Track usage",
    blurb:
      "Tokens, cost, and context health per harness, per task, per day. Budgets are gates too: a worker that runs out of turn budget stops instead of improvising.",
    title: "Orchestrator",
    subtitle: "Worktree lifecycle inventory",
    projects: supervisorProjects,
    entries: [
      { kind: "user", text: "How much has this task cost so far, and how much context is left?" },
      {
        kind: "assistant",
        text: "$4.18 across three harnesses. The orchestrator is at 62% of its window with a verified checkpoint at turn 14, so a resume would restore from there.",
      },
      {
        kind: "notice",
        edge: "info",
        title: "Usage for this task",
        status: "today",
        text: "Counted per harness, from the provider's own reported tokens.",
        code: "claude   412k in · 38k out · $3.10\ncodex     96k in ·  9k out · $0.74\nopencode  31k in ·  4k out · $0.34",
      },
      {
        kind: "notice",
        edge: "warning",
        title: "Turn budget",
        status: "64% used",
        text: "A worker that exhausts its turn budget stops and reports rather than improvising a shortcut.",
        caption: "This turn",
        code: "limit 80k · used 51k · remaining 29k",
      },
      { kind: "rail", label: "Context health", status: "healthy", text: "Orchestrator 62% · worker 41% of window" },
    ],
    dock: {
      heading: "2 files changed",
      origin: "feat/worktree-lifecycle · uncommitted vs HEAD (8d0babe)",
      added: 38,
      removed: 12,
      files: [
        { dir: "src-tauri/bridge-core/src/", name: "worktree_coordinator.rs", added: 12, removed: 9 },
        { dir: "src-tauri/bridge-core/tests/", name: "worktree_reclaim.rs", added: 26, removed: 3 },
      ],
    },
  },
];
