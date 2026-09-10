export type Capability = {
  id: string;
  label: string;
  title: string;
  text: string;
  image: string;
  alt: string;
};

/*
 * The hero switcher. Each entry is a real capture of the Bridge frontend, taken from the
 * app's own preview mode at 1600×1000 and 2× scale, so the hero shows the product as it
 * is today rather than an approximation. Recapture when the app redesigns.
 */
export const capabilities: Capability[] = [
  {
    id: "worktrees",
    label: "Isolated worktrees",
    title: "Every worker in its own tree",
    text: "Parallel agents never share a dirty tree. Diffs, per-file risk, and the changes dock read from the worker's worktree, and you adopt or discard the result.",
    image: "/screens/worktrees.webp",
    alt: "The Bridge changes dock listing four changed files with risk labels beside a worker's inline diff.",
  },
  {
    id: "agent-fleet",
    label: "Agent Fleet",
    title: "Every agent CLI, one screen",
    text: "A terminal grid per checkout. Each new agent splits into the same screen beside the others, panes drag into place, and layout and scrollback persist across restarts.",
    image: "/screens/agent-fleet.webp",
    alt: "Bridge Agent Fleet with a shell split into three panes and tabs for Claude Code and Codex agent CLIs in the same workspace.",
  },
  {
    id: "mission-control",
    label: "Mission Control",
    title: "Every live chat, in motion",
    text: "All active conversations at once, each one the real chat with its transcript, approvals, and composer, just resized into a grid you can rearrange.",
    image: "/screens/mission-control.webp",
    alt: "Bridge Mission Control showing several live agent conversations side by side, each with its own composer.",
  },
  {
    id: "fleet",
    label: "Orchestrate a fleet",
    title: "One orchestrator, many workers",
    text: "The orchestrator plans and routes. Workers on Codex, Claude Code, Cursor, and OpenCode take bounded slices; the Tasks pane shows who is working, done, or queued.",
    image: "/screens/fleet.webp",
    alt: "A Bridge orchestrator delegating implementation and verification, with the Tasks pane listing a working implementer, a finished verifier, and a queued task.",
  },
  {
    id: "verify",
    label: "Verify across harnesses",
    title: "No worker closes its own task",
    text: "A completion gate can require deterministic checks plus scrutiny from a different harness family. Evidence is recorded, and the GitHub pane shows PR checks beside it.",
    image: "/screens/verify.webp",
    alt: "A Bridge verification record showing cargo test and bun run build passed, with Claude scrutiny running and Codex user testing pending.",
  },
  {
    id: "policy",
    label: "Policy owns the gates",
    title: "Nothing widens without you",
    text: "Write scope, capability tier, isolation, budgets, approvals. When a worker proposes paths you never granted, Bridge stops and asks for a one-time authorization.",
    image: "/screens/policy.webp",
    alt: "An approval card in Bridge asking to allow a worker's proposed write scope once, beside the raw event stream in the Transcript pane.",
  },
  {
    id: "usage",
    label: "Track usage",
    title: "Tokens and cost, per harness",
    text: "Imported from local Claude, Codex, and OpenCode history. Cost at API rates, cache savings, and a per-model breakdown, with budgets enforced as gates.",
    image: "/screens/usage.webp",
    alt: "The Bridge usage page with cost per harness, a daily cost chart, token totals, and a per-model breakdown.",
  },
];
