export type Harness = "claude" | "codex" | "shell";
export type SessionStatus = "idle" | "working" | "waiting" | "ready" | "stopped" | "failed";

export interface Project { id: string; name: string; path: string; createdAt: string }
export interface Workspace {
  id: string; projectId: string; city: string; title: string; branch: string; path: string;
  status: SessionStatus; dirtyFiles: number; additions: number; deletions: number; createdAt: string;
}
export interface Session {
  id: string; workspaceId: string; harness: Harness; label: string; status: SessionStatus;
  startedAt: string | null; endedAt: string | null; contextPercent: number | null;
  usagePercent: number | null; metricSource: "reported" | "measured" | "estimated";
}
export interface BridgeEvent { id: number; source: string; kind: string; entityId: string; body: string; createdAt: string }
export interface BridgeState { projects: Project[]; workspaces: Workspace[]; sessions: Session[]; events: BridgeEvent[] }
export interface Health { ok: boolean; version: string; harnesses: Record<Harness, boolean>; database: string }
export interface TerminalChunk { sessionId: string; data: string }
