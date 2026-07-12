import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { BridgeState, Harness, Health, TerminalChunk } from "./types";
import { safeSlug } from "./utils";

const isTauri = () => "__TAURI_INTERNALS__" in window;
const now = new Date().toISOString();
let mockState: BridgeState = {
  projects: [{ id: "demo-project", name: "Bridge", path: "/Users/you/Developer/bridge", createdAt: now }],
  workspaces: [
    { id: "demo-1", projectId: "demo-project", city: "Kyoto", title: "Build session supervisor", branch: "bridge/session-supervisor", path: "/Users/you/bridge/Kyoto", status: "working", dirtyFiles: 4, additions: 284, deletions: 31, createdAt: now },
    { id: "demo-2", projectId: "demo-project", city: "Lisbon", title: "Polish the Deck shell", branch: "bridge/deck-shell", path: "/Users/you/bridge/Lisbon", status: "waiting", dirtyFiles: 7, additions: 612, deletions: 88, createdAt: now },
    { id: "demo-3", projectId: "demo-project", city: "Reykjavik", title: "Add event ledger", branch: "bridge/event-ledger", path: "/Users/you/bridge/Reykjavik", status: "ready", dirtyFiles: 0, additions: 148, deletions: 12, createdAt: now }
  ],
  sessions: [
    { id: "session-1", workspaceId: "demo-1", harness: "codex", label: "Codex", status: "working", startedAt: now, endedAt: null, contextPercent: 38, usagePercent: 24, metricSource: "estimated" },
    { id: "session-2", workspaceId: "demo-2", harness: "claude", label: "Claude", status: "waiting", startedAt: now, endedAt: null, contextPercent: 68, usagePercent: 42, metricSource: "estimated" },
    { id: "session-3", workspaceId: "demo-3", harness: "shell", label: "Shell", status: "ready", startedAt: now, endedAt: now, contextPercent: null, usagePercent: null, metricSource: "measured" }
  ],
  events: [
    { id: 3, source: "supervisor", kind: "session.waiting", entityId: "session-2", body: "Claude needs your decision on the navigation density.", createdAt: now },
    { id: 2, source: "git", kind: "workspace.changed", entityId: "demo-1", body: "4 files changed · +284 −31", createdAt: now },
    { id: 1, source: "gate", kind: "workspace.ready", entityId: "demo-3", body: "Tests and typecheck passed.", createdAt: now }
  ]
};

export const bridgeApi = {
  health: (): Promise<Health> => isTauri() ? invoke("health") : Promise.resolve({ ok: true, version: "0.1.0-demo", harnesses: { claude: true, codex: true, shell: true }, database: "demo" }),
  state: (): Promise<BridgeState> => isTauri() ? invoke("get_state") : Promise.resolve(structuredClone(mockState)),
  addProject: async (path: string): Promise<BridgeState> => {
    if (isTauri()) return invoke("add_project", { path });
    const name = path.split("/").filter(Boolean).at(-1) || "Repository";
    mockState.projects.push({ id: crypto.randomUUID(), name, path, createdAt: new Date().toISOString() });
    return structuredClone(mockState);
  },
  createWorkspace: async (projectId: string, title: string, harness: Harness): Promise<BridgeState> => {
    if (isTauri()) return invoke("create_workspace", { projectId, title, harness });
    const cities = ["Oslo", "Seoul", "Tallinn", "Nairobi"];
    const city = cities[mockState.workspaces.length % cities.length];
    const id = crypto.randomUUID();
    mockState.workspaces.push({ id, projectId, city, title, branch: `bridge/${safeSlug(title)}`, path: `/tmp/bridge/${city}`, status: "idle", dirtyFiles: 0, additions: 0, deletions: 0, createdAt: new Date().toISOString() });
    mockState.sessions.push({ id: crypto.randomUUID(), workspaceId: id, harness, label: harness === "shell" ? "Shell" : harness[0].toUpperCase() + harness.slice(1), status: "idle", startedAt: null, endedAt: null, contextPercent: null, usagePercent: null, metricSource: "estimated" });
    return structuredClone(mockState);
  },
  startSession: (workspaceId: string, harness: Harness): Promise<BridgeState> => isTauri() ? invoke("start_session", { workspaceId, harness }) : Promise.resolve(structuredClone(mockState)),
  stopSession: (sessionId: string): Promise<BridgeState> => isTauri() ? invoke("stop_session", { sessionId }) : Promise.resolve(structuredClone(mockState)),
  writeSession: (sessionId: string, data: string): Promise<void> => isTauri() ? invoke("write_session", { sessionId, data }) : Promise.resolve(),
  resizeSession: (sessionId: string, rows: number, cols: number): Promise<void> => isTauri() ? invoke("resize_session", { sessionId, rows, cols }) : Promise.resolve(),
  refreshWorkspace: (workspaceId: string): Promise<BridgeState> => isTauri() ? invoke("refresh_workspace", { workspaceId }) : Promise.resolve(structuredClone(mockState)),
  archiveWorkspace: async (workspaceId: string): Promise<BridgeState> => {
    if (isTauri()) return invoke("archive_workspace", { workspaceId });
    mockState.sessions = mockState.sessions.filter(session => session.workspaceId !== workspaceId);
    mockState.workspaces = mockState.workspaces.filter(workspace => workspace.id !== workspaceId);
    return structuredClone(mockState);
  },
  onTerminal: async (handler: (chunk: TerminalChunk) => void): Promise<UnlistenFn> => isTauri() ? listen<TerminalChunk>("session-output", e => handler(e.payload)) : () => undefined,
  onStateChanged: async (handler: () => void): Promise<UnlistenFn> => isTauri() ? listen("state-changed", handler) : () => undefined
};
