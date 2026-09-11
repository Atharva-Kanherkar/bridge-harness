/*
 * The animated product demo. Every class in `components/app/*` is lifted from the running
 * app (sidebar widths, row heights, the ui/caption/message type scale), so this is the real
 * interface rendered as DOM rather than a picture of it. To re-derive the markup, run
 * `node node_modules/.bin/vite --port 1468` from the repo root, accept the model setup
 * wizard, add the `dark` class to <html>, and read the rendered classes off a session.
 *
 * Scenes are data. Each one lists the steps the player reveals in order; nothing here
 * branches inside a component.
 */

export type HarnessId = "claude" | "codex" | "opencode" | "cursor";
export type Tone = "success" | "warning" | "info" | "destructive" | "faint";

export const harnessLabel: Record<HarnessId, string> = {
  claude: "Claude Code",
  codex: "Codex",
  opencode: "OpenCode",
  cursor: "Cursor",
};

/** Mirrors `--harness-*` in the app's `src/index.css`; Cursor has no token there, so it
 * borrows the neutral info ink rather than inventing a brand colour. */
export const harnessDot: Record<HarnessId, string> = {
  claude: "bg-harness-claude",
  codex: "bg-harness-codex",
  opencode: "bg-harness-opencode",
  cursor: "bg-info",
};

export type SidebarSession = {
  title: string;
  harness: HarnessId;
  status: string;
  tone: Tone;
  time: string;
  worker?: boolean;
  selected?: boolean;
};

export type SidebarRepo = {
  name: string;
  branch: string;
  sessions: SidebarSession[];
  open?: boolean;
};

export const repos: SidebarRepo[] = [
  {
    name: "bridge-harness",
    branch: "feat/worktree-lifecycle",
    open: true,
    sessions: [
      { title: "Worktree lifecycle inventory", harness: "codex", status: "working", tone: "success", time: "2m", selected: true },
      { title: "Implementation · strong", harness: "claude", status: "isolated", tone: "success", time: "1m", worker: true },
      { title: "Verification · strong", harness: "codex", status: "read-only", tone: "warning", time: "18s", worker: true },
    ],
  },
  {
    name: "atlas-api",
    branch: "atlas/token-rotation",
    open: true,
    sessions: [
      { title: "Refresh-token rotation", harness: "opencode", status: "working", tone: "success", time: "6m" },
      { title: "Implementation · standard", harness: "cursor", status: "isolated", tone: "success", time: "3m", worker: true },
    ],
  },
  {
    name: "rimo-frontend",
    branch: "rimo/chart-regression",
    open: true,
    sessions: [
      { title: "Streaming chart regression", harness: "claude", status: "needs you", tone: "warning", time: "12m" },
      { title: "Review · fast", harness: "codex", status: "merged", tone: "faint", time: "2d", worker: true },
    ],
  },
  {
    name: "deck-shell",
    branch: "bridge/deck-shell",
    open: false,
    sessions: [{ title: "Polish the Deck shell", harness: "opencode", status: "merged", tone: "faint", time: "3d" }],
  },
];

export const directChats: SidebarSession[] = [
  { title: "Explain the policy engine", harness: "claude", status: "ready", tone: "faint", time: "1h" },
  { title: "Rust lifetimes, one question", harness: "codex", status: "ready", tone: "faint", time: "3h" },
];

export type Check = { name: string; kind: string; state: "passed" | "running" | "pending"; detail?: string; family?: HarnessId };
export type DockFile = { dir: string; name: string; added: number; removed: number; risk?: "High" | "Medium" | "Low"; lang?: string };

export type Entry =
  | { kind: "user"; text: string }
  | { kind: "assistant"; text: string }
  | { kind: "collapsed"; label: string }
  | { kind: "rail"; label: string; status?: string; text: string }
  | { kind: "notice"; edge: Tone; title: string; status?: string; text: string; caption?: string; code?: string; actions?: string[] }
  | { kind: "activity"; summary: string; steps: string; rows: { label: string; path?: string; stat?: string }[]; diff?: string }
  | { kind: "checks"; title: string; status: string; revision: string; checks: Check[] }
  | { kind: "worker"; harness: HarnessId; label: string; status: string; tone: Tone; text: string };

export type Dock = {
  origin: string;
  added: number;
  removed: number;
  files: DockFile[];
};

export type Tile = {
  title: string;
  harness: HarnessId;
  repo: string;
  status: string;
  tone: Tone;
  elapsed: string;
  lines: Entry[];
};

export type Scene = {
  id: string;
  label: string;
  title: string;
  text: string;
  /** Which body the frame renders. The chrome around it never changes. */
  view: "chat" | "mission" | "usage";
  toolbar?: { title: string; subtitle: string };
  prompt?: string;
  entries?: Entry[];
  dock?: Dock;
  tiles?: Tile[];
};

const codeDiff = `  self.registry.mark_reclaimed(&branch)?;
- forest.append(entry)?;
+ forest.append(entry)?;
+ self.registry.mark_reclaimed(&branch)?;`;

export const scenes: Scene[] = [
  {
    id: "mission",
    label: "Mission Control",
    title: "Every live chat, in motion",
    text: "All active conversations at once. Each tile is the real chat with its transcript, approvals, and composer, resized into a grid you can rearrange.",
    view: "mission",
    tiles: [
      {
        title: "Worktree lifecycle inventory",
        harness: "codex",
        repo: "bridge-harness",
        status: "working",
        tone: "success",
        elapsed: "2m",
        lines: [
          { kind: "assistant", text: "The coordinator marks a branch reclaimed before the forest entry lands, so the sidebar reads one stale tick." },
          { kind: "rail", label: "Delegated", status: "running", text: "Implementation on Claude, isolated worktree" },
          { kind: "notice", edge: "success", title: "Worker result", status: "tests passed", text: "Reordered reclaim after the forest append." },
        ],
      },
      {
        title: "Refresh-token rotation",
        harness: "opencode",
        repo: "atlas-api",
        status: "working",
        tone: "success",
        elapsed: "6m",
        lines: [
          { kind: "assistant", text: "Rotating on read means two concurrent reads can both mint a token. Moving the swap behind the store lock." },
          { kind: "collapsed", label: "Read tokenStore.ts · src/auth" },
          { kind: "rail", label: "Checkpoint", status: "Saved", text: "Lock ordering decided" },
        ],
      },
      {
        title: "Streaming chart regression",
        harness: "claude",
        repo: "rimo-frontend",
        status: "needs you",
        tone: "warning",
        elapsed: "12m",
        lines: [
          { kind: "assistant", text: "The chart drops frames because every tick re-sorts the series. I can memoise it, but the fix touches shared state." },
          {
            kind: "notice",
            edge: "warning",
            title: "These paths weren't pre-approved by you.",
            status: "waiting for you",
            text: "The worker proposed them itself, so Bridge needs a one-time authorization.",
            actions: ["Decline", "Allow once"],
          },
        ],
      },
      {
        title: "Implementation · strong",
        harness: "cursor",
        repo: "atlas-api",
        status: "isolated",
        tone: "success",
        elapsed: "3m",
        lines: [
          { kind: "collapsed", label: "Edited tokenStore.ts · +9 −4" },
          { kind: "rail", label: "Worktree", status: "clean", text: ".worktrees/worker-2f9a" },
          { kind: "notice", edge: "info", title: "Ran bun test src/auth", status: "exit 0", text: "42 passed, 0 failed." },
        ],
      },
    ],
  },
  {
    id: "worktrees",
    label: "Isolated worktrees",
    title: "Every worker in its own tree",
    text: "Parallel agents never share a dirty tree. The diff, per-file risk, and the changes dock all read from the worker's own worktree, and you adopt or discard the result.",
    view: "chat",
    toolbar: { title: "Worktree lifecycle inventory", subtitle: "bridge-harness · feat/worktree-lifecycle" },
    prompt: "Run the usage rollup refactor in parallel with the lifecycle fix.",
    entries: [
      { kind: "assistant", text: "Their write scopes overlap, so each worker gets its own worktree rather than sharing yours." },
      {
        kind: "notice",
        edge: "info",
        title: "Worktree created",
        status: "isolated",
        text: "A branch and a checkout of its own, cut from the task worktree.",
        caption: "Worktree",
        code: ".worktrees/worker-2f9a  ·  feat/usage-rollups  ·  main @ 8d0babe",
      },
      {
        kind: "activity",
        summary: "Ran 1 command, read 1 file, edited 1 file",
        steps: "3 steps · 2s",
        rows: [
          { label: "Read", path: "worktree_coordinator.rs" },
          { label: "Edited", path: "worktree_coordinator.rs", stat: "+12 −9" },
          { label: "Ran", path: "cargo test -p bridge-core worktree::", stat: "exit 0" },
        ],
        diff: codeDiff,
      },
      {
        kind: "notice",
        edge: "success",
        title: "Worker changes are ready to review",
        status: "2 files",
        text: "Adopt merges this worker's changes into your workspace. Discard deletes them.",
        actions: ["Discard", "Adopt changes"],
      },
    ],
    dock: {
      origin: "feat/worktree-lifecycle · uncommitted vs HEAD (8d0babe)",
      added: 303,
      removed: 23,
      files: [
        { dir: "src-tauri/bridge-core/src/", name: "worktree_coordinator.rs", added: 18, removed: 4, risk: "High", lang: "rust" },
        { dir: "src/components/", name: "ChangesPanel.tsx", added: 42, removed: 6, risk: "Medium", lang: "frontend" },
        { dir: "src/", name: "utils.ts", added: 3, removed: 1, risk: "Low", lang: "frontend" },
      ],
    },
  },
  {
    id: "fleet",
    label: "Orchestrate a fleet",
    title: "One orchestrator, many workers",
    text: "The orchestrator plans and routes. Workers on Codex, Claude Code, Cursor, and OpenCode take bounded slices and hand back typed results with checkpoints.",
    view: "chat",
    toolbar: { title: "Worktree lifecycle inventory", subtitle: "bridge-harness · feat/worktree-lifecycle" },
    prompt: "Reclaimed branches still show as active. Trace it, fix it, and have a second harness verify.",
    entries: [
      { kind: "assistant", text: "Delegating implementation and verification to two different harnesses so neither closes its own task." },
      { kind: "worker", harness: "claude", label: "Implementation · strong", status: "working", tone: "success", text: "Isolated worktree, write scope src-tauri/bridge-core/**" },
      { kind: "worker", harness: "codex", label: "Verification · strong", status: "queued", tone: "warning", text: "Read-only, starts when the implementer lands" },
      {
        kind: "notice",
        edge: "success",
        title: "Worker result",
        status: "typed",
        text: "Reordered reclaim after the forest append and added a regression test.",
        caption: "2 changed files · cargo test -p bridge-core worktree::",
      },
      { kind: "rail", label: "Checkpoint", status: "Saved", text: "Reclaim ordering and its regression test" },
    ],
    dock: {
      origin: "feat/worktree-lifecycle · uncommitted vs HEAD (8d0babe)",
      added: 38,
      removed: 12,
      files: [
        { dir: "src-tauri/bridge-core/src/", name: "worktree_coordinator.rs", added: 12, removed: 9, risk: "High", lang: "rust" },
        { dir: "src-tauri/bridge-core/tests/", name: "worktree_reclaim.rs", added: 26, removed: 3, risk: "Low", lang: "rust" },
      ],
    },
  },
  {
    id: "policy",
    label: "Policy owns the gates",
    title: "Nothing widens without you",
    text: "Write scope, capability tier, isolation, budgets, approvals. When a worker proposes paths you never granted, Bridge stops and asks for a one-time authorization.",
    view: "chat",
    toolbar: { title: "Streaming chart regression", subtitle: "rimo-frontend · rimo/chart-regression" },
    prompt: "Let the worker touch the sidebar too, and push to main once tests pass.",
    entries: [
      { kind: "assistant", text: "Two requests, two gates. Widening the write scope needs a new grant, and merging to main is approval-only at this tier." },
      {
        kind: "notice",
        edge: "warning",
        title: "These paths weren't pre-approved by you.",
        status: "waiting for you",
        text: "The worker proposed them itself, so Bridge needs a one-time authorization.",
        caption: "Write scope",
        code: "src/components/**\nsrc/index.css",
        actions: ["Decline", "Allow once"],
      },
      {
        kind: "notice",
        edge: "warning",
        title: "Merge to main needs your approval",
        status: "approval-only",
        text: "At this capability tier a merge is never automatic, however green the checks are.",
        actions: ["Not now", "Approve merge"],
      },
      { kind: "rail", label: "Recorded", status: "Saved", text: "Both decisions are durable either way" },
    ],
    dock: {
      origin: "rimo/chart-regression · uncommitted vs HEAD (a1b2c3d)",
      added: 64,
      removed: 18,
      files: [
        { dir: "src/components/", name: "StreamingChart.tsx", added: 48, removed: 12, risk: "Medium", lang: "frontend" },
        { dir: "src/", name: "series.ts", added: 16, removed: 6, risk: "Low", lang: "frontend" },
      ],
    },
  },
  {
    id: "verify",
    label: "Verify across harnesses",
    title: "No worker closes its own task",
    text: "A completion gate can demand deterministic checks plus scrutiny from a different harness family. The evidence is recorded before the merge ever waits on you.",
    view: "chat",
    toolbar: { title: "Worktree lifecycle inventory", subtitle: "bridge-harness · feat/worktree-lifecycle" },
    prompt: "Don't merge until a different harness has verified it.",
    entries: [
      { kind: "assistant", text: "Claude implemented, so the gate routes scrutiny to Codex. Same-family evidence is rejected by policy." },
      {
        kind: "checks",
        title: "Verifying",
        status: "2 of 4 required checks passed",
        revision: "Revision 307729bf075c · clean",
        checks: [
          { name: "cargo test", kind: "Deterministic", state: "passed", detail: "216 tests passed" },
          { name: "bun run build", kind: "Deterministic", state: "passed" },
          { name: "scrutiny", kind: "Scrutiny", state: "running", family: "codex" },
          { name: "user-journey", kind: "User testing", state: "pending", family: "claude" },
        ],
      },
      {
        kind: "notice",
        edge: "success",
        title: "Completion gate passed",
        status: "cross-family",
        text: "Claude implemented and Codex verified, both as typed results carrying file evidence.",
      },
      { kind: "rail", label: "Checkpoint", status: "Saved", text: "Verified boundary at turn 14" },
    ],
    dock: {
      origin: "feat/worktree-lifecycle · uncommitted vs HEAD (8d0babe)",
      added: 38,
      removed: 12,
      files: [
        { dir: "src-tauri/bridge-core/src/", name: "worktree_coordinator.rs", added: 12, removed: 9, risk: "High", lang: "rust" },
        { dir: "src-tauri/bridge-core/tests/", name: "worktree_reclaim.rs", added: 26, removed: 3, risk: "Low", lang: "rust" },
      ],
    },
  },
  {
    id: "usage",
    label: "Track usage",
    title: "Tokens and cost, per harness",
    text: "Imported from your local Claude, Codex, and OpenCode history. Cost at API rates, cache savings, and a per-model breakdown, with budgets enforced as gates.",
    view: "usage",
  },
];

export type UsageRow = { model: string; harness: HarnessId; cost: string; share: number; tokens: string };

export const usage = {
  total: "$7.33",
  caption: "605 requests · API estimate · Partly unpriced",
  harnesses: [
    { harness: "claude" as HarnessId, label: "Claude", cost: "$4.17", share: 57, tokens: "3.19M" },
    { harness: "codex" as HarnessId, label: "Codex", cost: "$2.60", share: 35.5, tokens: "3.58M" },
    { harness: "opencode" as HarnessId, label: "OpenCode", cost: "$0.56", share: 7.6, tokens: "708K" },
  ],
  stats: [
    { label: "Processed tokens", value: "7.48M" },
    { label: "Cached input", value: "6.04M" },
    { label: "Uncached input", value: "776K" },
    { label: "Output", value: "280K" },
    { label: "Reasoning", value: "112K" },
    { label: "Cache savings", value: "$12.19" },
  ],
  rows: [
    { model: "claude-fable-5-1", harness: "claude" as HarnessId, cost: "$4.17", share: 57, tokens: "3.19M" },
    { model: "gpt-5.6-luna", harness: "codex" as HarnessId, cost: "$2.60", share: 35.5, tokens: "3.31M" },
    { model: "opencode/big-pickle", harness: "opencode" as HarnessId, cost: "$0.56", share: 7.6, tokens: "708K" },
  ],
  /** Deterministic so server and client markup agree. */
  chart: [18, 34, 22, 46, 30, 58, 41, 67, 38, 72, 49, 61, 44, 78, 52, 66, 35, 59, 47, 74],
};
