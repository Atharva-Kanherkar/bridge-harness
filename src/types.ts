export type Harness = "claude" | "codex" | "shell";
export type SessionStatus = "idle" | "starting" | "working" | "waiting" | "warm" | "checkpointing" | "ready" | "stopped" | "resuming" | "restored" | "failed" | "completed" | "cancelled";
export type CapabilityTier = "fast" | "standard" | "strong";
export type RestorationMode = "hot" | "native" | "checkpoint_restored" | "fresh";
export type ContinuationFidelity = "native" | "projected_at_boundary" | "projected_mid_turn";
export type ResumeEligibility = "none" | "native" | "checkpoint_restored";
export type WorkerQueueStatus = "queued" | "blocked_on_human" | "dispatching" | "dispatched" | "expired" | "cancelled" | "rejected" | "dead_letter";

export interface Project { id: string; name: string; path: string; createdAt: string }
export interface Workspace {
  id: string; projectId: string | null; city: string | null; title: string; branch: string | null; path: string | null;
  status: SessionStatus; dirtyFiles: number; additions: number; deletions: number; createdAt: string;
}
export interface Session {
  id: string; workspaceId: string | null; harness: Harness; label: string; status: SessionStatus;
  startedAt: string | null; endedAt: string | null; contextPercent: number | null;
  usagePercent: number | null; metricSource: "reported" | "measured" | "estimated";
  providerSessionId?: string | null; activeTurnId?: string | null; model?: string | null;
  requestedTier?: CapabilityTier | null;
  effort?: string | null; parentSessionId?: string | null; depth?: number | null;
  restorationMode: RestorationMode;
  continuationFidelity?: ContinuationFidelity;
  title?: string | null; kind?: string; cwd?: string | null;
}
export interface BridgeEvent { id: number; source: string; kind: string; entityId: string; body: string; createdAt: string }
export interface AgentEvent {
  id: number; sessionId: string; sequence: number; protocolVersion: number; kind: string;
  itemId: string | null; role: string | null; status: string | null; title: string | null;
  text: string | null; data: Record<string, unknown>; providerMeta: Record<string, unknown>; createdAt: string;
}
export interface SessionEntry {
  id: string; sessionId: string; parentEntryId: string | null; sequence: number; semanticSchemaVersion: number; kind: string;
  payload: Record<string, unknown>; providerEventId: string | null; contextVisibility: string;
  tokenEstimate: number | null; createdAt: string;
}
export interface SessionHead {
  sessionId: string; activeEntryId: string | null; nativeProviderSessionId: string | null;
  restorationMode: RestorationMode; resumeEligibility: ResumeEligibility;
  latestCheckpointEntryId: string | null; updatedAt: string;
}
export interface WorkerLease {
  sessionId: string; workspaceId: string; role: string; capabilityTier: string; taskFamily: string;
  ownedPaths: string[]; writeMode: string; leaseStatus: string; expiresAt: string | null;
  createdAt: string; updatedAt: string;
}
export interface WorkerRuntimeRecord {
  sessionId: string; parentSessionId: string; lifecycleState: string; taskFamily: string;
  compatibilityKey: string; resultStatus: string; retryCount: number; warmUntil: string | null;
  worktreePath: string | null; worktreeBranch: string | null; lastResult: Record<string, unknown> | null;
  updatedAt: string;
}
export interface QueuedWorkerRequest {
  id: string; parentSessionId: string; workspaceId: string; turnId: string;
  request: Record<string, unknown>; actualModel: string; queueStatus: WorkerQueueStatus; sequence: number;
  dispatchedSessionId: string | null; expiresAt?: string; blockedAt?: string | null;
  claimedAt?: string | null; lastError?: string | null; createdAt: string; updatedAt: string;
}
export interface UsageLedgerRow {
  id: number; workspaceId: string; sessionId: string | null; turnId: string | null;
  inputTokens: number | null; outputTokens: number | null; cacheReadTokens: number | null;
  cacheWriteTokens: number | null; contextPercent: number | null; capabilityUnits: number;
  runtimeMs: number | null; source: string; createdAt: string;
}
export interface PolicyLimits {
  maxWorkersPerTurn: number; maxStrongWorkersPerTurn: number; maxCapabilityUnitsPerTurn: number;
}
export interface SessionForestSnapshot {
  sessionId: string; entries: SessionEntry[]; head: SessionHead | null; leaves: SessionEntry[];
  workerLeases: WorkerLease[]; workerRuntimes: WorkerRuntimeRecord[];
  workerQueue: QueuedWorkerRequest[]; usage: UsageLedgerRow[]; reasons: BridgeEvent[];
  policyLimits: PolicyLimits;
  repositoryDivergence: { status: "aligned" | "diverged" | "unknown"; selectedState: Record<string, unknown> | null; currentState: Record<string, unknown> };
}
export interface ModelOption { id: string; label: string; tier: CapabilityTier; defaultForTier: boolean }
export interface AdapterDescriptor {
  id: string; label: string; available: boolean; version: string | null; capabilities: string[];
  unavailableReason: string | null; models: ModelOption[]; defaultModel: string | null;
}
export interface BridgeState { projects: Project[]; workspaces: Workspace[]; sessions: Session[]; events: BridgeEvent[] }
export interface Health { ok: boolean; version: string; harnesses: Record<Harness, boolean>; database: string; adapters: AdapterDescriptor[] }
export interface TerminalChunk { sessionId: string; data: string }
export interface SlashCommand { name: string; description: string; harness: Harness; kind: "command" | "skill" | "prompt" | "builtin" }
export type MarketplaceProvider = "codex" | "claude";
export type MarketplaceAction = "install" | "enable" | "disable" | "update" | "uninstall" | "authenticate";
export interface MarketplaceVariant {
  provider: MarketplaceProvider; pluginId: string; name: string; description: string | null;
  marketplace: string | null; version: string | null; source: string | null; repository: string | null;
  publisher: string | null; capabilities: string[]; mcpEndpoint: string | null; connectorType: string | null;
  installed: boolean; enabled: boolean; authenticationState: string; sharedAuthMechanism: string | null;
  portableMcp: boolean; compatibilityNotes: string[]; supportedActions: MarketplaceAction[]; providerMetadata: Record<string, unknown>;
}
export interface MarketplaceProviderCatalog {
  provider: MarketplaceProvider; available: boolean; variants: MarketplaceVariant[]; error: string | null;
}
export interface MarketplaceCatalog { providers: MarketplaceProviderCatalog[] }
export interface MarketplaceActionResult {
  provider: MarketplaceProvider; pluginId: string; action: MarketplaceAction;
  success: boolean; message: string; error: string | null;
}
