export type Harness = "claude" | "codex" | "shell";
export type SessionStatus = "idle" | "working" | "waiting" | "ready" | "stopped" | "failed";
export type CapabilityTier = "fast" | "standard" | "strong";

export interface Project { id: string; name: string; path: string; createdAt: string }
export interface Workspace {
  id: string; projectId: string; city: string; title: string; branch: string; path: string;
  status: SessionStatus; dirtyFiles: number; additions: number; deletions: number; createdAt: string;
}
export interface Session {
  id: string; workspaceId: string; harness: Harness; label: string; status: SessionStatus;
  startedAt: string | null; endedAt: string | null; contextPercent: number | null;
  usagePercent: number | null; metricSource: "reported" | "measured" | "estimated";
  providerSessionId?: string | null; activeTurnId?: string | null; model?: string | null;
  requestedTier?: CapabilityTier | null;
  effort?: string | null; parentSessionId?: string | null; depth?: number | null;
}
export interface BridgeEvent { id: number; source: string; kind: string; entityId: string; body: string; createdAt: string }
export interface AgentEvent {
  id: number; sessionId: string; sequence: number; protocolVersion: number; kind: string;
  itemId: string | null; role: string | null; status: string | null; title: string | null;
  text: string | null; data: Record<string, unknown>; providerMeta: Record<string, unknown>; createdAt: string;
}
export interface ModelOption { id: string; label: string; tier: CapabilityTier; defaultForTier: boolean }
export interface AdapterDescriptor {
  id: string; label: string; available: boolean; version: string | null; capabilities: string[];
  unavailableReason: string | null; models: ModelOption[]; defaultModel: string | null;
}
export interface BridgeState { projects: Project[]; workspaces: Workspace[]; sessions: Session[]; events: BridgeEvent[]; agentEvents: AgentEvent[] }
export interface Health { ok: boolean; version: string; harnesses: Record<Harness, boolean>; database: string; adapters: AdapterDescriptor[] }
export interface TerminalChunk { sessionId: string; data: string }
