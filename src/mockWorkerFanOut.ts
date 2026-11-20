import type { Session, SessionEntry, SessionForestSnapshot } from "./types";

// Mock mode's delegation: the "Polish the Deck shell" orchestrator fanned out
// to three workers, one per surface, so `bun run dev` draws the fan-out card
// and the Agents pane lineage with a worker in each state that matters: one
// working, one waiting on an approval, one done. Its first worker,
// `session-2w`, is the demo's existing live worker; this adds the other two.

const ago = (now: string, seconds: number) => new Date(Date.parse(now) - seconds * 1000).toISOString();
const WORKTREES = "/Users/you/bridge/Lisbon/.worktrees";

/** The two workers the demo orchestrator started beside `session-2w`. */
export function mockDeckWorkers(now: string): Session[] {
  const worker = { workspaceId: "demo-2", contextPercent: 11, usagePercent: 4, metricSource: "reported", parentSessionId: "session-2", depth: 1, restorationMode: "native", continuationFidelity: "native", kind: "worker" } as const;
  return [
    { ...worker, id: "session-2w2", harness: "claude", label: "Implementation · strong", title: "Dock work pane", status: "waiting", startedAt: ago(now, 41), endedAt: null, providerSessionId: "mock-claude-2w2", activeTurnId: "mock-turn-2w2", model: "sonnet", requestedTier: "strong", effort: "high" },
    { ...worker, id: "session-2w3", harness: "codex", label: "Implementation · standard", title: "Composer footer", status: "completed", startedAt: ago(now, 72), endedAt: ago(now, 36), providerSessionId: "mock-codex-2w3", activeTurnId: null, model: "gpt-5.6-terra", requestedTier: "standard", effort: "medium" },
  ];
}

function entry(now: string, id: string, sequence: number, kind: string, payload: Record<string, unknown>, parentEntryId: string | null, secondsAgo: number): SessionEntry {
  return { id, sessionId: "session-2", parentEntryId, sequence, semanticSchemaVersion: 2, kind, payload, providerEventId: null, contextVisibility: "eligible", tokenEstimate: null, createdAt: ago(now, secondsAgo) };
}

/** The orchestrator's history: the ask, the plan, three spawns, one result and one blocked worker. */
export function mockDeckForest(now: string): SessionForestSnapshot {
  const spawn = (child: string, label: string, objective: string, data: Record<string, unknown>) =>
    ({ itemId: `spawn-${child}`, role: "system", status: "working", title: `Delegated to ${label}`, text: objective, data: { childSessionId: child, depth: 1, ...data } });
  const entries = [
    entry(now, "deck-user", 1, "user.message", { itemId: "deck-user", text: "Split the Deck shell polish across workers: the sidebar running section, the dock work pane, and the composer footer." }, null, 80),
    entry(now, "deck-plan", 2, "assistant.message", { text: "Delegating to three workers, one per surface, each in its own worktree. I'll review and adopt their changes when they finish." }, "deck-user", 76),
    entry(now, "deck-spawn-2w3", 3, "delegation.spawned", spawn("session-2w3", "Implementation · standard", "Rework the composer footer hints for idle and running turns.", { harness: "codex", model: "gpt-5.6-terra", modelLabel: "GPT Terra", effort: "medium" }), "deck-plan", 72),
    entry(now, "deck-spawn-2w2", 4, "delegation.spawned", spawn("session-2w2", "Implementation · strong", "Turn the dock Work pane into the chat overview.", { harness: "claude", model: "sonnet", modelLabel: "Sonnet", effort: "high" }), "deck-spawn-2w3", 41),
    entry(now, "deck-spawn-2w", 5, "delegation.spawned", spawn("session-2w", "Docs · fast", "Add a Running section above Projects in the sidebar.", { harness: "opencode", modelLabel: "Space Bunny" }), "deck-spawn-2w2", 1),
    entry(now, "deck-result-2w3", 6, "delegation.result", { itemId: "result-2w3", role: "system", status: "completed", title: "Worker result", text: "[worker result] Implementation · standard finished:\n\nComposer hints follow the turn state.", data: { childSessionId: "session-2w3", delivered: true, status: "completed" } }, "deck-spawn-2w", 36),
    entry(now, "deck-blocked-2w2", 7, "delegation.blocked", { role: "system", status: "waiting", title: "Dock work pane wants to run bun install", text: "Run bun install to add the dock's dependencies?", data: { childBlocked: true, childSessionId: "session-2w2", label: "Implementation · strong", objective: "Turn the dock Work pane into the chat overview.", command: "bun install", cwd: `${WORKTREES}/deck-dock`, ownedPaths: ["src/components/**"], orchestratorNotified: true } }, "deck-result-2w3", 10),
  ];
  const head = entries.at(-1)!;
  const runtime = { parentSessionId: "session-2", compatibilityKey: "demo", retryCount: 0, warmUntil: null, lastActivityAt: now } as const;
  return {
    sessionId: "session-2",
    entries,
    head: { sessionId: "session-2", activeEntryId: head.id, nativeProviderSessionId: "mock-thread-2", restorationMode: "fresh", resumeEligibility: "native", latestCheckpointEntryId: null, updatedAt: now },
    leaves: [head],
    workerLeases: [],
    workerRuntimes: [
      { ...runtime, sessionId: "session-2w", lifecycleState: "working", taskFamily: "documentation", resultStatus: "pending", worktreePath: `${WORKTREES}/deck-sidebar`, worktreeBranch: "bridge/deck-sidebar", lastResult: null, progressSummary: "editing src/components/BridgeSidebar.tsx", updatedAt: now },
      { ...runtime, sessionId: "session-2w2", lifecycleState: "waiting", taskFamily: "implementation", resultStatus: "pending", worktreePath: `${WORKTREES}/deck-dock`, worktreeBranch: "bridge/deck-dock", lastResult: null, waitingReason: "approval_required", waitingSince: ago(now, 10), updatedAt: now },
      { ...runtime, sessionId: "session-2w3", lifecycleState: "completed", taskFamily: "implementation", resultStatus: "reported", worktreePath: `${WORKTREES}/deck-composer`, worktreeBranch: "bridge/deck-composer", lastResult: { status: "completed", summary: "Composer hints follow the turn state.", filesChanged: ["src/components/ComposerPill.tsx", "src/keymap.ts", "src/components/ComposerPill.test.tsx"] }, updatedAt: ago(now, 36) },
    ],
    workerQueue: [],
    usage: [],
    reasons: [],
    policyLimits: { maxWorkersPerTurn: 3, maxStrongWorkersPerTurn: 1, maxCapabilityUnitsPerTurn: 24 },
    repositoryDivergence: { status: "unknown", selectedState: null, currentState: { status: "unavailable" } },
    completion: null,
    entryWindow: { returned: entries.length, total: entries.length, trimmedPayloads: 0 },
  };
}
