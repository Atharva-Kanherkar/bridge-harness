/*
 * The comparison page's facts. Kept as data so a row can be corrected in one place when a
 * competitor ships something new.
 *
 * Rules for this file, which the tests enforce:
 * - Every row carries one cell per product, in `products` order.
 * - Only claim what Bridge ships today. A feature on the roadmap does not go in a table.
 * - Competitor cells describe what that product publishes. Never guess.
 */

export const comparedOn = "September 2026";

export const products = [
  { id: "bridge", name: "Bridge", note: "This is us" },
  { id: "conductor", name: "Conductor", note: "conductor.build" },
  { id: "orca", name: "Orca", note: "onorca.dev" },
  { id: "superset", name: "Superset", note: "superset.sh" },
  { id: "t3", name: "T3 Code", note: "t3.codes" },
] as const;

export type CompareRow = { label: string; hint?: string; cells: string[] };
export type CompareTable = { id: string; title: string; text: string; rows: CompareRow[] };

export const licenseTable: CompareTable = {
  id: "open-source",
  title: "Open source, checked line by line",
  text: "Free to run is not the same as free to own. These are the licenses and the price gates.",
  rows: [
    {
      label: "License",
      hint: "The terms you actually get.",
      cells: ["MIT", "Closed source", "MIT", "Elastic License 2.0", "MIT"],
    },
    {
      label: "Read the source",
      hint: "Every line of the app, not a plugin API.",
      cells: ["Yes", "No", "Yes", "Yes", "Yes"],
    },
    {
      label: "Fork it and ship your own build",
      cells: ["Yes", "No", "Yes", "Limits apply", "Yes"],
    },
    {
      label: "Price for local parallel agents",
      cells: ["Free", "Free", "Free", "Free", "Free"],
    },
    {
      label: "Features locked behind a paid plan",
      hint: "Published plans, not our estimate.",
      cells: ["None", "Pro, 50 USD per month", "None published", "Pro, 20 USD per seat", "None"],
    },
    {
      label: "Your code and credentials leave the machine",
      cells: ["Never", "Cloud workspaces on Pro", "SSH worktrees, opt in", "Remote access on Pro", "Web and mobile relay"],
    },
  ],
};

export const controlTable: CompareTable = {
  id: "supervision",
  title: "Supervision, where Bridge is alone",
  text: "Every product here runs agents in parallel. Only one of them governs what those agents may do.",
  rows: [
    {
      label: "Policy engine on every gate",
      hint: "Write scope, worktree isolation, concurrency, delegation depth, retries, per-turn budgets.",
      cells: ["Yes", "Not offered", "Not offered", "Not offered", "Not offered"],
    },
    {
      label: "Agents delegate to worker agents",
      hint: "An orchestrator hands a typed objective to a worker, inside a scope you approved.",
      cells: ["Yes", "Not offered", "Not offered", "Not offered", "Not offered"],
    },
    {
      label: "A different model family must verify the work",
      hint: "The completion gate excludes the harness that wrote the code from passing it.",
      cells: ["Yes", "Not offered", "Not offered", "Not offered", "Not offered"],
    },
    {
      label: "Approvals rendered as app UI",
      hint: "Risky commands, out-of-scope writes, and delegation pause for one click.",
      cells: ["Yes", "Agent's own prompts", "Agent's own prompts", "Agent's own prompts", "Agent's own prompts"],
    },
    {
      label: "Routing can never widen a permission",
      hint: "Model choice ranks what policy already allows. It cannot grant anything.",
      cells: ["Guaranteed", "Not applicable", "Not applicable", "Not applicable", "Not applicable"],
    },
  ],
};

export const workTable: CompareTable = {
  id: "your-work",
  title: "Your history, your spend, your models",
  text: "What happens after the agent stops typing. This is where a terminal wrapper runs out.",
  rows: [
    {
      label: "Append-only local history you can rewind and fork",
      hint: "Immutable entries in SQLite on your disk, survivable across restarts.",
      cells: ["Yes", "Provider session files", "Provider session files", "Provider session files", "Provider session files"],
    },
    {
      label: "Switch model or provider inside one conversation",
      hint: "The provider session restarts. Your thread stays.",
      cells: ["Yes", "Chosen per workspace", "Chosen per workspace", "Chosen per workspace", "Chosen per workspace"],
    },
    {
      label: "Cost, rate limits, and context pressure per provider",
      hint: "Cost per harness and per model, plus what the cache saved you.",
      cells: ["Built in", "Not offered", "Not offered", "Not offered", "Not offered"],
    },
    {
      label: "Memory that carries across agents and repositories",
      hint: "Preferences and project decisions recalled into any session.",
      cells: ["Built in", "Agent files only", "Agent files only", "Agent files only", "Agent files only"],
    },
    {
      label: "Lend an agent a logged-in browser tab",
      hint: "You approve one tab. No password, cookie, or profile is ever copied.",
      cells: ["Yes", "Not offered", "Not offered", "Not offered", "Not offered"],
    },
    {
      label: "Typed protocol with generated schemas",
      hint: "JSON-RPC over a local socket, so CI and scripts drive the same runtime the app does.",
      cells: ["Yes, free", "API on Pro", "CLI wrapper", "CLI wrapper", "Not published"],
    },
  ],
};

export const parityTable: CompareTable = {
  id: "table-stakes",
  title: "Table stakes, all matched",
  text: "The things every serious tool in this category does. Bridge does them too.",
  rows: [
    { label: "Run several coding agents at once", cells: ["Yes", "Yes", "Yes", "Yes", "Yes"] },
    { label: "An isolated Git worktree and branch per task", cells: ["Yes", "Yes", "Yes", "Yes", "Yes"] },
    { label: "Uses the subscriptions you already pay for", cells: ["Yes", "Yes", "Yes", "Yes", "Yes"] },
    { label: "Review the diff before anything lands", cells: ["Yes", "Yes", "Yes", "Yes", "Yes"] },
    { label: "Built-in terminals for the CLIs themselves", cells: ["Yes", "Yes", "Yes", "Yes", "Yes"] },
  ],
};

export const tables = [licenseTable, controlTable, workTable, parityTable];

export const edges = [
  {
    title: "MIT, all the way down",
    body: "The whole app is MIT licensed. No source-available asterisk, no clause about what you may build with it, no seat that unlocks the real product.",
  },
  {
    title: "Rules, not vibes",
    body: "Scope, isolation, concurrency, depth, retries, and budgets are enforced by one policy engine. Nothing in the app can widen a permission it did not grant.",
  },
  {
    title: "Nobody marks their own homework",
    body: "A completion gate can demand that a different model family verifies the change. Evidence from the harness that wrote the code is rejected.",
  },
  {
    title: "One thread, every model",
    body: "Change model or provider in the middle of a conversation. The provider process restarts underneath. Your history and context stay put.",
  },
  {
    title: "History you own",
    body: "Every message, plan, tool call, and approval lands in an append-only local store. Rewind to any point, fork it, and try a second approach.",
  },
  {
    title: "Spend visible before the wall",
    body: "Cost per harness and per model, rate-limit status, and context pressure sit in the app. You switch or wrap up before quality drops.",
  },
];

export const scope = "Bridge is a native desktop app for macOS. Everything runs on your machine.";
