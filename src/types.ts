// The frontend's type vocabulary. Protocol shapes come from the GENERATED
// contract (`src/protocol/generated/protocol.ts`) — never restate one here;
// regenerate with:
//   cargo run --manifest-path src-tauri/Cargo.toml -p bridge-protocol --bin generate-protocol-artifacts
//
// What legitimately lives in this file:
// 1. Renames of generated types to the names the app grew up with.
// 2. Documented *refinements* derived from generated types (never restated).
// 3. Result shapes the contract still defers (`resultDeferred` in
//    methods.json) and notification payloads without a schema yet. These are
//    the only hand-written shapes left; each will move to the generated file
//    when its slice of the contract lands.

import type {
  QueuedWorkerRequest as WireQueuedWorkerRequest,
  SessionEntry as WireSessionEntry,
  SessionForestSnapshot as WireSessionForestSnapshot,
  WorkerLease as WireWorkerLease,
  WorkerRuntimeRecord as WireWorkerRuntimeRecord,
  ExternalLearningTriggerKind,
  LearningSchedule,
  LocalLearningTriggerKind,
  MarketplaceAction,
  MarketplaceProvider,
  ModelProfileDraft,
  RemoteBrowserConfig,
  ReplaySessionEvent,
  SkillAction,
  SkillProvider,
  AutomationAction,
  AutomationProvider,
  SaveAutomationParams,
} from "./protocol/generated/protocol";

// ---------------------------------------------------------------------------
// Contracted shapes, re-exported under their app names.
// ---------------------------------------------------------------------------

export type {
  AdapterDescriptor,
  AgentDefinition,
  ApprovalDecision,
  AuthState,
  AutomationAction,
  AutomationProvider,
  SaveAutomationParams,
  BaseBranchDivergence,
  BridgeEvent,
  BridgeState,
  BrowserActionRequest,
  BrowserRouteDecision,
  BrowserRouteRequest,
  BrowserSkill,
  CapabilityTier,
  CheckStatus,
   CompletionSummary,
   CompletionVerdict,
   CompiledPromptPreviewResult,
   ConfigState,
   PermissionPolicy,
   ContinuationFidelity,
   EvalKind,
   ExternalLearningTriggerKind,
   HarnessConfig,
   HealthWarning,
   LearningSchedule,
   LocalLearningTriggerKind,
   ModelOption,
   ModelProfileDraft,
   PolicyLimits,
   ProfilePurpose,
   Project,
   PromptLintWarningView,
   PromptProviderLayerStatus,
   PromptRevisionView,
   PromptSectionMutationResult,
   PromptSectionStatePayload,
   PromptSectionView,
   PromptStackView,
   PromptTargetChoice,
   RemoteBrowserConfig,
  RepositoryDivergence,
  RestorationMode,
  ResumeEligibility,
  RouterMode,
  RouterPreferences,
  SanitizedTurn,
  DispatchAgentShortcutResult,
  SecretInterception,
  Session,
  SessionHead,
  SessionStatus,
  SkillAction,
  SkillProvider,
  SlashCommand,
  SlashCommandResolve,
  SearchSessionEntriesResult,
  SessionRecallHit,
  MemoryRecord,
  ListMemoryRecordsResult,
  MemoryCapabilities,
  ProviderMemoryCommand,
  MemoryExtractionSettings,
  MemoryExtractionRun,
  MemoryInjectionSettings,
  MemoryPacketAudit,
  MemoryPacketItem,
  UsageLedgerRow,
  VerifierCandidate,
  VerifierManifest,
  WorkerRepositoryBinding,
  Workspace,
  WorkspaceChangesResult,
  WorkspaceFileChange,
  RiskTier,
  MarketplaceAction,
  MarketplaceProvider,
} from "./protocol/generated/protocol";

export type {
  HarnessId as Harness,
  Effort as ReasoningEffort,
  CheckRun as CompletionCheckRun,
  HealthResult as Health,
} from "./protocol/generated/protocol";

// ---------------------------------------------------------------------------
// Refinements derived from contracted shapes.
// ---------------------------------------------------------------------------

/** A durable/live agent event. The contract types `data`/`providerMeta` as
 * arbitrary structured JSON; every producer emits objects, and the
 * conversation UI reads them as objects, so the frontend narrows here. */
export type AgentEvent = Omit<ReplaySessionEvent, "data" | "providerMeta"> & {
  data: Record<string, unknown>;
  providerMeta: Record<string, unknown>;
};

/** Structured-JSON fields the contract leaves open (`unknown`) but every
 * core serializer emits in one shape; the frontend narrows to that shape. */
export type SessionEntry = Omit<WireSessionEntry, "payload"> & { payload: Record<string, unknown> };
export type QueuedWorkerRequest = Omit<WireQueuedWorkerRequest, "request"> & { request: Record<string, unknown> };
export type WorkerLease = Omit<WireWorkerLease, "ownedPaths"> & { ownedPaths: string[] };
export type WorkerRuntimeRecord = Omit<WireWorkerRuntimeRecord, "lastResult"> & { lastResult: Record<string, unknown> | null };
export type SessionForestSnapshot = Omit<WireSessionForestSnapshot, "entries" | "leaves" | "workerLeases" | "workerRuntimes" | "workerQueue"> & {
  entries: SessionEntry[]; leaves: SessionEntry[];
  workerLeases: WorkerLease[]; workerRuntimes: WorkerRuntimeRecord[]; workerQueue: QueuedWorkerRequest[];
};

/** UI narrowing of `AgentDefinition.role` (a contract string). */
export type AgentRole = "orchestrator" | "research" | "implementation" | "verification" | "planning" | "documentation";

/** UI narrowing of `QueuedWorkerRequest.queueStatus` (a contract string). */
export type WorkerQueueStatus = "queued" | "blocked_on_human" | "dispatching" | "dispatched" | "expired" | "cancelled" | "rejected" | "dead_letter";

/** Every trigger a learning run can report, local and external. */
export type LearningTriggerKind = LocalLearningTriggerKind | ExternalLearningTriggerKind;

// ---------------------------------------------------------------------------
// Notification payloads without a contracted schema yet.
// ---------------------------------------------------------------------------

export interface TerminalChunk { sessionId: string; terminalId: string; data: string }
export interface TerminalExit { sessionId: string; terminalId: string }

/** `memory-changed` refetch hint: names the scope, never carries a record. */
export interface MemoryChangedPayload { scopeKey: string }

// ---------------------------------------------------------------------------
// Memory aggregations. Read-only, display-only. Served today by the derived
// mock layer in `api.ts`; the protocol-first `memory.recall_stats` Rust+daemon
// method is the tracked follow-up, and these shapes are what it will return.
// ---------------------------------------------------------------------------

/** Per-record recall aggregation over the packet-injection audit. */
export interface MemoryRecallStat {
  id: string;
  recalls: number;
  /** Day bucket of the last recall (0 = 13 days ago … 13 = today), -1 if never. */
  lastRecalledDay: number;
  /** recalls / total injections — how often this record made the packet. */
  inPacketRatio: number;
  /** 14-day recall series, oldest first. */
  daily: number[];
}

/** The recall-analytics payload for a scope. */
export interface MemoryRecallStats {
  perRecord: MemoryRecallStat[];
  injectionsPerDay: number[];
  budgetCharsUsed: number;
  budgetCharsMax: number;
}

/** The closed consolidation op vocabulary from `memory_consolidation.rs`. */
export type MemoryConsolidationOp = "merge" | "correct" | "expire" | "group" | "retire" | "keep";

/** One entry in the consolidation log. */
export interface MemoryConsolidationEntry {
  op: MemoryConsolidationOp;
  detail: string;
  /** Day bucket, 0 = 13 days ago … 13 = today. */
  day: number;
}

/** `session-startup` cold-start phase, observed at a real adapter launch
 *  boundary. Transient and best-effort: never replayed, never a timer. */
export type SessionStartupPhase = "spawning" | "handshake" | "session_open";
export interface SessionStartupPayload { sessionId: string; phase: SessionStartupPhase }

// ---------------------------------------------------------------------------
// Deferred result shapes (`resultDeferred` methods). Hand-written until their
// contract slice lands; keep field-for-field with the core serializers.
// ---------------------------------------------------------------------------

export type CanonicalWorkerRole = "research" | "implementation" | "verification" | "planning" | "documentation";
export interface ModelProfile extends ModelProfileDraft {
  schemaVersion: number; version: number; profileId: string; canonicalRole: CanonicalWorkerRole; createdAt: string;
}
export interface ModelSetupState { complete: boolean; activeVersion: number | null; profiles: ModelProfile[] }

export interface OpenCodeAuthMethod { kind: string; label: string }
export interface OpenCodeModel {
  id: string; providerId: string; modelId: string; label: string;
  reasoning: boolean; toolCall: boolean; attachment: boolean;
  contextWindow: number | null; outputLimit: number | null;
  inputCost: number | null; outputCost: number | null;
}
export interface OpenCodeProvider {
  id: string; name: string; connected: boolean; source: string | null;
  environmentVariables: string[]; defaultModel: string | null;
  authMethods: OpenCodeAuthMethod[]; models: OpenCodeModel[];
}
export interface OpenCodeCatalog { executablePath: string; version: string; providers: OpenCodeProvider[] }

export type LearningRunStatus = "queued" | "running" | "completed" | "failed" | "cancelled" | "noop";
export interface LearningReport {
  reason: string; evidenceBoundary: number; evidenceCount: number; basePolicyVersion: number;
  candidatePolicyVersion: number | null; qualityBps: number | null; averageCostMicrousd: number | null;
  averageLatencyMs: number | null; retryRateBps: number | null; interventionRateBps: number | null;
  averageConfidenceBps: number | null; costComplete: boolean; evaluatedSpendMicrousd: number;
  evaluatedTokens: number; evaluationExecution: "not_run" | "deterministic_only" | "reused_existing_evidence" | "queued" | "executed" | "evaluation_failed" | "deferred"; replayPassed: boolean | null; promotionStatus: string;
  policyDiff: Record<string, unknown>; recommendationOnly: boolean;
}
export interface LearningRun {
  id: string; jobId: string; triggerKind: LearningTriggerKind; idempotencyKey: string;
  evidenceBoundary: number; basePolicyVersion: number; status: LearningRunStatus;
  report: LearningReport | null; candidatePolicyVersion: number | null; cancellationRequested: boolean;
  leaseExpiresAt: string | null; replayPassed: boolean | null; promotionStatus: string;
  duplicate: boolean; createdAt: string; completedAt: string | null;
}
export interface LearningState {
  schedule: LearningSchedule; latestRun: LearningRun | null;
  activePolicyVersion: number; canaryPolicyVersion: number | null;
  rollbackTargetVersion: number | null;
}

export interface MarketplaceVariant {
  provider: MarketplaceProvider; pluginId: string; name: string; description: string | null;
  marketplace: string | null; version: string | null; source: string | null; repository: string | null; iconDataUrl: string | null;
  publisher: string | null; capabilities: string[]; mcpEndpoint: string | null; connectorType: string | null;
  appConnectorIds: string[];
  installed: boolean; enabled: boolean; authenticationState: string; sharedAuthMechanism: string | null;
  portableMcp: boolean; compatibilityNotes: string[]; supportedActions: MarketplaceAction[]; providerMetadata: Record<string, unknown>;
}
export interface MarketplaceProviderCatalog {
  provider: MarketplaceProvider; available: boolean; variants: MarketplaceVariant[]; error: string | null;
}
export interface MarketplaceCatalog { providers: MarketplaceProviderCatalog[] }
export interface MarketplaceAppAuthState {
  provider: MarketplaceProvider; connectorId: string; displayName: string | null; nativeConnector: boolean;
  authenticationState: "connected" | "required";
}
export interface MarketplaceActionResult {
  provider: MarketplaceProvider; pluginId: string; action: MarketplaceAction;
  success: boolean; message: string; error: string | null;
}

export interface SkillProviderState {
  provider: SkillProvider; installed: boolean; managed: boolean; installedRef: string | null;
  updateAvailable: boolean; rollbackAvailable: boolean; receiptError: string | null;
}
export interface CommunitySkill {
  id: string; slug: string; name: string; description: string; source: string; sourceUrl: string;
  pinnedRef: string; installs: number; official: boolean; compatibility: SkillProvider[]; fileCount: number;
  permissions: string[]; risk: string; riskSummary: string; categories: string[]; providerStates: SkillProviderState[];
}
export interface PersonalSkill { id: string; name: string; description: string; providers: SkillProvider[]; source: string }
export interface SkillCatalog { community: CommunitySkill[]; personal: PersonalSkill[]; installer: string }
export interface CapabilitySuggestion {
  id: string; name: string; command: string; relevance: string; source: string; providers: SkillProvider[];
  permissions: string[]; risk: string; installed: boolean;
}
export interface SkillPreview {
  confirmationId: string; expiresAt: string; action: SkillAction; skill: CommunitySkill;
  targets: SkillProvider[]; changes: string[]; installer: string;
}
export interface SkillActionResult {
  provider: SkillProvider; action: SkillAction; success: boolean; message: string; error: string | null;
}

export interface AutomationSchedule { kind: "cron" | "rrule" | string; expression: string; human: string }
export type AutomationCapability = "create" | "edit" | "runNow" | "pause" | "resume" | "delete";
export interface AutomationRun {
  id: string; automationId: string; status: string; title: string | null; summary: string | null; createdAt: number | null;
}
export interface UnifiedAutomation {
  id: string; provider: AutomationProvider; name: string; prompt: string; schedule: AutomationSchedule;
  status: "active" | "paused" | "unknown"; recurring: boolean;
  createdAt: number | null; nextRunAt: number | null; lastRunAt: number | null;
  cwds: string[]; model: string | null; effort: string | null; runs: AutomationRun[];
}
export interface AutomationProviderState {
  provider: AutomationProvider; available: boolean; detail: string; count: number; capabilities: AutomationCapability[];
}
export interface AutomationCatalog { automations: UnifiedAutomation[]; providers: AutomationProviderState[] }
export interface AutomationActionResult {
  provider: AutomationProvider; id: string; action: AutomationAction; success: boolean; message: string;
}
export interface AutomationSaveResult {
  provider: AutomationProvider; id: string; created: boolean; message: string;
}

export interface BrowserTab {
  id: number; title: string; url: string; domain: string | null; favIconUrl: string | null; attached: boolean;
}
export interface BrowserLease {
  id: string; tabId: number; domain: string; status: string; permission: "read_only" | "interact";
  attachedAt: string; expiresAt: string; lastActivityAt: string;
}
export interface BrowserAuditEvent {
  id: string; kind: string; summary: string; commandId: string | null; domain: string | null; createdAt: string; data: Record<string, unknown>;
}
export interface BrowserElement {
  id: string; role: string; name: string; tag: string; value: string | null; disabled: boolean;
  sensitiveKind: string | null; contentBoundary: "untrusted_web_content"; promptInjectionSuspected: boolean;
  bounds: { x: number; y: number; width: number; height: number };
}
export interface BrowserTokenAccounting {
  snapshots: number; fullSnapshots: number; deltaSnapshots: number; serializedBytes: number;
  estimatedInputTokens: number; screenshotCount: number;
}
export interface BrowserApproval {
  id: string; commandId: string; action: string; effect: string; domain: string; createdAt: string;
}
export interface BrowserSiteMetric {
  domain: string; actions: number; successes: number; failures: number; totalLatencyMs: number;
  inputTokens: number; screenshots: number; interventions: number; approvals: number; duplicateSideEffects: number;
}
export interface BrowserBridgeSnapshot {
  transportConnected: boolean; extensionId: string; extensionPath: string; nativeHostInstalled: boolean;
  nativeHostManifestPath: string | null; tabs: BrowserTab[]; lease: BrowserLease | null; status: string;
  captureActive: boolean; captureError: string | null;
  screenshot: string | null; screenshotRedactedRegions: number; elements: BrowserElement[];
  viewport: { width?: number; height?: number; scrollX?: number; scrollY?: number } | null;
  promptInjectionSuspected: boolean; promptInjectionSignals: string[]; tokenAccounting: BrowserTokenAccounting; pendingApproval: BrowserApproval | null;
  audit: BrowserAuditEvent[]; debugEvents: Record<string, unknown>[]; siteMetrics: BrowserSiteMetric[];
  remoteProvider: RemoteBrowserConfig | null;
}
