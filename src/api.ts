import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { AgentEvent, BridgeState, Harness, Health, TerminalChunk } from "./types";
import { safeSlug } from "./utils";

const isTauri = () => "__TAURI_INTERNALS__" in window;
const now = new Date().toISOString();
const stateListeners = new Set<() => void>();
const agentListeners = new Set<(event: AgentEvent) => void>();
let nextEventId = 20;

let mockState: BridgeState = {
  projects: [{ id: "demo-project", name: "Bridge", path: "/Users/you/Developer/bridge", createdAt: now }],
  workspaces: [
    { id: "demo-1", projectId: "demo-project", city: "Kyoto", title: "Build session supervisor", branch: "bridge/session-supervisor", path: "/Users/you/bridge/Kyoto", status: "working", dirtyFiles: 4, additions: 284, deletions: 31, createdAt: now },
    { id: "demo-2", projectId: "demo-project", city: "Lisbon", title: "Polish the Deck shell", branch: "bridge/deck-shell", path: "/Users/you/bridge/Lisbon", status: "ready", dirtyFiles: 7, additions: 612, deletions: 88, createdAt: now },
    { id: "demo-3", projectId: "demo-project", city: "Reykjavik", title: "Add event ledger", branch: "bridge/event-ledger", path: "/Users/you/bridge/Reykjavik", status: "ready", dirtyFiles: 0, additions: 148, deletions: 12, createdAt: now }
  ],
  sessions: [
    { id: "session-1", workspaceId: "demo-1", harness: "codex", label: "Orchestrator", status: "working", startedAt: now, endedAt: null, contextPercent: 38, usagePercent: 24, metricSource: "reported", providerSessionId: "mock-thread-1", activeTurnId: "mock-turn-1", model: "gpt-5.6-luna", effort: null, parentSessionId: null, depth: 0 },
    { id: "session-1w", workspaceId: "demo-1", harness: "claude", label: "Claude · Fable", status: "working", startedAt: now, endedAt: null, contextPercent: 21, usagePercent: 14, metricSource: "reported", providerSessionId: "mock-claude-1", activeTurnId: "mock-turn-1w", model: "fable", effort: "high", parentSessionId: "session-1", depth: 1 },
    { id: "session-1w2", workspaceId: "demo-1", harness: "codex", label: "Codex · GPT Sol", status: "ready", startedAt: now, endedAt: null, contextPercent: 9, usagePercent: 6, metricSource: "reported", providerSessionId: "mock-codex-2", activeTurnId: null, model: "gpt-5.6-sol", effort: "xhigh", parentSessionId: "session-1w", depth: 2 },
    { id: "session-2", workspaceId: "demo-2", harness: "codex", label: "Orchestrator", status: "ready", startedAt: now, endedAt: null, contextPercent: 12, usagePercent: 8, metricSource: "reported", providerSessionId: "mock-thread-2", activeTurnId: null, model: "gpt-5.6-luna", effort: null, parentSessionId: null, depth: 0 }
  ],
  events: [
    { id: 2, source: "git", kind: "workspace.changed", entityId: "demo-1", body: "4 files changed · +284 −31", createdAt: now },
    { id: 1, source: "gate", kind: "workspace.ready", entityId: "demo-3", body: "Tests and typecheck passed.", createdAt: now }
  ],
  agentEvents: [
    agentEvent(1, "session-1", "message.completed", { itemId: "user-1", role: "user", status: "completed", text: "Build the structured session supervisor." }),
    agentEvent(2, "session-1", "plan.updated", { title: "Implementation plan", status: "inProgress", data: { plan: [{ step: "Define normalized harness primitives", status: "completed" }, { step: "Build the native conversation GUI", status: "inProgress" }, { step: "Verify the real Codex adapter", status: "pending" }] } }),
    agentEvent(3, "session-1", "tool.started", { itemId: "tool-1", title: "Inspect workspace", status: "completed", data: { type: "commandExecution", cwd: "/Users/you/bridge/Kyoto", aggregatedOutput: "src/api.ts\nsrc/App.tsx\nsrc-tauri/src/lib.rs" } }),
    agentEvent(4, "session-1", "message.completed", { itemId: "assistant-1", role: "assistant", status: "completed", text: "The adapter boundary is in place. This is heavy refactor work, so I’m delegating it to Claude Fable at high effort." }),
    agentEvent(5, "session-1", "delegation.spawned", { itemId: "spawn-1w", role: "system", status: "working", title: "Delegated to Claude · Fable", text: "Refactor the auth module to use the new token store, then verify.", data: { childSessionId: "session-1w", harness: "claude", model: "fable", modelLabel: "Fable", effort: "high", depth: 1 } }),
    agentEvent(6, "session-1w", "message.completed", { itemId: "user-1w", role: "user", status: "completed", text: "Refactor the auth module to use the new token store, then verify." }),
    agentEvent(7, "session-1w", "message.completed", { itemId: "assistant-1w", role: "assistant", status: "completed", text: "Refactor done. Delegating a focused verification pass to Codex GPT Sol at xhigh effort." }),
    agentEvent(8, "session-1w", "delegation.spawned", { itemId: "spawn-1w2", role: "system", status: "working", title: "Delegated to Codex · GPT Sol", text: "Run the auth test suite and confirm the token store migration is correct.", data: { childSessionId: "session-1w2", harness: "codex", model: "gpt-5.6-sol", modelLabel: "GPT Sol", effort: "xhigh", depth: 2 } }),
    agentEvent(9, "session-1w2", "message.completed", { itemId: "assistant-1w2", role: "assistant", status: "completed", text: "All 42 auth tests pass. Token store migration verified." }),
    agentEvent(10, "session-1w", "delegation.result", { itemId: "result-1w2", role: "system", status: "completed", title: "Worker result", text: "[worker result] Codex · GPT Sol (codex/GPT Sol, effort xhigh) finished:\n\nAll 42 auth tests pass. Token store migration verified.", data: { childSessionId: "session-1w2", delivered: true } }),
    agentEvent(11, "session-1", "delegation.result", { itemId: "result-1w", role: "system", status: "completed", title: "Worker result", text: "[worker result] Claude · Fable (claude/Fable, effort high) finished:\n\nAuth module refactored to the new token store and verified by a Codex sub-worker (42/42 tests pass).", data: { childSessionId: "session-1w", delivered: true } })
  ]
};

function agentEvent(id: number, sessionId: string, kind: string, fields: Partial<AgentEvent> = {}): AgentEvent {
  return { id, sessionId, sequence: id, protocolVersion: 1, kind, itemId: null, role: null, status: null, title: null, text: null, data: {}, providerMeta: { adapter: "fake" }, createdAt: new Date().toISOString(), ...fields };
}
function snapshot() { return structuredClone(mockState); }
function emitState() { stateListeners.forEach(listener => listener()); }
function appendAgent(sessionId: string, kind: string, fields: Partial<AgentEvent> = {}) {
  const event = agentEvent(nextEventId++, sessionId, kind, fields);
  event.sequence = Math.max(0, ...mockState.agentEvents.filter(item => item.sessionId === sessionId).map(item => item.sequence)) + 1;
  mockState.agentEvents.push(event); agentListeners.forEach(listener => listener(structuredClone(event)));
}

const mockHealth: Health = {
  ok: true, version: "0.1.0-demo", harnesses: { claude: true, codex: true, shell: true }, database: "demo",
  adapters: [
    { id: "codex", label: "Orchestrator", available: true, version: "mock", capabilities: ["messages", "streaming", "reasoning", "plans", "tools", "commands", "file_changes", "approvals", "usage", "history", "interrupt"], unavailableReason: null, models: [{ id: "gpt-5.6-luna", label: "GPT Luna" }, { id: "gpt-5.6-sol", label: "GPT Sol" }], defaultModel: "gpt-5.6-luna" },
    { id: "claude", label: "Claude Code", available: true, version: "mock", capabilities: ["messages", "streaming", "reasoning", "tools", "commands", "approvals", "usage", "interrupt"], unavailableReason: null, models: [{ id: "sonnet", label: "Claude Sonnet" }, { id: "opus", label: "Claude Opus" }, { id: "haiku", label: "Claude Haiku" }, { id: "fable", label: "Claude Fable" }], defaultModel: "sonnet" }
  ]
};

export const bridgeApi = {
  health: (): Promise<Health> => isTauri() ? invoke("health") : Promise.resolve(structuredClone(mockHealth)),
  state: (): Promise<BridgeState> => isTauri() ? invoke("get_state") : Promise.resolve(snapshot()),
  addProject: async (path: string): Promise<BridgeState> => {
    if (isTauri()) return invoke("add_project", { path });
    const name = path.split("/").filter(Boolean).at(-1) || "Repository";
    mockState.projects.push({ id: crypto.randomUUID(), name, path, createdAt: new Date().toISOString() }); emitState(); return snapshot();
  },
  createWorkspace: async (projectId: string, title: string, harness: Harness): Promise<BridgeState> => {
    if (isTauri()) return invoke("create_workspace", { projectId, title, harness });
    const cities = ["Oslo", "Seoul", "Tallinn", "Nairobi"]; const city = cities[mockState.workspaces.length % cities.length]; const id = crypto.randomUUID();
    mockState.workspaces.push({ id, projectId, city, title, branch: `bridge/${safeSlug(title)}`, path: `/tmp/bridge/${city}`, status: "idle", dirtyFiles: 0, additions: 0, deletions: 0, createdAt: new Date().toISOString() });
    mockState.sessions.push({ id: crypto.randomUUID(), workspaceId: id, harness: "codex", label: "Orchestrator", status: "idle", startedAt: null, endedAt: null, contextPercent: null, usagePercent: null, metricSource: "estimated", providerSessionId: null, activeTurnId: null, model: "gpt-5.6-luna" }); emitState(); return snapshot();
  },
  startSession: async (workspaceId: string, harness?: Harness | null, model?: string | null): Promise<BridgeState> => {
    if (isTauri()) return invoke("start_session", { workspaceId, harness: harness ?? null, model: model ?? null });
    const resolvedHarness = harness ?? "codex";
    let session = mockState.sessions.find(item => item.workspaceId === workspaceId && item.harness === resolvedHarness);
    if (!session) { session = { id: crypto.randomUUID(), workspaceId, harness: resolvedHarness, label: "Orchestrator", status: "idle", startedAt: null, endedAt: null, contextPercent: null, usagePercent: null, metricSource: "estimated", model: model ?? "gpt-5.6-luna" }; mockState.sessions.push(session); }
    session.status = "working"; session.startedAt = new Date().toISOString(); session.endedAt = null; session.providerSessionId = `mock-${crypto.randomUUID()}`; session.model = model ?? session.model ?? "gpt-5.6-luna"; session.label = "Orchestrator";
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
  resolveApproval: async (eventId: number, decision: string): Promise<void> => {
    if (isTauri()) return invoke("resolve_approval", { eventId, decision });
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
    if (isTauri()) return listen<AgentEvent>("agent-event", event => handler(event.payload)); agentListeners.add(handler); return () => agentListeners.delete(handler);
  },
  onStateChanged: async (handler: () => void): Promise<UnlistenFn> => {
    if (isTauri()) return listen("state-changed", handler); stateListeners.add(handler); return () => stateListeners.delete(handler);
  }
};
