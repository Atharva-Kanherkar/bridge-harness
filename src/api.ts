import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { AgentEvent, BridgeState, Harness, Health, SessionEntry, SessionForestSnapshot, TerminalChunk } from "./types";
import { safeSlug } from "./utils";

const isTauri = () => typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
const now = new Date().toISOString();
const stateListeners = new Set<() => void>();
let nextEventId = 20;

let mockState: BridgeState & { agentEvents: AgentEvent[] } = {
  projects: [{ id: "demo-project", name: "Bridge", path: "/Users/you/Developer/bridge", createdAt: now }],
  workspaces: [
    { id: "demo-1", projectId: "demo-project", city: "Kyoto", title: "Build session supervisor", branch: "bridge/session-supervisor", path: "/Users/you/bridge/Kyoto", status: "working", dirtyFiles: 4, additions: 284, deletions: 31, createdAt: now },
    { id: "demo-2", projectId: "demo-project", city: "Lisbon", title: "Polish the Deck shell", branch: "bridge/deck-shell", path: "/Users/you/bridge/Lisbon", status: "ready", dirtyFiles: 7, additions: 612, deletions: 88, createdAt: now },
    { id: "demo-3", projectId: "demo-project", city: "Reykjavik", title: "Add event ledger", branch: "bridge/event-ledger", path: "/Users/you/bridge/Reykjavik", status: "ready", dirtyFiles: 0, additions: 148, deletions: 12, createdAt: now }
  ],
  sessions: [
    { id: "session-1", workspaceId: "demo-1", harness: "codex", label: "Orchestrator", status: "working", startedAt: now, endedAt: null, contextPercent: 38, usagePercent: 24, metricSource: "reported", providerSessionId: "mock-thread-1", activeTurnId: "mock-turn-1", model: "gpt-5.6-luna", requestedTier: "fast", effort: null, parentSessionId: null, depth: 0, restorationMode: "hot" },
    { id: "session-1w", workspaceId: "demo-1", harness: "claude", label: "Implementation · strong", status: "working", startedAt: now, endedAt: null, contextPercent: 21, usagePercent: 14, metricSource: "reported", providerSessionId: "mock-claude-1", activeTurnId: "mock-turn-1w", model: "fable", requestedTier: "strong", effort: "high", parentSessionId: "session-1", depth: 1, restorationMode: "native" },
    { id: "session-1w2", workspaceId: "demo-1", harness: "codex", label: "Verification · strong", status: "ready", startedAt: now, endedAt: null, contextPercent: 9, usagePercent: 6, metricSource: "reported", providerSessionId: "mock-codex-2", activeTurnId: null, model: "gpt-5.6-sol", requestedTier: "strong", effort: "xhigh", parentSessionId: "session-1", depth: 1, restorationMode: "checkpoint_restored" },
    { id: "session-2", workspaceId: "demo-2", harness: "codex", label: "Orchestrator", status: "ready", startedAt: now, endedAt: null, contextPercent: 12, usagePercent: 8, metricSource: "reported", providerSessionId: "mock-thread-2", activeTurnId: null, model: "gpt-5.6-luna", requestedTier: "fast", effort: null, parentSessionId: null, depth: 0, restorationMode: "fresh" }
  ],
  events: [
    { id: 2, source: "git", kind: "workspace.changed", entityId: "demo-1", body: "4 files changed · +284 −31", createdAt: now },
    { id: 1, source: "gate", kind: "workspace.ready", entityId: "demo-3", body: "Tests and typecheck passed.", createdAt: now }
  ],
  agentEvents: [
    agentEvent(1, "session-1", "message.completed", { itemId: "user-1", role: "user", status: "completed", text: "Build the structured session supervisor." }),
    agentEvent(2, "session-1", "plan.updated", { title: "Implementation plan", status: "inProgress", data: { plan: [{ step: "Define normalized harness primitives", status: "completed" }, { step: "Build the native conversation GUI", status: "inProgress" }, { step: "Verify the real Codex adapter", status: "pending" }] } }),
    agentEvent(3, "session-1", "tool.started", { itemId: "tool-1", title: "Inspect workspace", status: "completed", data: { type: "commandExecution", cwd: "/Users/you/bridge/Kyoto", aggregatedOutput: "src/api.ts\nsrc/App.tsx\nsrc-tauri/src/lib.rs" } }),
    agentEvent(4, "session-1", "message.completed", { itemId: "assistant-1", role: "assistant", status: "completed", text: "This is high-risk implementation work, so I’m delegating it to a strong-tier implementation worker at high effort." }),
    agentEvent(5, "session-1", "delegation.spawned", { itemId: "spawn-1w", role: "system", status: "working", title: "Delegated to Implementation · strong", text: "Refactor the auth module to use the new token store, then verify.", data: { childSessionId: "session-1w", harness: "claude", requestedTier: "strong", model: "fable", modelLabel: "Fable", effort: "high", depth: 1 } }),
    agentEvent(6, "session-1w", "message.completed", { itemId: "user-1w", role: "user", status: "completed", text: "Refactor the auth module to use the new token store, then verify." }),
    agentEvent(7, "session-1w", "message.completed", { itemId: "assistant-1w", role: "assistant", status: "completed", text: "Refactor done. The typed result suggests a separate verification worker." }),
    agentEvent(8, "session-1", "delegation.spawned", { itemId: "spawn-1w2", role: "system", status: "working", title: "Delegated to Verification · strong", text: "Run the auth test suite and confirm the token store migration is correct.", data: { childSessionId: "session-1w2", harness: "codex", requestedTier: "strong", model: "gpt-5.6-sol", modelLabel: "GPT Sol", effort: "xhigh", depth: 1 } }),
    agentEvent(9, "session-1w2", "message.completed", { itemId: "assistant-1w2", role: "assistant", status: "completed", text: "All 42 auth tests pass. Token store migration verified." }),
    agentEvent(10, "session-1", "delegation.result", { itemId: "result-1w2", role: "system", status: "completed", title: "Worker result", text: "[worker result] Verification · strong (STRONG TIER, runtime GPT Sol, effort xhigh) finished:\n\nAll 42 auth tests pass. Token store migration verified.", data: { childSessionId: "session-1w2", delivered: true } }),
    agentEvent(11, "session-1", "delegation.result", { itemId: "result-1w", role: "system", status: "completed", title: "Worker result", text: "[worker result] Implementation · strong (STRONG TIER, runtime Fable, effort high) finished:\n\nAuth module refactored to the new token store.", data: { childSessionId: "session-1w", delivered: true } })
  ]
};

function agentEvent(id: number, sessionId: string, kind: string, fields: Partial<AgentEvent> = {}): AgentEvent {
  return { id, sessionId, sequence: id, protocolVersion: 1, kind, itemId: null, role: null, status: null, title: null, text: null, data: {}, providerMeta: { adapter: "fake" }, createdAt: new Date().toISOString(), ...fields };
}
function forestEntry(id: string, sessionId: string, sequence: number, kind: string, payload: Record<string, unknown>, parentEntryId: string | null): SessionEntry {
  return { id, sessionId, parentEntryId, sequence, kind, payload, providerEventId: null, contextVisibility: "eligible", tokenEstimate: null, createdAt: now };
}
const demoEntries: SessionEntry[] = [
  forestEntry("entry-1", "session-1", 1, "user.message", { text: "Build the structured session supervisor." }, null),
  forestEntry("entry-2", "session-1", 2, "checkpoint", { schemaVersion: 1, summary: "Policy and schema decisions are durable", decisions: ["SQLite is authoritative"] }, "entry-1"),
  forestEntry("entry-3", "session-1", 3, "assistant.message", { text: "Delegating implementation and verification." }, "entry-2"),
  forestEntry("entry-4a", "session-1", 4, "user.message", { text: "Try the direct implementation path." }, "entry-3"),
  forestEntry("entry-5a", "session-1", 5, "assistant.message", { text: "This is the inactive branch." }, "entry-4a"),
  forestEntry("entry-4b", "session-1", 6, "user.message", { text: "Use isolated workers instead." }, "entry-3"),
  forestEntry("entry-5b", "session-1", 7, "compaction", { schemaVersion: 1, summary: "Workers own isolated paths", firstRetainedEntryId: "entry-6b", tokensBefore: 9200, filesTouched: ["src-tauri/src/lib.rs"], reason: "phase_boundary", sourceAgent: "session-1" }, "entry-4b"),
  forestEntry("entry-6b", "session-1", 8, "branch.summary", { summary: "Selected isolated-worker branch" }, "entry-5b"),
  forestEntry("entry-7b", "session-1", 9, "worker.result", { status: "completed", summary: "Lifecycle implementation verified", decisions: ["Keep SQLite authoritative"], tests: ["130 Rust tests"] }, "entry-6b"),
  forestEntry("entry-raw", "session-1", 10, "provider.unknown", { method: "provider/debug", raw: { trace: "collapsed" } }, "entry-7b")
];
const mockForests: Record<string, SessionForestSnapshot> = {
  "session-1": {
    sessionId: "session-1", entries: demoEntries, head: { sessionId: "session-1", activeEntryId: "entry-raw", nativeProviderSessionId: "mock-thread-1", restorationMode: "hot", resumeEligibility: "native", latestCheckpointEntryId: "entry-2", updatedAt: now }, leaves: [demoEntries[4], demoEntries[9]],
    workerLeases: [
      { sessionId: "session-1w", workspaceId: "demo-1", role: "implementation", capabilityTier: "strong", taskFamily: "implementation", ownedPaths: ["src/auth/**"], writeMode: "isolated", leaseStatus: "active", expiresAt: null, createdAt: now, updatedAt: now },
      { sessionId: "session-1w2", workspaceId: "demo-1", role: "verification", capabilityTier: "strong", taskFamily: "verification", ownedPaths: ["src/auth/**"], writeMode: "readOnly", leaseStatus: "released", expiresAt: null, createdAt: now, updatedAt: now }
    ],
    workerRuntimes: [
      { sessionId: "session-1w", parentSessionId: "session-1", lifecycleState: "working", taskFamily: "implementation", compatibilityKey: "demo", resultStatus: "pending", retryCount: 0, warmUntil: null, worktreePath: "/tmp/bridge/worker-1w", worktreeBranch: "bridge/worker-1w", lastResult: null, updatedAt: now },
      { sessionId: "session-1w2", parentSessionId: "session-1", lifecycleState: "completed", taskFamily: "verification", compatibilityKey: "demo", resultStatus: "reported", retryCount: 0, warmUntil: null, worktreePath: null, worktreeBranch: null, lastResult: { status: "completed", summary: "All 42 auth tests pass", tests: ["auth suite"] }, updatedAt: now }
    ],
    workerQueue: [{ id: "queue-1", parentSessionId: "session-1", workspaceId: "demo-1", turnId: "mock-turn-1", request: { role: "implementation", objective: "Update the auth serializer", ownedPaths: ["src/auth/**"], writeMode: "isolated", reason: "owned_path_conflict" }, actualModel: "gpt-5.6-terra", queueStatus: "queued", sequence: 1, dispatchedSessionId: null, createdAt: now, updatedAt: now }],
    usage: [
      { id: 1, workspaceId: "demo-1", sessionId: "session-1", turnId: "mock-turn-1", inputTokens: null, outputTokens: null, cacheReadTokens: null, cacheWriteTokens: null, contextPercent: 38, capabilityUnits: 0, runtimeMs: null, source: "provider.codex", createdAt: now },
      { id: 2, workspaceId: "demo-1", sessionId: "session-1w", turnId: "mock-turn-1", inputTokens: null, outputTokens: null, cacheReadTokens: null, cacheWriteTokens: null, contextPercent: null, capabilityUnits: 8, runtimeMs: null, source: "policy.spawn.strong", createdAt: now }
    ],
    reasons: [
      { id: 106, source: "adapter", kind: "session.shutdown", entityId: "session-old", body: "user_stopped", createdAt: now },
      { id: 105, source: "compaction", kind: "checkpoint.turn_started", entityId: "session-1", body: "phase_boundary", createdAt: now },
      { id: 104, source: "worker-pool", kind: "worker.queued", entityId: "queue-1", body: "owned_path_conflict: src/auth/**", createdAt: now },
      { id: 103, source: "policy", kind: "policy.rejected", entityId: "session-1", body: "turn_budget_exhausted", createdAt: now },
      { id: 102, source: "restoration", kind: "session.restored", entityId: "session-1w2", body: "checkpoint_restored", createdAt: now },
      { id: 101, source: "policy", kind: "worker.spawned", entityId: "session-1w", body: "implementation strong isolated", createdAt: now }
    ],
    policyLimits: { maxWorkersPerTurn: 3, maxStrongWorkersPerTurn: 1, maxCapabilityUnitsPerTurn: 24 }
  }
};
function mockForest(sessionId: string): SessionForestSnapshot {
  const existing = mockForests[sessionId];
  if (existing) return structuredClone(existing);
  const session = mockState.sessions.find(item => item.id === sessionId);
  const entry = forestEntry(`${sessionId}-root`, sessionId, 1, "branch.summary", { summary: "Session started" }, null);
  const created: SessionForestSnapshot = { sessionId, entries: [entry], head: { sessionId, activeEntryId: entry.id, nativeProviderSessionId: session?.providerSessionId ?? null, restorationMode: session?.restorationMode ?? "fresh", resumeEligibility: session?.providerSessionId ? "native" : "none", latestCheckpointEntryId: null, updatedAt: now }, leaves: [entry], workerLeases: [], workerRuntimes: [], workerQueue: [], usage: [], reasons: [], policyLimits: { maxWorkersPerTurn: 3, maxStrongWorkersPerTurn: 1, maxCapabilityUnitsPerTurn: 24 } };
  mockForests[sessionId] = created;
  return structuredClone(created);
}
function snapshot() { return structuredClone(mockState); }
function emitState() { stateListeners.forEach(listener => listener()); }
function appendAgent(sessionId: string, kind: string, fields: Partial<AgentEvent> = {}) {
  const event = agentEvent(nextEventId++, sessionId, kind, fields);
  event.sequence = Math.max(0, ...mockState.agentEvents.filter(item => item.sessionId === sessionId).map(item => item.sequence)) + 1;
  mockState.agentEvents.push(event);
}

const mockHealth: Health = {
  ok: true, version: "0.1.0-demo", harnesses: { claude: true, codex: true, shell: true }, database: "demo",
  adapters: [
    { id: "codex", label: "Orchestrator", available: true, version: "mock", capabilities: ["messages", "streaming", "reasoning", "plans", "tools", "commands", "file_changes", "approvals", "usage", "history", "interrupt"], unavailableReason: null, models: [{ id: "gpt-5.6-luna", label: "GPT Luna", tier: "fast", defaultForTier: true }, { id: "gpt-5.6-terra", label: "GPT Terra", tier: "standard", defaultForTier: true }, { id: "gpt-5.6-sol", label: "GPT Sol", tier: "strong", defaultForTier: true }, { id: "gpt-5.3-codex", label: "GPT-5.3 Codex", tier: "standard", defaultForTier: false }], defaultModel: "gpt-5.6-luna" },
    { id: "claude", label: "Claude Code", available: true, version: "mock", capabilities: ["messages", "streaming", "reasoning", "tools", "commands", "approvals", "usage", "interrupt"], unavailableReason: null, models: [{ id: "sonnet", label: "Claude Sonnet", tier: "standard", defaultForTier: true }, { id: "opus", label: "Claude Opus", tier: "strong", defaultForTier: false }, { id: "haiku", label: "Claude Haiku", tier: "fast", defaultForTier: true }, { id: "fable", label: "Claude Fable", tier: "strong", defaultForTier: true }], defaultModel: "sonnet" }
  ]
};

export const bridgeApi = {
  health: (): Promise<Health> => isTauri() ? invoke("health") : Promise.resolve(structuredClone(mockHealth)),
  state: (): Promise<BridgeState> => isTauri() ? invoke("get_state") : Promise.resolve(snapshot()),
  sessionForest: (sessionId: string): Promise<SessionForestSnapshot> => isTauri() ? invoke("get_session_forest", { sessionId }) : Promise.resolve(mockForest(sessionId)),
  activateSessionEntry: async (sessionId: string, entryId: string): Promise<SessionForestSnapshot> => {
    if (isTauri()) return invoke("activate_session_entry", { sessionId, entryId });
    if (!mockForests[sessionId]) mockForest(sessionId);
    const forest = mockForests[sessionId];
    if (!forest.entries.some(entry => entry.id === entryId)) throw new Error("Entry is not in this session");
    if (forest.head) forest.head.activeEntryId = entryId;
    forest.reasons.unshift({ id: nextEventId++, source: "session-forest", kind: "session.head_moved", entityId: sessionId, body: `Conversation head moved to ${entryId}; files were not changed`, createdAt: new Date().toISOString() });
    emitState(); return structuredClone(forest);
  },
  compactSession: async (sessionId: string): Promise<void> => {
    if (isTauri()) return invoke("compact_session", { sessionId });
    if (!mockForests[sessionId]) mockForest(sessionId);
    const forest = mockForests[sessionId];
    const parent = forest.head?.activeEntryId ?? null;
    const sequence = Math.max(0, ...forest.entries.map(entry => entry.sequence));
    const checkpointId = `checkpoint-${nextEventId++}`;
    const compactionId = `compaction-${nextEventId++}`;
    const retainedId = `retained-${nextEventId++}`;
    forest.entries.push(
      forestEntry(checkpointId, sessionId, sequence + 1, "checkpoint", { schemaVersion: 1, summary: "Manual checkpoint", sourceAgent: sessionId }, parent),
      forestEntry(compactionId, sessionId, sequence + 2, "compaction", { schemaVersion: 1, summary: "Manual compaction", reason: "manual", firstRetainedEntryId: retainedId, sourceAgent: sessionId }, checkpointId),
      forestEntry(retainedId, sessionId, sequence + 3, "branch.summary", { summary: "Manual compaction boundary" }, compactionId)
    );
    if (forest.head) { forest.head.activeEntryId = retainedId; forest.head.latestCheckpointEntryId = checkpointId; }
    forest.leaves = [...forest.leaves.filter(entry => entry.id !== parent), forest.entries.at(-1)!];
    forest.reasons.unshift({ id: nextEventId++, source: "compaction", kind: "compaction.completed", entityId: sessionId, body: "manual", createdAt: new Date().toISOString() });
    emitState();
  },
  addProject: async (path: string): Promise<BridgeState> => {
    if (isTauri()) return invoke("add_project", { path });
    const name = path.split("/").filter(Boolean).at(-1) || "Repository";
    mockState.projects.push({ id: crypto.randomUUID(), name, path, createdAt: new Date().toISOString() }); emitState(); return snapshot();
  },
  createWorkspace: async (projectId: string, title: string, harness: Harness): Promise<BridgeState> => {
    if (isTauri()) return invoke("create_workspace", { projectId, title, harness });
    const cities = ["Oslo", "Seoul", "Tallinn", "Nairobi"]; const city = cities[mockState.workspaces.length % cities.length]; const id = crypto.randomUUID();
    mockState.workspaces.push({ id, projectId, city, title, branch: `bridge/${safeSlug(title)}`, path: `/tmp/bridge/${city}`, status: "idle", dirtyFiles: 0, additions: 0, deletions: 0, createdAt: new Date().toISOString() });
    mockState.sessions.push({ id: crypto.randomUUID(), workspaceId: id, harness: "codex", label: "Orchestrator", status: "idle", startedAt: null, endedAt: null, contextPercent: null, usagePercent: null, metricSource: "estimated", providerSessionId: null, activeTurnId: null, model: "gpt-5.6-luna", requestedTier: "fast", restorationMode: "fresh" }); emitState(); return snapshot();
  },
  startSession: async (workspaceId: string, harness?: Harness | null, model?: string | null): Promise<BridgeState> => {
    if (isTauri()) return invoke("start_session", { workspaceId, harness: harness ?? null, model: model ?? null });
    const resolvedHarness = harness ?? "codex";
    let session = mockState.sessions.find(item => item.workspaceId === workspaceId && item.harness === resolvedHarness);
    if (!session) { session = { id: crypto.randomUUID(), workspaceId, harness: resolvedHarness, label: "Orchestrator", status: "idle", startedAt: null, endedAt: null, contextPercent: null, usagePercent: null, metricSource: "estimated", model: model ?? "gpt-5.6-luna", requestedTier: "fast", restorationMode: "fresh" }; mockState.sessions.push(session); }
    session.restorationMode = session.providerSessionId ? "native" : "fresh"; session.status = "working"; session.startedAt = new Date().toISOString(); session.endedAt = null; session.providerSessionId = session.providerSessionId ?? `mock-${crypto.randomUUID()}`; session.model = model ?? session.model ?? "gpt-5.6-luna"; session.label = "Orchestrator";
    const workspace = mockState.workspaces.find(item => item.id === workspaceId); if (workspace) workspace.status = "working";
    appendAgent(session.id, "session.started", { status: "working" }); emitState(); return snapshot();
  },
  stopSession: async (sessionId: string): Promise<BridgeState> => {
    if (isTauri()) return invoke("stop_session", { sessionId });
    const session = mockState.sessions.find(item => item.id === sessionId); if (session) { session.status = "stopped"; session.endedAt = new Date().toISOString(); session.activeTurnId = null; }
    emitState(); return snapshot();
  },
  sendTurn: async (sessionId: string, text: string): Promise<void> => {
    if (isTauri()) return invoke("send_turn", { sessionId, text });
    const session = mockState.sessions.find(item => item.id === sessionId); if (!session) throw new Error("Structured adapter session is not running");
    session.status = "working"; session.activeTurnId = `mock-turn-${nextEventId}`;
    appendAgent(sessionId, "message.completed", { itemId: `user-${nextEventId}`, role: "user", status: "completed", text });
    const assistantItemId = `assistant-${nextEventId}`;
    appendAgent(sessionId, "message.delta", { itemId: assistantItemId, role: "assistant", status: "streaming", text: "I’ll handle that through the normalized adapter layer. " });
    appendAgent(sessionId, "message.completed", { itemId: assistantItemId, role: "assistant", status: "completed", text: "I’ll handle that through the normalized adapter layer. The GUI remains provider-neutral, and no agent TUI is rendered." });
    session.status = "ready"; session.activeTurnId = null; emitState();
  },
  interruptTurn: (sessionId: string): Promise<void> => isTauri() ? invoke("interrupt_turn", { sessionId }) : Promise.resolve(),
  resolveApproval: async (sessionId: string, eventId: number, decision: string): Promise<void> => {
    if (isTauri()) return invoke("resolve_approval", { sessionId, eventId, decision });
    const request = mockState.agentEvents.find(item => item.id === eventId); if (request) appendAgent(request.sessionId, "approval.resolved", { status: decision, data: { requestEventId: eventId, decision } }); emitState();
  },
  openTerminal: (workspaceId: string): Promise<void> => isTauri() ? invoke("open_terminal", { workspaceId }) : Promise.resolve(),
  writeTerminal: (workspaceId: string, data: string): Promise<void> => isTauri() ? invoke("write_terminal", { workspaceId, data }) : Promise.resolve(),
  resizeTerminal: (workspaceId: string, rows: number, cols: number): Promise<void> => isTauri() ? invoke("resize_terminal", { workspaceId, rows, cols }) : Promise.resolve(),
  refreshWorkspace: (workspaceId: string): Promise<BridgeState> => isTauri() ? invoke("refresh_workspace", { workspaceId }) : Promise.resolve(snapshot()),
  archiveWorkspace: async (workspaceId: string): Promise<BridgeState> => {
    if (isTauri()) return invoke("archive_workspace", { workspaceId });
    mockState.sessions = mockState.sessions.filter(session => session.workspaceId !== workspaceId); mockState.workspaces = mockState.workspaces.filter(workspace => workspace.id !== workspaceId); emitState(); return snapshot();
  },
  onTerminal: async (handler: (chunk: TerminalChunk) => void): Promise<UnlistenFn> => isTauri() ? listen<TerminalChunk>("session-output", event => handler(event.payload)) : () => undefined,
  onAgentEvent: async (handler: (event: AgentEvent) => void): Promise<UnlistenFn> => {
    if (isTauri()) return listen<AgentEvent>("agent-event", event => handler(event.payload)); 
    return () => undefined;
  },
  onStateChanged: async (handler: () => void): Promise<UnlistenFn> => {
    if (isTauri()) return listen("state-changed", handler); stateListeners.add(handler); return () => stateListeners.delete(handler);
  }
};
