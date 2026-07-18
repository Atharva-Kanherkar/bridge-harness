export type Harness = "claude" | "codex" | "opencode" | "shell";
export type SessionStatus = "idle" | "starting" | "working" | "waiting" | "warm" | "checkpointing" | "ready" | "stopped" | "resuming" | "restored" | "failed" | "completed" | "cancelled";
export type CapabilityTier = "fast" | "standard" | "strong";
export type ReasoningEffort = "low" | "medium" | "high" | "xhigh";
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
  runtimeMs: number | null; costMicrousd: number | null; costSource: string | null; source: string; createdAt: string;
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
  completion: CompletionSummary | null;
}
export type CompletionVerdict = "verifying" | "changes_requested" | "verified" | "waived" | "failed" | "superseded";
export type EvalKind = "deterministic" | "scrutiny" | "user_testing";
export type CheckStatus = "pending" | "running" | "passed" | "failed" | "skipped" | "blocked" | "stale";
export interface CompletionCheckRun {
  checkId: string; kind: EvalKind; required: boolean; status: CheckStatus; executor: string;
  command: string | null; verifierFamily: string | null; detail: string | null;
  outputDigest: string | null; artifactRefs: string[];
}
export interface CompletionSummary {
  attemptId: string; contractId: string; verdict: CompletionVerdict;
  repository: { head: string; dirtyDigest: string };
  passedRequired: number; totalRequired: number; checks: CompletionCheckRun[];
  markdownCommitted: boolean; waiverReason: string | null;
}
export interface VerifierManifest {
  id: string; kind: EvalKind; triggers: string[]; requiredCapabilities: string[];
  differentModelFamily: boolean; checks: string[]; evidenceRequired: string[];
}
export interface VerifierCandidate { manifest: VerifierManifest; eligible: boolean; exclusionReasons: string[] }
export interface ModelOption { id: string; label: string; tier: CapabilityTier; defaultForTier: boolean }
export interface AdapterDescriptor {
  id: string; label: string; available: boolean; version: string | null; capabilities: string[];
  unavailableReason: string | null; models: ModelOption[]; defaultModel: string | null;
}
export interface BridgeState { projects: Project[]; workspaces: Workspace[]; sessions: Session[]; events: BridgeEvent[] }
export interface Health { ok: boolean; version: string; harnesses: Record<Harness, boolean>; database: string; adapters: AdapterDescriptor[] }
export type RouterMode = "disabled" | "shadow" | "autonomous";
export interface RouterPreferences {
  mode: RouterMode; minimumPassBps: number; pinnedHarness: string | null; pinnedModel: string | null;
  excludedHarnesses: string[]; excludedModels: string[];
}
export type ProfilePurpose = "standard_orchestrator" | "premium_orchestrator" | "planner" | "implementer" | "verifier" | "reviewer" | "research" | "documentation" | "evaluator";
export type CanonicalWorkerRole = "research" | "implementation" | "verification" | "planning" | "documentation";
export interface ModelProfileDraft {
  purpose: ProfilePurpose; provider: string; model: string; effort: ReasoningEffort;
  fallbackPurpose: ProfilePurpose | null; pinned: boolean; learningEnabled: boolean;
  budgetPreference: string | null; latencyPreference: string | null;
}
export interface ModelProfile extends ModelProfileDraft {
  schemaVersion: number; version: number; profileId: string; canonicalRole: CanonicalWorkerRole; createdAt: string;
}
export interface ModelSetupState { complete: boolean; activeVersion: number | null; profiles: ModelProfile[] }
export interface HarnessConfig {
  id: "bridge" | "codex" | "claude" | "opencode"; label: string; enabled: boolean;
  defaultModel: string | null; effort: ReasoningEffort | null; systemPrompt: string;
  advanced: Record<string, unknown>; isOverride: boolean;
}
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
export type AgentRole = "orchestrator" | "research" | "implementation" | "verification" | "planning" | "documentation";
export interface AgentDefinition {
  id: string; name: string; description: string; role: AgentRole; harness: "bridge" | "codex" | "claude" | "opencode";
  model: string | null; effort: ReasoningEffort; systemPrompt: string; enabled: boolean;
  isDefault: boolean; isBuiltIn: boolean; createdAt: string; updatedAt: string;
}
export interface ConfigState { harnesses: HarnessConfig[]; agents: AgentDefinition[]; defaultAgentId: string }
export type LearningTriggerKind = "manual" | "in_app" | "codex" | "claude" | "opencode";
export type LearningRunStatus = "queued" | "running" | "completed" | "failed" | "cancelled" | "noop";
export interface LearningReport {
  reason: string; evidenceBoundary: number; evidenceCount: number; basePolicyVersion: number;
  candidatePolicyVersion: number | null; qualityBps: number | null; averageCostMicrousd: number | null;
  averageLatencyMs: number | null; retryRateBps: number | null; interventionRateBps: number | null;
  averageConfidenceBps: number | null; costComplete: boolean; evaluatedSpendMicrousd: number;
  evaluatedTokens: number; evaluationExecution: "not_run" | "deterministic_only" | "reused_existing_evidence" | "deferred"; replayPassed: boolean | null; promotionStatus: string;
  policyDiff: Record<string, unknown>; recommendationOnly: boolean;
}
export interface LearningRun {
  id: string; jobId: string; triggerKind: LearningTriggerKind; idempotencyKey: string;
  evidenceBoundary: number; basePolicyVersion: number; status: LearningRunStatus;
  report: LearningReport | null; candidatePolicyVersion: number | null; cancellationRequested: boolean;
  leaseExpiresAt: string | null; replayPassed: boolean | null; promotionStatus: string;
  duplicate: boolean; createdAt: string; completedAt: string | null;
}
export interface LearningSchedule {
  jobId: string; enabled: boolean; cadenceMinutes: number; nextRunAt: string | null;
  runBudgetMicrousd: number; runBudgetTokens: number; mode: "manual" | "ask" | "automatic";
}
export interface LearningState {
  schedule: LearningSchedule; latestRun: LearningRun | null;
  activePolicyVersion: number; canaryPolicyVersion: number | null;
}
export interface TerminalChunk { sessionId: string; data: string }
export interface SlashCommand { name: string; description: string; harness: Harness; kind: "command" | "skill" | "prompt" | "builtin" }
export interface SecretInterception { reference: string; detector: string }
export interface SanitizedTurn { text: string; interceptions: SecretInterception[] }
export type MarketplaceProvider = "codex" | "claude";
export type MarketplaceAction = "install" | "enable" | "disable" | "update" | "uninstall" | "authenticate";
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
export type SkillProvider = "codex" | "claude" | "opencode";
export type SkillAction = "install" | "rollback" | "uninstall";
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
