export type Tone = "success" | "warning" | "destructive" | "faint" | "fg";

export type SidebarRow = {
  label: string;
  sub?: string;
  tone: Tone;
  right?: string;
  depth?: 0 | 1;
  active?: boolean;
};

export type StatusChip = { label: string; tone: Tone };

export type CheckItem = { label: string; detail?: string; tone: Tone };

export type CardBody =
  | { kind: "text"; text: string }
  | { kind: "grid"; rows: [string, string][] }
  | { kind: "checks"; items: CheckItem[] }
  | { kind: "code"; lines: string[] };

export type Card = { title: string; status?: StatusChip; body: CardBody };

export type ConversationItem =
  | { kind: "message"; who: "you" | "bridge"; meta?: string; text: string }
  | { kind: "card"; card: Card };

export type Scene = {
  id: string;
  tab: string;
  blurb: string;
  chips: string[];
  model: string;
  changes: number;
  sidebar: SidebarRow[];
  conversation: ConversationItem[];
  work: Card[];
};

const lifecycleTask: SidebarRow[] = [
  { label: "Worktree lifecycle inventory", sub: "feat/worktree-lifecycle", tone: "success", right: "live", active: true },
  { label: "impl · claude", sub: "worker-2f9a · isolated", tone: "success", depth: 1 },
  { label: "verify · codex", sub: "worker-71c0 · read-only", tone: "warning", depth: 1 },
];

const usageTask: SidebarRow[] = [
  { label: "Usage ledger rollups", sub: "feat/usage-tracking", tone: "success", right: "live" },
  { label: "research · opencode", sub: "worker-b330", tone: "success", depth: 1 },
];

const settledTasks: SidebarRow[] = [
  { label: "Refresh-token rotation", sub: "merged · PR #577", tone: "faint" },
  { label: "Sidebar archive action", sub: "archived", tone: "faint" },
];

const usageToday: Card = {
  title: "Usage · today",
  body: {
    kind: "grid",
    rows: [
      ["claude", "412k"],
      ["codex", "96k"],
      ["opencode", "31k"],
      ["cost", "$4.18"],
    ],
  },
};

export const scenes: Scene[] = [
  {
    id: "orchestrate",
    tab: "Orchestrate a fleet",
    blurb:
      "One orchestrator plans and routes. Workers on Codex, Claude Code, and OpenCode take bounded slices of the task and hand back typed results.",
    chips: ["harness · main", "3 workers live"],
    model: "fable-5-1 · high",
    changes: 7,
    sidebar: [...lifecycleTask, ...usageTask, ...settledTasks],
    conversation: [
      {
        kind: "message",
        who: "you",
        text: "Reclaimed branches still show as active in the sidebar. Trace it and fix it, but I want a second harness to verify.",
      },
      {
        kind: "message",
        who: "bridge",
        meta: "orchestrator",
        text: "The coordinator marks a branch reclaimed before the forest entry lands, so the sidebar reads one stale tick. Delegating the fix to an isolated worker.",
      },
      {
        kind: "card",
        card: {
          title: "Delegation · implementation",
          status: { label: "running", tone: "success" },
          body: {
            kind: "grid",
            rows: [
              ["tier", "standard · medium"],
              ["write", "isolated · src-tauri/bridge-core/**"],
              ["verify", "cargo test -p bridge-core worktree::"],
            ],
          },
        },
      },
      {
        kind: "card",
        card: {
          title: "Worker result · worker-2f9a",
          status: { label: "tests passed", tone: "success" },
          body: {
            kind: "text",
            text: "Reordered reclaim after the forest append and added a regression test. 2 files changed, cargo test green.",
          },
        },
      },
      {
        kind: "card",
        card: {
          title: "Completion gate",
          status: { label: "verifying", tone: "warning" },
          body: {
            kind: "text",
            text: "Codex verifier launched in the implementation worktree. Same-family evidence rejected by policy.",
          },
        },
      },
    ],
    work: [
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
      {
        title: "Policy",
        body: {
          kind: "checks",
          items: [
            { label: "write scope authorized", tone: "success" },
            { label: "isolation enforced", tone: "success" },
            { label: "approval pending", detail: "merge", tone: "warning" },
          ],
        },
      },
      usageToday,
    ],
  },
  {
    id: "policy",
    tab: "Policy owns the gates",
    blurb:
      "Capability tier, write scope, worktree isolation, concurrency, depth, retries, budgets, approvals. The policy engine decides. Routing and learning only rank what it already allows.",
    chips: ["harness · main", "2 approvals pending"],
    model: "fable-5-1 · high",
    changes: 7,
    sidebar: [...lifecycleTask, ...usageTask, ...settledTasks],
    conversation: [
      {
        kind: "message",
        who: "you",
        text: "Let the worker touch the sidebar too, and just push it to main when the tests pass.",
      },
      {
        kind: "message",
        who: "bridge",
        meta: "orchestrator",
        text: "Two requests, two gates. Widening the write scope to src/** needs a new grant, and merging to main is approval-only for this tier. Nothing moves until you decide.",
      },
      {
        kind: "card",
        card: {
          title: "Policy decision · widen scope",
          status: { label: "needs approval", tone: "warning" },
          body: {
            kind: "grid",
            rows: [
              ["requested", "src/**"],
              ["granted", "src-tauri/bridge-core/**"],
              ["verdict", "queue for approval"],
            ],
          },
        },
      },
      {
        kind: "card",
        card: {
          title: "Policy decision · merge to main",
          status: { label: "needs approval", tone: "warning" },
          body: {
            kind: "grid",
            rows: [
              ["branch", "feat/worktree-lifecycle"],
              ["evidence", "cargo test · 41 passed"],
              ["gate", "human approval"],
            ],
          },
        },
      },
      {
        kind: "card",
        card: {
          title: "Approval request",
          status: { label: "waiting for you", tone: "warning" },
          body: {
            kind: "text",
            text: "Approve widening to src/** and merging feat/worktree-lifecycle. Both decisions are recorded in the session forest either way.",
          },
        },
      },
    ],
    work: [
      {
        title: "Gates",
        body: {
          kind: "checks",
          items: [
            { label: "capability tier", detail: "standard", tone: "success" },
            { label: "write scope", detail: "bridge-core/**", tone: "success" },
            { label: "isolation", detail: "worker worktree", tone: "success" },
            { label: "concurrency", detail: "2 of 3", tone: "success" },
            { label: "depth", detail: "1 of 2", tone: "success" },
            { label: "turn budget", detail: "61% used", tone: "warning" },
            { label: "approvals", detail: "2 pending", tone: "warning" },
          ],
        },
      },
      {
        title: "Approvals",
        body: {
          kind: "grid",
          rows: [
            ["widen scope", "pending"],
            ["merge", "pending"],
          ],
        },
      },
      {
        title: "Routing",
        body: {
          kind: "text",
          text: "Ranked claude for this slice from 14 prior tasks. Ranking cannot grant scope or skip a gate.",
        },
      },
    ],
  },
  {
    id: "worktrees",
    tab: "Isolated worktrees",
    blurb:
      "Every task gets its own branch and worktree. Workers whose write scopes overlap get their own too, so parallel work never shares a dirty tree.",
    chips: ["harness · main", "3 worktrees"],
    model: "fable-5-1 · high",
    changes: 3,
    sidebar: [
      { ...lifecycleTask[0], active: false },
      lifecycleTask[1],
      { ...usageTask[0], active: true },
      { label: "impl · opencode", sub: "worker-b330 · isolated", tone: "success", depth: 1 },
      ...settledTasks,
    ],
    conversation: [
      {
        kind: "message",
        who: "you",
        text: "Run the usage rollup refactor in parallel with the lifecycle fix.",
      },
      {
        kind: "message",
        who: "bridge",
        meta: "orchestrator",
        text: "Both touch bridge-core, so the second worker gets its own worktree instead of sharing one. Scopes stay disjoint and Git state stays honest.",
      },
      {
        kind: "card",
        card: {
          title: "Worktree created",
          status: { label: "ready", tone: "success" },
          body: {
            kind: "grid",
            rows: [
              ["path", ".worktrees/worker-b330"],
              ["branch", "feat/usage-tracking"],
              ["base", "main @ 8d0babe"],
              ["scope", "src-tauri/bridge-core/usage/**"],
            ],
          },
        },
      },
      {
        kind: "card",
        card: {
          title: "Worker result · worker-b330",
          status: { label: "tests passed", tone: "success" },
          body: {
            kind: "text",
            text: "Rollups now aggregate per harness per day. 3 files changed, no tracked files touched outside the scope.",
          },
        },
      },
      {
        kind: "card",
        card: {
          title: "Divergence check",
          status: { label: "clean", tone: "success" },
          body: {
            kind: "text",
            text: "HEAD unchanged since the last checkpoint and the working tree is clean. Conversation and files agree.",
          },
        },
      },
    ],
    work: [
      {
        title: "Worktrees",
        body: {
          kind: "checks",
          items: [
            { label: "feat/worktree-lifecycle", detail: "worker-2f9a", tone: "success" },
            { label: "feat/usage-tracking", detail: "worker-b330", tone: "success" },
            { label: "feat/dock-tasks", detail: "idle", tone: "faint" },
          ],
        },
      },
      {
        title: "Scope · worker-b330",
        body: {
          kind: "grid",
          rows: [
            ["read", "repository"],
            ["write", "usage/**"],
            ["denied", "src/**"],
          ],
        },
      },
      {
        title: "Git evidence",
        body: {
          kind: "grid",
          rows: [
            ["head", "8d0babe"],
            ["dirty", "0 files"],
            ["recorded by", "controller"],
          ],
        },
      },
    ],
  },
  {
    id: "verify",
    tab: "Verify across harnesses",
    blurb:
      "A worker's own word is not evidence. Completion gates demand typed results and, when you ask for it, a verifier from a different harness family.",
    chips: ["harness · main", "gate · verifying"],
    model: "fable-5-1 · high",
    changes: 7,
    sidebar: [
      lifecycleTask[0],
      lifecycleTask[1],
      { ...lifecycleTask[2], tone: "success", right: "live" },
      ...usageTask,
      ...settledTasks,
    ],
    conversation: [
      {
        kind: "message",
        who: "you",
        text: "Don't merge until a different harness has verified it.",
      },
      {
        kind: "message",
        who: "bridge",
        meta: "orchestrator",
        text: "Completion gate set to cross-family evidence. A Codex verifier is running read-only in the implementation worktree. Claude's own report does not count.",
      },
      {
        kind: "card",
        card: {
          title: "Verification · codex · worker-71c0",
          status: { label: "passed", tone: "success" },
          body: {
            kind: "checks",
            items: [
              { label: "cargo test -p bridge-core worktree::", detail: "41 passed", tone: "success" },
              { label: "diff reviewed", detail: "2 files", tone: "success" },
              { label: "no tracked changes outside scope", tone: "success" },
            ],
          },
        },
      },
      {
        kind: "card",
        card: {
          title: "Completion gate",
          status: { label: "passed", tone: "success" },
          body: {
            kind: "text",
            text: "Evidence from two harness families recorded as typed results. The merge now waits on your approval.",
          },
        },
      },
    ],
    work: [
      {
        title: "Evidence",
        body: {
          kind: "grid",
          rows: [
            ["claude", "implementation · typed"],
            ["codex", "verification · typed"],
            ["rejected", "same-family report"],
          ],
        },
      },
      {
        title: "Policy",
        body: {
          kind: "checks",
          items: [
            { label: "cross-family evidence", tone: "success" },
            { label: "verifier read-only", tone: "success" },
            { label: "approval pending", detail: "merge", tone: "warning" },
          ],
        },
      },
      {
        title: "Session forest",
        body: {
          kind: "grid",
          rows: [
            ["entries", "142"],
            ["checkpoint", "turn 14 · verified"],
            ["branch", "active"],
          ],
        },
      },
    ],
  },
  {
    id: "usage",
    tab: "Track usage",
    blurb:
      "Tokens, cost, and context health per harness, per task, per day. Budgets are gates too: a worker that runs out of turn budget stops instead of improvising.",
    chips: ["harness · main", "$4.18 today"],
    model: "fable-5-1 · high",
    changes: 7,
    sidebar: [...lifecycleTask, ...usageTask, ...settledTasks],
    conversation: [
      {
        kind: "message",
        who: "you",
        text: "How much has this task cost so far, and how much context is left?",
      },
      {
        kind: "message",
        who: "bridge",
        meta: "orchestrator",
        text: "$4.18 across three harnesses. The orchestrator is at 62% of its window with a verified checkpoint at turn 14, so a resume would restore from there.",
      },
      {
        kind: "card",
        card: {
          title: "Usage · this task",
          body: {
            kind: "grid",
            rows: [
              ["claude", "412k in · 38k out · $3.10"],
              ["codex", "96k in · 9k out · $0.74"],
              ["opencode", "31k in · 4k out · $0.34"],
              ["total", "$4.18"],
            ],
          },
        },
      },
      {
        kind: "card",
        card: {
          title: "Context health",
          status: { label: "healthy", tone: "success" },
          body: {
            kind: "checks",
            items: [
              { label: "orchestrator", detail: "62% of window", tone: "success" },
              { label: "worker-2f9a", detail: "41% of window", tone: "success" },
              { label: "checkpoint", detail: "turn 14 · verified", tone: "success" },
            ],
          },
        },
      },
    ],
    work: [
      {
        title: "Budget · this turn",
        body: {
          kind: "grid",
          rows: [
            ["limit", "80k"],
            ["used", "51k"],
            ["remaining", "29k"],
          ],
        },
      },
      usageToday,
      {
        title: "Checkpoints",
        body: {
          kind: "checks",
          items: [
            { label: "turn 14", detail: "verified", tone: "success" },
            { label: "turn 9", detail: "verified", tone: "faint" },
            { label: "compaction", detail: "none pending", tone: "faint" },
          ],
        },
      },
    ],
  },
];
