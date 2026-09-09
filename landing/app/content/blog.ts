export type PostBlock =
  | { kind: "paragraph"; text: string }
  | { kind: "heading"; text: string }
  | { kind: "list"; items: string[] }
  | { kind: "code"; code: string }
  | { kind: "aside"; text: string };

export type Post = {
  slug: string;
  title: string;
  summary: string;
  date: string;
  iso: string;
  blocks: PostBlock[];
};

export const posts: Post[] = [
  {
    slug: "three-trees-kept-apart",
    title: "Three trees, kept apart",
    summary:
      "A workspace, an agent, and a conversation each form a hierarchy. Conflating any two of them is the design bug that keeps coming back.",
    date: "September 8, 2026",
    iso: "2026-09-08",
    blocks: [
      {
        kind: "paragraph",
        text: "Bridge keeps three kinds of hierarchy, and the recurring design mistake is treating them as one. The workspace tree runs from a repository to a task worktree to a worker worktree. The agent tree runs from an orchestrator to the workers its policy authorized. The conversation tree runs from immutable entries to whichever branch is currently active.",
      },
      {
        kind: "paragraph",
        text: "They are related. They are not the same shape, they do not change together, and an operation on one does not imply a matching operation on another.",
      },
      { kind: "heading", text: "What that rules out" },
      {
        kind: "paragraph",
        text: "Forking a conversation changes the active history branch. It does not undo a filesystem change, revert a commit, or roll back provider state. Ending a session does not discard a worktree. Reclaiming a worktree is a decision the coordinator makes against the filesystem, never a side effect of a conversation moving.",
      },
      {
        kind: "paragraph",
        text: "That last one has teeth. Because branch refs survive reclamation, losing a checkout never loses a session its project.",
      },
      { kind: "heading", text: "Making divergence visible" },
      {
        kind: "paragraph",
        text: "If the trees can move independently, they can disagree, and the honest thing is to say so rather than paper over it. Every controller append stamps its entry with the repository HEAD and a deterministic hash of the full dirty state. A snapshot compares the selected entry's stamp against the current worktree, and a mismatch surfaces as conversation and file divergence.",
      },
      {
        kind: "paragraph",
        text: "Entries that predate the stamp are reported as unknown rather than assumed aligned. The stamp belongs to the controller, so it is stripped before anything is projected into an agent's context.",
      },
      { kind: "heading", text: "Why the separation is worth the cost" },
      {
        kind: "paragraph",
        text: "A single tree would be simpler to explain and would lie constantly. It would let a rewind imply a revert, and let ending a chat quietly delete work. Keeping the three apart costs a stamp on every append and a reconciliation path for every restart. What it buys is that no operation silently means more than it says.",
      },
      {
        kind: "aside",
        text: "The storage model and its projection rules are in docs/session-forest.md, and the retention policy that reclaims checkouts is in docs/worktree-lifecycle.md.",
      },
    ],
  },
  {
    slug: "agents-request-rust-decides",
    title: "Agents may request. Rust decides.",
    summary:
      "Every delegation crosses a typed boundary into a policy engine that returns one auditable outcome. The learning router ranks what policy already allows, and can never widen it.",
    date: "September 5, 2026",
    iso: "2026-09-05",
    blocks: [
      {
        kind: "paragraph",
        text: "An agent in Bridge can ask for help. It cannot grant itself the right to receive it. A delegation request crosses the adapter boundary as a typed envelope carrying role, objective, acceptance criteria, known facts, owned paths, write mode, capability tier, effort, verification steps, and an output contract. On the other side, Rust decides.",
      },
      { kind: "heading", text: "Six outcomes, one of them auditable" },
      {
        kind: "paragraph",
        text: "The policy engine weighs task family, compatible warm workers, requested capability tier, per-turn budget, active leases, owned-path overlap, retry count, and the previous outcome. It returns exactly one decision.",
      },
      {
        kind: "list",
        items: [
          "Execute in the parent",
          "Resume a compatible worker",
          "Spawn a new worker",
          "Queue the request",
          "Reject it",
          "Require human approval",
        ],
      },
      {
        kind: "paragraph",
        text: "There is no seventh outcome where a sufficiently confident model talks its way past the gate.",
      },
      { kind: "heading", text: "Blocked is not failed" },
      {
        kind: "paragraph",
        text: "A background worker's approval card renders on the worker's own conversation, which is usually not the one you are looking at. So Bridge mirrors it onto the parent with the worker label, objective, command, working directory, and owned-path scope, and tells the parent its child is blocked rather than failed.",
      },
      {
        kind: "paragraph",
        text: "Waiting on a person is legitimately idle, so it is excluded from the stall watchdog and given its own deadline of thirty minutes. Past that the worker resolves to a terminal blocked result naming the unanswered approval, which releases the parent instead of stranding it. A queued request whose ancestor is waiting on a human moves to a durable blocked state where it can neither dispatch nor expire, and resolution advances its expiry by the full blocked duration.",
      },
      { kind: "heading", text: "Failing before the reservation" },
      {
        kind: "paragraph",
        text: "An adapter descriptor declares which sandbox modes its harness can actually start in, including transport constraints. The router excludes an incompatible harness before any reservation is made, so a route guaranteed to fail at adapter startup never creates a worker session at all. A pinned incompatible route returns an actionable error naming the harness, the sandbox mode, and the alternatives that would work.",
      },
      {
        kind: "paragraph",
        text: "OpenCode, for instance, declares no read-only support, because its localhost HTTP transport cannot run inside the offline read-only sandbox. The adapter keeps its own fail-closed guard anyway, as defense in depth.",
      },
      { kind: "heading", text: "The router ranks. It does not grant." },
      {
        kind: "paragraph",
        text: "Every route records the full candidate inventory, reason-coded exclusions, a conservative prediction, the baseline, the recommendation, the executed candidate, the deterministic policy outcome, and the eventual worker outcome. Predictions combine tier priors with durable task-family history for pass probability, latency, normalized quota cost, and retry risk.",
      },
      {
        kind: "paragraph",
        text: "Sparse history stays visibly prior-weighted rather than turning missing data into certainty. And all of it only ever reorders candidates the policy has already found eligible. Learning can make a better choice among permitted options. It cannot create one.",
      },
    ],
  },
  {
    slug: "who-owns-the-context-window",
    title: "The harness owns its context. Bridge owns the record.",
    summary:
      "Bridge does not compact a live provider window. Three facts settled that, and one of them was that our own pressure trigger had never once fired.",
    date: "September 7, 2026",
    iso: "2026-09-07",
    blocks: [
      {
        kind: "paragraph",
        text: "Within a live process the harness owns its context window, and Bridge does not compact it. Bridge owns the durable record of what the harness did, and the checkpoint that outlives the process. That split was a decision, not an accident, and three facts settled it.",
      },
      { kind: "heading", text: "The trigger that never fired" },
      {
        kind: "paragraph",
        text: "Bridge had its own context-pressure trigger. Across a month of real use it never fired once. Every successful compaction in that period was a phase boundary, a model switch, or a shutdown, which are all events Bridge already knows about without measuring pressure.",
      },
      { kind: "heading", text: "A checkpoint frees nothing" },
      {
        kind: "paragraph",
        text: "Committing a Bridge checkpoint writes forest entries and moves the session head. It does not touch the adapter, so it frees no provider tokens. Making it free tokens would mean restarting the process to replay a shorter history, which throws away the native session and the prompt-cache prefix that native resume and the byte-stable system prompt exist to preserve.",
      },
      { kind: "heading", text: "The harness is the only layer that can" },
      {
        kind: "paragraph",
        text: "The harness is what talks to the model, so it is the only layer that can shrink the request actually being sent. All three managed harnesses already do, and each reports it in its own vocabulary. Claude Code emits a compact boundary carrying its trigger and token counts before and after. Codex emits a context compaction thread item. OpenCode emits a session compacted event. Bridge normalizes all three rather than competing with them.",
      },
      { kind: "heading", text: "What a resume is allowed to claim" },
      {
        kind: "paragraph",
        text: "Persisting an entry is not the same as injecting it into a provider thread, so a returning session has to say which of four things happened.",
      },
      {
        kind: "list",
        items: [
          "Hot, meaning the adapter process is still running",
          "Native, meaning the provider resumed its own stored thread",
          "Checkpoint restored, meaning native resume was unavailable or failed and Bridge started fresh from a validated projection",
          "Fresh, meaning the task deliberately began with no prior context",
        ],
      },
      {
        kind: "paragraph",
        text: "A failed native resume is recorded before the checkpoint fallback runs. Bridge never labels a fresh process as natively resumed, because the difference is exactly the thing a user needs to know when the agent seems to have forgotten something.",
      },
      {
        kind: "aside",
        text: "The restoration modes, the per-harness compaction table, and the ownership split are in docs/compaction-and-resume.md.",
      },
    ],
  },
];

export function postBySlug(slug: string) {
  return posts.find((post) => post.slug === slug);
}
