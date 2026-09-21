import { mockTerminalWorkspace, mockCreateTerminal, mockSnapshot, mockSaveLayout, mockRenameTerminal, mockCloseTerminal } from "./terminal/mock";
import type { TerminalRecord, TerminalSnapshot, TerminalWorkspace, CreateTerminalParams, TerminalFrame } from "./terminal/types";
import { recordStreamReceipt } from "./streamTiming";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { MENU_COMMAND_EVENT, type CommandId } from "./keymap";
import { normalizeAgentToken } from "./agentMention";
import { createInvokeQueue } from "./invokeQueue";
import { asWireKind, readWireKind } from "./transcript/wire";
import type { AgentDefinition, ArchiveChatResult, AgentEvent, ApprovalDecision, AutomationAction, AutomationActionResult, AutomationCatalog, AutomationProvider, BaseBranchDivergence, BridgeState, BrowserActionRequest, BrowserBridgeSnapshot, BrowserFrame, BrowserRouteDecision, BrowserRouteRequest, BrowserSkill, CapabilitySuggestion, CompletionCheckRun, CompletionSummary, ConfigState, CompiledPromptPreviewResult, ExternalLearningTriggerKind, PermissionPolicy, Harness, HarnessConfig, Health, LearningRun, LearningSchedule, LearningState, ListMemoryRecordsResult, LocalLearningTriggerKind, MarketplaceAction, MarketplaceActionResult, MarketplaceAppAuthState, MarketplaceCatalog, MarketplaceProvider, MemoryCapabilities, MemoryChangedPayload, MemoryExtractionSettings, MemoryInjectionSettings, MemoryPacketAudit, MemoryRecord, ModelProfileDraft, ModelSetupState, OpenCodeCatalog, PromptProviderLayerStatus, PromptRevisionView, PromptSectionMutationResult, PromptSectionStatePayload, PromptStackView, PromptTargetChoice, RemoteBrowserConfig, RouterPreferences, SanitizedTurn, ExportSessionTranscriptResult, TranscriptExportScope, SearchSessionEntriesResult, SessionEntry, SessionStartupPayload, TerminalExit, SessionForestSnapshot, SkillAction, SkillActionResult, SkillCatalog, SkillPreview, SkillProvider, SlashCommand, SlashCommandResolve, TerminalChunk, VerifierCandidate, VerifierManifest, WorkerRepositoryBinding, WorktreeInventoryEntry, WorktreeReclaimResult, WorktreeSweepResult, WorktreeUsage } from "./types";
import type { AutomationSaveResult, SaveAutomationParams } from "./types";
import type { ScanHistoryParams, ScanHistoryResult, SetPriceOverrideParams, SummaryParams, UsageBucket, UsageHistorySource, UsagePriceOverride, UsagePricingStatus, UsageSummaryResult } from "./types";
import type { MeterRegistry, InsightsParams, UsageInsightsResult } from "./types";
import type { MemoryRecallStats, MemoryConsolidationEntry } from "./types";
import { deriveRecallStats, PACKET_BUDGET_CHARS, type PacketInjection } from "./memoryStats";
import { BRIDGE_METHODS, type BridgeMethod, type BridgeMethodParams, type BridgeMethodResults, type BridgeNotification, type ContextBreakdownResult, type ForkSessionResult, type ResolveReferenceResult } from "./protocol/generated/protocol";
import type { TurnImage, ArchivedChatsResult, ReviewerSettings, ReviewerSettingsResult, WorkerSettings } from "./protocol/generated/protocol";
import type {
  CommitExternalImportParams,
  DiscoverExternalImportParams,
  ExternalImportCandidate,
  ExternalImportCommit,
  ExternalImportDiscovery,
  ExternalImportPlan,
  ExternalImportPreview,
  PreviewExternalImportParams,
} from "./protocol/generated/protocol";
import type { ComposerAttachment } from "./pasteAttachments";
import type { GithubCiFinishedPayload } from "./githubSurface";
import type {
  ManagedAgentInspection,
  ManagedAgentList,
  ManagedAgentOperationKind,
  ManagedAgentOperationResult,
  ManagedAgentStatus,
  ListWorkspaceBranchesResult,
  ReadWorkspaceFileResult,
  DispatchAgentShortcutResult,
  SubmitInputResult,
  WorkspaceChangesResult,
  WorkBoard,
  WorkBriefReceipt,
  WorkBriefingOptions,
  WorkSettings,
  WorkSettingsSnapshot,
  WorkTask,
  WorkTaskDraft,
  WriteWorkspaceFileResult,
  GithubAction,
  GithubActResult,
  GithubReviewResult,
  GithubCheckoutResult,
  GithubConnectResult,
  SearchGithubReposResult,
  GithubChecksResult,
  GithubIssueResult,
  ConnectorActionRequest,
  ConnectorActResult,
  ConnectorDismissResult,
  ConnectorInboxItem,
  ConnectorInboxResult,
  ConnectorListResult,
  ConnectorRefreshResult,
  ConnectorSetSettingsResult,
  GithubIssuesResult,
  GithubMergeConfigResult,
  GithubPullRequestResult,
  GithubPullRequestsResult,
  GithubRepositoryResult,
  GithubStatusResult,
  InteractionResolutionResult,
  CreateAsideChatResult,
  QuestionAction,
  SuggestCompletionResult,
  SuggestionSettings,
  SuggestionSettingsSnapshot,
} from "./protocol/generated/protocol";
import type { AccountUsagePayload } from "./usage";
import type { MenuBarSettings, UsageOverviewSnapshot, ProviderUsageOverviews } from "./protocol/generated/protocol";
import type {
  ConnectorCardReadyPayload,
  ConnectorItemArrivedPayload,
  ConnectorItemResolvedPayload,
} from "./connectorSurface";
import { recommendedProfileDrafts } from "./modelProfiles";

const isTauri = () => typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
let mockMenuBarSettings: MenuBarSettings = {
  schemaVersion: 1, enabled: true, codexEnabled: true, claudeEnabled: false, cursorEnabled: false, opencodeEnabled: false, selectedProvider: "codex", opencodeWorkspace: null, displayMode: "remaining", quotaWindow: "auto",
  showAccount: true, showTokens: true, showCost: true, refreshSeconds: 300,
};

// Mock-mode provider overviews so the chat's usage dot has something to draw
// under `bun run dev`: a Codex account with a quiet session window and a busy
// weekly one, and a Claude read that failed. Fresh at call time by design.
function mockProviderUsageOverviews(): ProviderUsageOverviews {
  const now = Math.floor(Date.now() / 1000);
  const empty = { tokens: { status: "unavailable" as const }, costMicrousd: { status: "unavailable" as const }, models: [] };
  return {
    schemaVersion: 1,
    generatedAt: now,
    providers: [
      {
        schemaVersion: 1, generatedAt: now, provider: "codex", account: "dev@example.com", plan: "plus", observedAt: now - 30, coverage: "Mock data",
        windows: [
          { id: "session", label: "5-hour", usedPercent: { value: 4, source: "reported", status: "current" }, resetsAt: now + 4 * 3600, windowMinutes: 300 },
          { id: "weekly", label: "Weekly", usedPercent: { value: 63, source: "reported", status: "current" }, resetsAt: now + 3 * 86400, windowMinutes: 10080 },
        ],
        today: empty, month: empty, error: null,
      },
      { schemaVersion: 1, generatedAt: now, provider: "claude", observedAt: null, coverage: "Mock data", windows: [], today: empty, month: empty, error: "Claude Code usage SDK unavailable. Open Claude Code and check its sign-in." },
    ],
  };
}

// The typed protocol boundary. Every Tauri round-trip goes through these two
// helpers, so params, results, and event names all come from the generated
// contract: renaming a wire field breaks `bun run check`, not a user session.
const COMMAND_BY_METHOD = Object.fromEntries(
  BRIDGE_METHODS.map(entry => [entry.method, entry.command]),
) as Record<BridgeMethod, string>;

// Match daemon_host.rs's connection partitions. Sending the whole UI fan-out
// at once exhausted its 24-job limit; slow GitHub calls also need to stay out
// of the lanes used by settings, sessions, and health.
const generalInvokes = createInvokeQueue(4);
const githubInvokes = createInvokeQueue(2);

function call<M extends BridgeMethod>(
  method: M,
  ...params: BridgeMethodParams[M] extends undefined ? [] : [BridgeMethodParams[M]]
): Promise<BridgeMethodResults[M]> {
  const enqueue = method.startsWith("github/") ? githubInvokes : generalInvokes;
  return enqueue(() => invoke(COMMAND_BY_METHOD[method], params[0] as Record<string, unknown> | undefined));
}

const subscribe = <T,>(notification: BridgeNotification, handler: (payload: T) => void): Promise<UnlistenFn> =>
  listen<T>(notification, event => handler(event.payload));

/**
 * Shell-to-webview transport for the live agent stream, deliberately not a
 * protocol notification: `bridged` still speaks `agent-event`, one frame per
 * notification, and the daemon contract is unchanged. The Tauri shell batches
 * a flush window's worth of those into one message under this name
 * (`src-tauri/src/agent_batch.rs`), and `onAgentEvent` unpacks it.
 */
const AGENT_EVENT_BATCH = "agent-event-batch";

/** Adapt a contract `UnitResult` (null) to the `Promise<void>` the app uses. */
const unit = (result: Promise<null>): Promise<void> => result.then(() => undefined);
const now = new Date().toISOString();
const stateListeners = new Set<() => void>();
const memoryListeners = new Set<(payload: MemoryChangedPayload) => void>();
type GithubChecksChangedPayload = { workspaceId: string; number: number };
const githubCiListeners = new Set<(payload: GithubCiFinishedPayload) => void>();
// Browser-mode stand-in for the daemon's global `agent-event` fan-out. Every
// surface that renders live turns (the aside panel above all — its optimistic
// pending rows reconcile only against this stream) subscribes here outside
// Tauri, so mock turns must be delivered, not just persisted to mock state.
const agentListeners = new Set<(event: AgentEvent) => void>();
const mockRouterPreferences = new Map<string, RouterPreferences>();
const mockVerifierManifests = new Map<string, VerifierManifest>();
let mockModelSetup: ModelSetupState = { complete: false, activeVersion: null, profiles: [] };
let mockConfigState: ConfigState = {
  harnesses: [
    { id: "bridge", label: "Bridge", enabled: true, defaultModel: null, effort: null, systemPrompt: "", advanced: {}, isOverride: false },
    { id: "codex", label: "Codex", enabled: true, defaultModel: null, effort: null, systemPrompt: "", advanced: {}, isOverride: false },
    { id: "claude", label: "Claude Code", enabled: true, defaultModel: null, effort: null, systemPrompt: "", advanced: {}, isOverride: false },
    { id: "cursor", label: "Cursor", enabled: true, defaultModel: null, effort: null, systemPrompt: "", advanced: {}, isOverride: false },
    { id: "opencode", label: "OpenCode", enabled: true, defaultModel: null, effort: null, systemPrompt: "", advanced: {}, isOverride: false },
  ],
  agents: [
    { id: "bridge-orchestrator", name: "Bridge orchestrator", description: "Plans, routes, and owns the final answer.", role: "orchestrator", harness: "bridge", model: null, effort: "medium", systemPrompt: "", enabled: true, isDefault: true, isBuiltIn: true, createdAt: "", updatedAt: "" },
    { id: "bridge-research", name: "Research agent", description: "Collects scoped evidence and findings.", role: "research", harness: "bridge", model: null, effort: "medium", systemPrompt: "", enabled: true, isDefault: false, isBuiltIn: true, createdAt: "", updatedAt: "" },
    { id: "bridge-implementation", name: "Implementation agent", description: "Makes focused code changes.", role: "implementation", harness: "bridge", model: null, effort: "medium", systemPrompt: "", enabled: true, isDefault: false, isBuiltIn: true, createdAt: "", updatedAt: "" },
    { id: "bridge-verification", name: "Verification agent", description: "Tests outcomes independently.", role: "verification", harness: "bridge", model: null, effort: "high", systemPrompt: "", enabled: true, isDefault: false, isBuiltIn: true, createdAt: "", updatedAt: "" },
    { id: "bridge-planning", name: "Planning agent", description: "Turns ambiguous work into an executable plan.", role: "planning", harness: "bridge", model: null, effort: "high", systemPrompt: "", enabled: true, isDefault: false, isBuiltIn: true, createdAt: "", updatedAt: "" },
    { id: "bridge-documentation", name: "Documentation agent", description: "Produces concise project documentation.", role: "documentation", harness: "bridge", model: null, effort: "low", systemPrompt: "", enabled: true, isDefault: false, isBuiltIn: true, createdAt: "", updatedAt: "" },
  ],
  defaultAgentId: "bridge-orchestrator",
  permissionPolicy: { autoApproveProviderPermissions: false, workerPromptProposalRoles: [], updatedAt: "" },
};
let mockOpenCodeCatalog: OpenCodeCatalog = {
  executablePath: "/usr/local/bin/opencode",
  version: "1.18.3",
  providers: [{
    id: "opencode-go", name: "OpenCode Go", connected: true, source: "api",
    environmentVariables: [], defaultModel: "opencode-go/kimi-k2.5",
    authMethods: [{ kind: "api", label: "API key" }],
    models: [{ id: "opencode-go/kimi-k2.5", providerId: "opencode-go", modelId: "kimi-k2.5", label: "Kimi K2.5", reasoning: true, toolCall: true, attachment: true, contextWindow: 262144, outputLimit: 65536, inputCost: null, outputCost: null }],
  }],
};
let mockLearningState: LearningState = {
  schedule: { jobId: "default", enabled: false, cadenceMinutes: 1440, nextRunAt: null, runBudgetMicrousd: 100_000, runBudgetTokens: 50_000, mode: "manual" },
  latestRun: null,
  activePolicyVersion: 1,
  canaryPolicyVersion: null,
  rollbackTargetVersion: null,
};

// Prompt Studio browser-mode state: overrides keyed "target:sectionId" with
// an append-only revision list per key. Default texts are demo stand-ins, not
// the real Rust defaults.
type MockPromptRevision = PromptRevisionView;
const mockPromptSections = new Map<string, { state: PromptSectionStatePayload; revisions: MockPromptRevision[] }>();
let nextMockPromptRevisionId = 1;
const MOCK_PROMPT_MAX_DEPTH = 1; // delegation::DEFAULT_MAX_DEPTH
// prompt_studio::MAX_REVISIONS_IN_VIEW — the mock mirrors the native
// bounded history window so views cannot grow without limit.
const MOCK_PROMPT_MAX_REVISIONS_IN_VIEW = 50;
const utf8Bytes = (text: string): number => new TextEncoder().encode(text).length;

/// The patch the mock transcript's edit carries. Two hunks, so the dev preview
/// shows the first inline and folds the second behind the fold bar.
const MOCK_PATCH = [
  "@@ -18,8 +18,11 @@ export class TokenStore {",
  "   async read(scope: Scope): Promise<Token | null> {",
  "-    const row = await this.db.get(scope.id);",
  "-    return row ? JSON.parse(row.value) : null;",
  "+    // One read, one parse: the old pair of awaits could observe a write",
  "+    // landing between them and hand back a token for the previous scope.",
  "+    const row = await this.db.getScoped(scope);",
  "+    if (!row) return null;",
  "+    return Token.parse(row.value);",
  "   }",
  "@@ -44,4 +47,5 @@ export class TokenStore {",
  "   async revoke(scope: Scope): Promise<void> {",
  "+    await this.db.deleteScoped(scope);",
  "   }",
].join("\n");

const MOCK_PROMPT_DEFAULTS: Record<PromptTargetChoice, { id: string; text: string }[]> = {
  orchestrator: [
    { id: "bridge_role", text: "You are Bridge's starter orchestrator: a planner and router." },
    { id: "delegation_protocol", text: "## Delegating work\nEmit one fenced bridge-delegate JSON object after a short sentence naming the role and reason." },
    { id: "additional_guidance", text: "" },
  ],
  "worker:research": [{ id: "worker_contract", text: "You are a research worker. Collect scoped evidence and report back." }, { id: "additional_guidance", text: "" }],
  "worker:implementation": [{ id: "worker_contract", text: "You are an implementation worker. Make one focused change." }, { id: "additional_guidance", text: "" }],
  "worker:verification": [{ id: "worker_contract", text: "You are a verification worker. Verify outcomes independently." }, { id: "additional_guidance", text: "" }],
  "worker:planning": [{ id: "worker_contract", text: "You are a planning worker. Turn ambiguous work into an executable plan." }, { id: "additional_guidance", text: "" }],
  "worker:documentation": [{ id: "worker_contract", text: "You are a documentation worker. Produce concise project documentation." }, { id: "additional_guidance", text: "" }],
  direct_session: [],
};

function mockPromptDepth(depth?: number): number {
  const resolved = depth ?? 0;
  if (!Number.isInteger(resolved) || resolved < 0 || resolved > MOCK_PROMPT_MAX_DEPTH) {
    throw new Error(`worker depth ${resolved} is outside the supported range 0..=${MOCK_PROMPT_MAX_DEPTH}`);
  }
  return resolved;
}

function mockPromptCompilerRole(target: PromptTargetChoice): string {
  return target === "direct_session" ? "session" : target;
}

function mockPromptLint(sectionId: string, text: string | null) {
  if (text === null || sectionId !== "delegation_protocol") return [];
  return text.includes("bridge-delegate") ? [] : [{
    marker: "bridge-delegate",
    message: "Typed delegation may stop working because `bridge-delegate` is missing.",
  }];
}

function mockPromptStack(target: PromptTargetChoice, depth?: number): PromptStackView {
  return {
    target,
    depth: depth ?? 0,
    sections: MOCK_PROMPT_DEFAULTS[target].map(({ id, text }) => {
      const record = mockPromptSections.get(`${target}:${id}`);
      const state = record?.state ?? { state: "default" } as PromptSectionStatePayload;
      const effectiveText = state.state === "deleted" ? null : state.state === "overridden" ? state.text : text;
      const bytes = effectiveText === null ? 0 : utf8Bytes(effectiveText);
      return {
        id,
        state,
        defaultText: text,
        effectiveText,
        bytes,
        tokenEstimate: Math.ceil(bytes / 4),
        lintWarnings: mockPromptLint(id, effectiveText),
        revisions: (record?.revisions ?? []).slice(-MOCK_PROMPT_MAX_REVISIONS_IN_VIEW).map(revision => structuredClone(revision)),
      };
    }),
  };
}

function mockPromptMutation(
  target: PromptTargetChoice,
  sectionId: string,
  depth: number,
  state: PromptSectionStatePayload,
  restoredFromRevisionId?: number,
): PromptSectionMutationResult {
  if (!MOCK_PROMPT_DEFAULTS[target].some(section => section.id === sectionId)) {
    throw new Error(`prompt section "${sectionId}" is not available for target ${target}`);
  }
  const key = `${target}:${sectionId}`;
  const record = mockPromptSections.get(key) ?? { state: { state: "default" } as PromptSectionStatePayload, revisions: [] };
  const revision: MockPromptRevision = {
    id: nextMockPromptRevisionId++,
    operation: restoredFromRevisionId != null ? "restore" : state.state === "default" ? "reset" : "override",
    state: structuredClone(state),
    restoredFromRevisionId: restoredFromRevisionId ?? null,
    createdAt: new Date().toISOString(),
  };
  record.state = structuredClone(state);
  record.revisions.push(revision);
  mockPromptSections.set(key, record);
  return { revision, stack: mockPromptStack(target, depth) };
}

/** Settings-wide reset deletes every override but appends a reset revision per
 * touched key, exactly like the native path's append_reset_all_revisions. */
function mockPromptResetAll() {
  for (const [key, record] of [...mockPromptSections.entries()]) {
    if (record.state.state === "default") continue;
    record.revisions.push({
      id: nextMockPromptRevisionId++,
      operation: "reset",
      state: { state: "default" },
      restoredFromRevisionId: null,
      createdAt: new Date().toISOString(),
    });
    record.state = { state: "default" };
    mockPromptSections.set(key, record);
  }
}

async function mockPromptPreview(target: PromptTargetChoice, depth?: number): Promise<CompiledPromptPreviewResult> {
  const stack = mockPromptStack(target, depth);
  // Same envelope shape the real compiler serializes: sorted stable keys.
  const stableSections = Object.fromEntries(stack.sections.filter(section => section.effectiveText != null && section.effectiveText.trim() !== "").map(section => [section.id, section.effectiveText]));
  const envelope = JSON.stringify({ schemaVersion: 1, role: mockPromptCompilerRole(target), stableSections, toolSchemas: {}, projectRules: {} });
  const stablePrefix = `<bridge-stable-prompt schema="1">\n${envelope}\n</bridge-stable-prompt>`;
  const variableSuffix = '<bridge-variable-context>\n{"sections":[]}\n</bridge-variable-context>';
  const providerLayers: PromptProviderLayerStatus[] = [
    ["claude", "the Claude Agent SDK compiles the preset internally and never returns it"],
    ["codex", "no app-server method returns Codex's own base agent instructions"],
    ["opencode", "OpenCode's session API has no endpoint for its provider base system prompt"],
  ].map(([adapter, detail]) => ({ layer: "provider_base", adapter, source: "unavailable", bytes: null, detail }));
  // Real digest over the exact envelope bytes — no invented hashes.
  const prefixBytes = utf8Bytes(stablePrefix);
  const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(stablePrefix));
  const prefixHash = [...new Uint8Array(digest)].map(byte => byte.toString(16).padStart(2, "0")).join("");
  return {
    target: stack.target,
    depth: stack.depth,
    stack,
    stablePrefix,
    variableSuffix,
    schemaVersion: 1,
    prefixId: `bridge-prompt-v1-${prefixHash.slice(0, 16)}`,
    prefixHash,
    prefixBytes,
    prefixTokenEstimate: Math.ceil(prefixBytes / 4),
    providerLayers,
  };
}
let nextEventId = 20;
let mockBrowserBridge: BrowserBridgeSnapshot = {
  transportConnected: false, extensionId: "jocamgijenfmpopdfecjfnjdnohhoool", extensionPath: "/path/to/browser-extension",
  nativeHostInstalled: false, nativeHostManifestPath: null, tabs: [], lease: null, status: "not_attached",
  captureActive: false, captureError: null,
  screenshot: null, screenshotRedactedRegions: 0, elements: [], viewport: null, promptInjectionSuspected: false,
  tokenAccounting: { snapshots: 0, fullSnapshots: 0, deltaSnapshots: 0, serializedBytes: 0, estimatedInputTokens: 0, screenshotCount: 0 },
  promptInjectionSignals: [], pendingApproval: null, audit: [], debugEvents: [], siteMetrics: [], remoteProvider: null,
};

let mockState: BridgeState & { agentEvents: AgentEvent[] } = {
  projects: [{ id: "demo-project", name: "Bridge", path: "/Users/you/Developer/bridge", createdAt: now }],
  workspaces: [
    { id: "demo-1", projectId: "demo-project", city: "Kyoto", title: "Build session supervisor", branch: "bridge/session-supervisor", path: "/Users/you/bridge/Kyoto", status: "working", dirtyFiles: 4, additions: 284, deletions: 31, createdAt: now },
    { id: "demo-2", projectId: "demo-project", city: "Lisbon", title: "Polish the Deck shell", branch: "bridge/deck-shell", path: "/Users/you/bridge/Lisbon", status: "ready", dirtyFiles: 7, additions: 612, deletions: 88, createdAt: now },
    { id: "demo-3", projectId: "demo-project", city: "Reykjavik", title: "Add event ledger", branch: "bridge/event-ledger", path: "/Users/you/bridge/Reykjavik", status: "ready", dirtyFiles: 0, additions: 148, deletions: 12, createdAt: now }
  ],
  sessions: [
    { id: "session-1", workspaceId: "demo-1", harness: "codex", label: "Orchestrator", status: "working", startedAt: now, endedAt: null, contextPercent: 38, usagePercent: 24, metricSource: "reported", providerSessionId: "mock-thread-1", activeTurnId: "mock-turn-1", model: "gpt-5.6-luna", requestedTier: "fast", effort: null, parentSessionId: null, depth: 0, restorationMode: "hot", continuationFidelity: "native", kind: "orchestrator" },
    { id: "session-1w", workspaceId: "demo-1", harness: "claude", label: "Implementation · strong", status: "working", startedAt: now, endedAt: null, contextPercent: 21, usagePercent: 14, metricSource: "reported", providerSessionId: "mock-claude-1", activeTurnId: "mock-turn-1w", model: "fable", requestedTier: "strong", effort: "high", parentSessionId: "session-1", depth: 1, restorationMode: "native", continuationFidelity: "native", kind: "worker" },
    { id: "session-1w2", workspaceId: "demo-1", harness: "codex", label: "Verification · strong", status: "ready", startedAt: now, endedAt: null, contextPercent: 9, usagePercent: 6, metricSource: "reported", providerSessionId: "mock-codex-2", activeTurnId: null, model: "gpt-5.6-sol", requestedTier: "strong", effort: "xhigh", parentSessionId: "session-1", depth: 1, restorationMode: "checkpoint_restored", continuationFidelity: "projected_at_boundary", kind: "worker" },
    { id: "session-2", workspaceId: "demo-2", harness: "codex", label: "Orchestrator", status: "ready", startedAt: now, endedAt: null, contextPercent: 12, usagePercent: 8, metricSource: "reported", providerSessionId: "mock-thread-2", activeTurnId: null, model: "gpt-5.6-luna", requestedTier: "fast", effort: null, parentSessionId: null, depth: 0, restorationMode: "fresh", continuationFidelity: "native", kind: "orchestrator" }
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
    agentEvent(11, "session-1", "delegation.result", { itemId: "result-1w", role: "system", status: "completed", title: "Worker result", text: "[worker result] Implementation · strong (STRONG TIER, runtime Fable, effort high) finished:\n\nAuth module refactored to the new token store.", data: { childSessionId: "session-1w", delivered: true } }),
    // A second turn, so the demo carries the things the transcript pane exists
    // to show: a turn boundary with its stamped ordinal, a thought, a tool
    // call that failed, and the turn's usage. Without one of each, mock mode
    // renders an observability surface with nothing to observe.
    agentEvent(12, "session-1", "turn.started", { status: "inProgress", data: { turnIndex: 1 } }),
    agentEvent(13, "session-1", "message.completed", { itemId: "user-2", role: "user", status: "completed", text: "Run the full suite before we land this." }),
    agentEvent(14, "session-1", "reasoning.completed", { itemId: "thought-1", status: "completed", text: "The migration touched the refresh path, so the auth suite is the one that actually exercises it. Running that before the whole tree." }),
    agentEvent(15, "session-1", "tool.completed", { itemId: "tool-2", title: "bun test src/auth", status: "failed", data: { type: "commandExecution", exitCode: 1, durationMs: 8421, aggregatedOutput: "(fail) rotation invalidates the old token\n  expected: null\n  received: Token { scope: 'session' }\n\n 41 pass\n 1 fail" } }),
    agentEvent(16, "session-1", "usage.updated", { status: "completed", data: { input_tokens: 18432, output_tokens: 611, cache_read_tokens: 16384, reasoning_tokens: 240, context_percent: 9 } }),
    agentEvent(17, "session-1", "message.completed", { itemId: "assistant-2", role: "assistant", status: "completed", text: "One test fails: the old token still verifies after a rotate. Looking at the store now." }),
    agentEvent(18, "session-1", "turn.completed", { status: "completed" })
  ]
};


function agentEvent(id: number, sessionId: string, kind: string, fields: Partial<AgentEvent> = {}): AgentEvent {
  return { id, sessionId, sequence: id, protocolVersion: 1, kind: asWireKind(kind), itemId: null, role: null, status: null, title: null, text: null, data: {}, providerMeta: { adapter: "fake" }, createdAt: new Date().toISOString(), ...fields };
}
function forestEntry(id: string, sessionId: string, sequence: number, kind: string, payload: Record<string, unknown>, parentEntryId: string | null): SessionEntry {
  return { id, sessionId, parentEntryId, sequence, semanticSchemaVersion: 2, kind, payload, providerEventId: null, contextVisibility: "eligible", tokenEstimate: null, createdAt: now };
}
const demoEntries: SessionEntry[] = [
  forestEntry("entry-1", "session-1", 1, "user.message", { text: "Build the structured session supervisor.", itemId: "user-1" }, null),
  forestEntry("entry-2", "session-1", 2, "checkpoint", { schemaVersion: 1, summary: "Policy and schema decisions are durable", decisions: ["SQLite is authoritative"] }, "entry-1"),
  forestEntry("entry-3", "session-1", 3, "assistant.message", { text: "Delegating implementation and verification." }, "entry-2"),
  forestEntry("entry-4a", "session-1", 4, "user.message", { text: "Try the direct implementation path." }, "entry-3"),
  forestEntry("entry-5a", "session-1", 5, "assistant.message", { text: "This is the inactive branch." }, "entry-4a"),
  forestEntry("entry-4b", "session-1", 6, "user.message", { text: "Use isolated workers instead." }, "entry-3"),
  forestEntry("entry-5b", "session-1", 7, "compaction", { schemaVersion: 1, summary: "Workers own isolated paths", firstRetainedEntryId: "entry-6b", tokensBefore: 9200, filesTouched: ["src-tauri/src/lib.rs"], reason: "phase_boundary", sourceAgent: "session-1" }, "entry-4b"),
  forestEntry("entry-6b", "session-1", 8, "branch.summary", { summary: "Selected isolated-worker branch" }, "entry-5b"),
  forestEntry("entry-7b", "session-1", 9, "worker.result", { status: "completed", summary: "Lifecycle implementation verified", decisions: ["Keep SQLite authoritative"], tests: ["130 Rust tests"] }, "entry-6b"),
  // A pending write-scope approval: the normal cold-start state, carrying the
  // machine-readable reason and its remediation.
  forestEntry("entry-8b", "session-1", 10, "approval.requested", { status: "pending", approvalType: "delegation_path_scope", approvalId: "delegation-path-scope:mock-turn-1:src/**", title: "Approve delegation write scope", objective: "Render Mermaid, math, and sandboxed HTML inline in chat", reason: "owned_path_provenance_required", remediation: "these write paths were proposed by the agent and were not explicitly authorized. Approve once for this turn, narrow the paths, or delegate read-only. A user message line of the form `Write scope: src/**` authorizes a scope without a card.", requestedOwnedPaths: ["src/components/**", "src/index.css"], writeMode: "isolated", role: "implementation" }, "entry-7b"),
  // A background worker blocked on its own in-session approval, mirrored here
  // because its card renders on a conversation nobody is looking at.
  forestEntry("entry-9b", "session-1", 11, "delegation.blocked", { role: "system", status: "waiting", title: "Implementation · strong needs your approval", text: "Run bun install to add the renderer dependencies?", data: { childBlocked: true, childSessionId: "session-1w", label: "Implementation · strong", objective: "Render Mermaid, math, and sandboxed HTML inline in chat", command: "bun install", cwd: "/tmp/bridge/worker-1w", ownedPaths: ["src/components/**"], orchestratorNotified: true } }, "entry-8b"),
  // A workspace far behind its base branch, with the counts and the choice.
  forestEntry("entry-10b", "session-1", 12, "workspace.stale_base", { role: "system", status: "warning", title: "Workspace is 67 commits behind origin/main", text: "this workspace is 67 commit(s) behind and 1 ahead of origin/main, measured against a freshly fetched ref; that ref's newest commit is 0 day(s) old", data: { staleBase: true, phase: "workspace_open", choices: ["refresh", "continue"], divergence: { baseRef: "origin/main", baseCommit: "90ce51c", head: "2b43aaad9b36", branch: "bridge/task", ahead: 1, behind: 67, refAgeSeconds: 3600, fetchAttempted: true, fetched: true, dirty: false, unavailableReason: null } } }, "entry-9b"),
  // A read, a diff-bearing edit and a command that reports its exit code, so
  // `bun run dev` exercises the inline patch, the hunk fold bar, the "Explored"
  // group label and the exit chip — not only the shapes that predate them.
  forestEntry("entry-11b", "session-1", 13, "tool.completed", { status: "completed", title: "Read tokenStore.ts", data: { type: "readFile", path: "src/auth/tokenStore.ts" } }, "entry-10b"),
  forestEntry("entry-12b", "session-1", 14, "file_change.completed", { status: "completed", title: "tokenStore.ts", data: { path: "src/auth/tokenStore.ts", additions: 9, deletions: 4, durationMs: 400, patch: MOCK_PATCH } }, "entry-11b"),
  forestEntry("entry-13b", "session-1", 15, "command.completed", { status: "completed", title: "bun test src/auth", data: { type: "commandExecution", command: "bun test src/auth", exitCode: 0, durationMs: 2400, aggregatedOutput: "bun test v1.1.34\n\n 42 pass\n 0 fail\nRan 42 tests across 6 files. [2.41s]" } }, "entry-12b"),
  // The harness's own boundary, beside Bridge's `compaction` above. Two
  // different facts on purpose: this one is the provider's context actually
  // shrinking, that one is Bridge saving a summary for a later cold start.
  forestEntry("entry-14b", "session-1", 16, "context.compacted", { status: "completed", title: "Context compacted", data: { harness: "claude", trigger: "auto", preTokens: 184000, postTokens: 22500 } }, "entry-13b"),
  forestEntry("entry-raw", "session-1", 17, "provider.unknown", { method: "provider/debug", raw: { trace: "collapsed" } }, "entry-14b")
];
// Seed memory for the mock host: a spread the Memory surface can actually
// render — pinned + accepted + proposed records, a supersession lineage, a
// conflict group, and a tombstone. `bun run dev` and component tests read this.
function memRecord(
  id: string,
  body: string,
  kind: string,
  status: string,
  provenance: string,
  extra: Partial<MemoryRecord> = {},
): MemoryRecord {
  const at = extra.createdAt ?? "2026-08-24T10:00:00Z";
  return {
    id, scopeKey: "account:local", kind, body, provenance, status,
    validFrom: at, createdAt: at, updatedAt: at, ...extra,
  };
}
const mockMemoryRecords: MemoryRecord[] = [
  memRecord("mem_a1c4", "Styles exclusively with Tailwind v4 utilities — no CSS-in-JS, no inline style a utility can express.", "preference", "active", "user_explicit", { confidenceBps: 9600 }),
  memRecord("mem_a2f7", "Ships every commit as Conventional Commits; never cites issue or PR numbers in the message.", "decision", "active", "user_explicit", { confidenceBps: 8800 }),
  memRecord("mem_a3b0", "Works in IST (UTC+5:30); schedule anything time-bound against that.", "fact", "active", "user_explicit", { confidenceBps: 9200 }),
  memRecord("mem_a4d9", "Graphite & Paper v2 is the locked chrome direction — achromatic, accents reserved for meaning.", "decision", "active", "user_explicit", { confidenceBps: 9600, conflictGroup: "design-direction" }),
  memRecord("mem_b1e2", "Accent color for the app is orange.", "decision", "superseded", "user_explicit", { confidenceBps: 4200, conflictGroup: "design-direction", createdAt: "2026-08-10T09:00:00Z", validTo: "2026-08-21T09:00:00Z" }),
  memRecord("mem_a5aa", "No loud accents in chrome — orange and purple were rejected for the resting surface.", "constraint", "active", "user_explicit", { confidenceBps: 9000, conflictGroup: "design-direction", supersedes: "mem_b1e2", createdAt: "2026-08-21T09:00:00Z" }),
  memRecord("mem_c1f3", "Uses bun (not npm) for this repo's install, dev, test, and build scripts.", "fact", "active", "model_proposal", { confidenceBps: 7400, rationale: "observed across 6 sessions" }),
  memRecord("mem_p1a8", "Read-only workers need an injected GH_TOKEN — the sandbox blocks the keychain.", "fact", "proposed", "model_proposal", { confidenceBps: 7100, rationale: "run 8821 · seen in 3 sessions" }),
  memRecord("mem_p2b5", "Prefers vitest -t filters over whole-file runs while triaging.", "preference", "proposed", "model_proposal", { confidenceBps: 6200, rationale: "run 8813" }),
  memRecord("mem_d1c9", "Sprint-scoped: land the onboarding picker before Friday.", "constraint", "deleted", "model_proposal", { confidenceBps: 3000, createdAt: "2026-08-05T09:00:00Z", validTo: "2026-08-18T09:00:00Z" }),
];

// Deterministic packet-injection audit over the recall-eligible ids — the mock
// stand-in for `memory_retrieval_audits`. Feeds the recall stats.
const RECALL_ELIGIBLE = ["mem_a1c4", "mem_a2f7", "mem_a3b0", "mem_a4d9", "mem_a5aa", "mem_c1f3"];
function buildMockAudit(ids: string[]): PacketInjection[] {
  const audit: PacketInjection[] = [];
  for (let day = 0; day < 14; day++) {
    for (let rep = 0; rep <= day % 3; rep++) {
      const picked = ids.filter((_, i) => ((day + 1) * (i + 2) + rep) % 4 !== 0);
      if (picked.length) audit.push({ day, ids: picked });
    }
  }
  return audit;
}
const mockPacketAudit = buildMockAudit(RECALL_ELIGIBLE);
const mockConsolidationLog: MemoryConsolidationEntry[] = [
  { op: "merge", detail: "2 worktree notes folded into one", day: 12 },
  { op: "correct", detail: "mem_a5aa superseded the orange-accent decision", day: 11 },
  { op: "keep", detail: "design-direction conflict resolved to the pinned winner", day: 9 },
  { op: "expire", detail: "sprint-scoped onboarding note lapsed", day: 6 },
  { op: "group", detail: "3 records tied under conflict-group design-direction", day: 4 },
  { op: "retire", detail: "mem_d1c9 tombstoned", day: 2 },
];
let mockExtractionSettings: MemoryExtractionSettings = { scopeKey: "account:local", mode: "propose" };
let mockMemoryInjection = true;
const mockForests: Record<string, SessionForestSnapshot> = {
  "session-1": {
    sessionId: "session-1", entries: demoEntries, head: { sessionId: "session-1", activeEntryId: "entry-raw", nativeProviderSessionId: "mock-thread-1", restorationMode: "hot", resumeEligibility: "native", latestCheckpointEntryId: "entry-2", updatedAt: now }, leaves: [demoEntries[4], demoEntries[demoEntries.length - 1]],
    workerLeases: [
      { sessionId: "session-1w", workspaceId: "demo-1", role: "implementation", capabilityTier: "strong", taskFamily: "implementation", ownedPaths: ["src/auth/**"], writeMode: "isolated", leaseStatus: "active", expiresAt: null, createdAt: now, updatedAt: now },
      { sessionId: "session-1w2", workspaceId: "demo-1", role: "verification", capabilityTier: "strong", taskFamily: "verification", ownedPaths: ["src/auth/**"], writeMode: "readOnly", leaseStatus: "released", expiresAt: null, createdAt: now, updatedAt: now }
    ],
    workerRuntimes: [
      { sessionId: "session-1w", parentSessionId: "session-1", lifecycleState: "working", taskFamily: "implementation", compatibilityKey: "demo", resultStatus: "pending", retryCount: 0, warmUntil: null, worktreePath: "/tmp/bridge/worker-1w", worktreeBranch: "bridge/worker-1w", lastResult: null, lastActivityAt: now, progressSummary: "editing src/auth/store.rs", updatedAt: now },
      { sessionId: "session-1w2", parentSessionId: "session-1", lifecycleState: "completed", taskFamily: "verification", compatibilityKey: "demo", resultStatus: "reported", retryCount: 0, warmUntil: null, worktreePath: null, worktreeBranch: null, lastResult: { status: "completed", summary: "All 42 auth tests pass and the token store migration is verified. The old serializer is gone from every call site, the migration is idempotent on a second run, and the two fixtures that pinned the previous column order have been rewritten against the new one.", filesChanged: ["src/auth/store.rs", "src/auth/mod.rs", "testing/fixtures/tokens.json"], tests: [{ command: "cargo test auth", status: "passed" }, { command: "bun run test", status: "passed" }] }, lastActivityAt: now, updatedAt: now }
    ],
    workerQueue: [{ id: "queue-1", parentSessionId: "session-1", workspaceId: "demo-1", turnId: "mock-turn-1", request: { role: "implementation", objective: "Update the auth serializer", ownedPaths: ["src/auth/**"], writeMode: "isolated", reason: "owned_path_conflict" }, actualModel: "gpt-5.6-terra", queueStatus: "queued", sequence: 1, attemptCount: 0, dispatchedSessionId: null, expiresAt: now, createdAt: now, updatedAt: now }],
    usage: [
      { id: 1, workspaceId: "demo-1", sessionId: "session-1", turnId: "mock-turn-1", inputTokens: null, outputTokens: null, cacheReadTokens: null, cacheWriteTokens: null, uncachedInputTokens: null, contextPercent: 38, capabilityUnits: 0, runtimeMs: null, costMicrousd: 12_500, costSource: "provider_reported", stablePrefixId: null, stablePrefixHash: null, promptSchemaVersion: null, prefixTokenEstimate: null, harness: "codex", model: null, role: null, taskFamily: null, restorationMode: null, crossHarnessReuse: null, source: "provider.codex", createdAt: now },
      { id: 2, workspaceId: "demo-1", sessionId: "session-1w", turnId: "mock-turn-1", inputTokens: null, outputTokens: null, cacheReadTokens: null, cacheWriteTokens: null, uncachedInputTokens: null, contextPercent: null, capabilityUnits: 8, runtimeMs: null, costMicrousd: null, costSource: null, stablePrefixId: null, stablePrefixHash: null, promptSchemaVersion: null, prefixTokenEstimate: null, harness: null, model: null, role: null, taskFamily: null, restorationMode: null, crossHarnessReuse: null, source: "policy.spawn.strong", createdAt: now }
    ],
    reasons: [
      { id: 106, source: "adapter", kind: "session.shutdown", entityId: "session-old", body: "user_stopped", createdAt: now },
      { id: 105, source: "compaction", kind: "checkpoint.turn_started", entityId: "session-1", body: "phase_boundary", createdAt: now },
      { id: 104, source: "worker-pool", kind: "worker.queued", entityId: "queue-1", body: "owned_path_conflict: src/auth/**", createdAt: now },
      { id: 103, source: "policy", kind: "policy.rejected", entityId: "session-1", body: "turn_budget_exhausted", createdAt: now },
      { id: 102, source: "restoration", kind: "session.restored", entityId: "session-1w2", body: "checkpoint_restored", createdAt: now },
      { id: 101, source: "policy", kind: "worker.spawned", entityId: "session-1w", body: "implementation strong isolated", createdAt: now }
    ],
    policyLimits: { maxWorkersPerTurn: 3, maxStrongWorkersPerTurn: 1, maxCapabilityUnitsPerTurn: 24 },
    repositoryDivergence: { status: "aligned", selectedState: { status: "clean", head: "demo", dirtyHash: "0" }, currentState: { status: "clean", head: "demo", dirtyHash: "0" } },
    completion: {
      attemptId: "proof-demo", contractId: "contract-demo", verdict: "verifying",
      repository: { head: "307729bf075c", dirtyDigest: "clean" }, passedRequired: 2, totalRequired: 4,
      markdownCommitted: false, waiverReason: null,
      checks: [
        { checkId: "rust-tests", kind: "deterministic", required: true, status: "passed", executor: "bridge.shell", command: "cargo test", verifierFamily: null, detail: "216 tests passed", outputDigest: "demo", artifactRefs: [] },
        { checkId: "build", kind: "deterministic", required: true, status: "passed", executor: "bridge.shell", command: "bun run build", verifierFamily: null, detail: null, outputDigest: "demo", artifactRefs: [] },
        { checkId: "scrutiny", kind: "scrutiny", required: true, status: "running", executor: "bridge.worker", command: null, verifierFamily: "claude", detail: null, outputDigest: null, artifactRefs: [] },
        { checkId: "user-journey", kind: "user_testing", required: true, status: "pending", executor: "bridge.worker", command: null, verifierFamily: "codex", detail: null, outputDigest: null, artifactRefs: [] }
      ]
    },
    entryWindow: { returned: demoEntries.length, total: demoEntries.length, trimmedPayloads: 0 }
  }
};
const mockPendingAdoption: WorkerRepositoryBinding = {
  sessionId: "session-1w", parentSessionId: "session-1", workspaceId: "demo-1",
  worktreePath: "/tmp/bridge/worker-1w", worktreeBranch: "bridge/worker-1w",
  taskWorktreePath: "/tmp/bridge/session-supervisor", state: "pending_adoption",
  head: "2b43aaad9b36", baseCommit: "307729bf075c", baseBranch: "bridge/session-supervisor",
  baselineDirtyPaths: [], changedPaths: ["src/components/Markdown.tsx", "src/index.css"],
  diffstat: "2 file(s) changed, 284 insertion(s), 31 deletion(s)", dirty: false,
  detail: null, createdAt: now, updatedAt: now,
};

// Three shapes worth seeing without the desktop app: one collectable, one that
// nothing may touch because its work is unadopted, and one a developer made by
// hand that Bridge reports but never reclaims.
let mockWorktrees: WorktreeInventoryEntry[] = [
  {
    id: "wt-1", kind: "worker", repoRoot: "/tmp/bridge/session-supervisor",
    path: "/tmp/bridge/worker-1w", branch: "bridge/worker-1w",
    ownerSessionId: "session-1w", ownerWorkspaceId: "demo-1", state: "idle",
    disposition: "retained", retainedReason: "a worker's output here has not been adopted or discarded yet",
    assessedAt: now, sizeBytes: 41_943_040, sizeMeasuredAt: now,
    createdAt: now, lastUsedAt: now, idleSeconds: 3_600,
  },
  {
    id: "wt-2", kind: "orchestrator", repoRoot: "/tmp/bridge/demo",
    path: "/tmp/bridge/orchestrators/demo/session-2", branch: "bridge/demo-session2",
    ownerSessionId: "session-2", ownerWorkspaceId: "demo-1", state: "idle",
    disposition: "reclaimable", retainedReason: null, assessedAt: now,
    sizeBytes: 2_684_354_560, sizeMeasuredAt: now,
    createdAt: now, lastUsedAt: now, idleSeconds: 9 * 24 * 3_600,
  },
  {
    id: "wt-3", kind: "orchestrator", repoRoot: "/tmp/bridge/demo",
    path: "/tmp/bridge/demo/.worktrees/hand-made", branch: "chore/hand-made",
    ownerSessionId: null, ownerWorkspaceId: null, state: "external",
    disposition: "retained", retainedReason: "outside Bridge's worktree namespace",
    assessedAt: now, sizeBytes: 33_554_432, sizeMeasuredAt: now,
    createdAt: now, lastUsedAt: now, idleSeconds: 5 * 24 * 3_600,
  },
  {
    id: "wt-4", kind: "worker", repoRoot: "/tmp/bridge/scratch",
    path: "/tmp/bridge/worker-scratch", branch: "bridge/worker-scratch",
    ownerSessionId: "session-scratch", ownerWorkspaceId: "demo-1", state: "idle",
    disposition: "at_risk", retainedReason: "uncommitted changes",
    assessedAt: now, sizeBytes: 20_971_520, sizeMeasuredAt: now,
    createdAt: now, lastUsedAt: now, idleSeconds: 2 * 3_600,
  },
];
// Usage roll-up for the browser host: three harnesses over the last week, with
// one unpriced Codex model so the screen's provenance notes have something to
// say, and one imported source that is only partially covered.
const mockUsagePricing: UsagePricingStatus = { status: "bundled", source: "litellm", snapshotDate: "2026-08-30", fetchedAt: null, knownModels: 259, overrides: 0 };
let mockPriceOverrides: UsagePriceOverride[] = [];
function mockUsageBucket(day: string, harness: string, model: string, scale: number, costSource: UsageBucket["costSource"] = "model_priced"): UsageBucket {
  const uncached = Math.round(18_000 * scale);
  const cacheRead = Math.round(140_000 * scale);
  const cacheWrite = Math.round(9_000 * scale);
  const output = Math.round(6_500 * scale);
  const cost = costSource === "unpriced" ? 0 : Math.round((uncached * 3 + cacheRead * 0.3 + cacheWrite * 3.75 + output * 15) * (harness === "claude" ? 1 : 0.6));
  return { day, hourStart: null, harness, model, records: Math.max(1, Math.round(14 * scale)), sessions: Math.max(1, Math.round(3 * scale)), costSource, costMicrousd: cost, cacheSavingsMicrousd: costSource === "unpriced" ? 0 : Math.round(cacheRead * 2.7 * (harness === "claude" ? 1 : 0.6)), unpricedRecords: costSource === "unpriced" ? Math.max(1, Math.round(14 * scale)) : 0, totals: { uncachedInputTokens: uncached, cacheReadTokens: cacheRead, cacheWriteTokens: cacheWrite, outputTokens: output, reasoningTokens: Math.round(output * 0.4) } };
}
function mockUsageSummary(params: SummaryParams): UsageSummaryResult {
  const buckets: UsageBucket[] = [];
  const since = Date.parse(`${params.sinceDay}T00:00:00Z`);
  const until = Date.parse(`${params.untilDay}T00:00:00Z`);
  let index = 0;
  for (let at = since; at <= until; at += 86_400_000, index += 1) {
    const day = new Date(at).toISOString().slice(0, 10);
    const wave = 0.6 + 0.5 * Math.abs(Math.sin(index * 1.3));
    if (index % 3 !== 2) buckets.push(mockUsageBucket(day, "claude", "claude-fable-5-1", wave));
    buckets.push(mockUsageBucket(day, "codex", "gpt-5.6-luna", wave * 0.7));
    if (index % 4 === 1) buckets.push(mockUsageBucket(day, "codex", "gpt-5.6-experimental", 0.2, "unpriced"));
    if (index % 2 === 0 && params.includeImported) buckets.push(mockUsageBucket(day, "opencode", "kimi-k2.5", wave * 0.3, "provider_reported"));
  }
  const hourly = params.resolution === "hour" && params.sinceTime && params.untilTime;
  if (hourly) {
    const start = Date.parse(params.sinceTime!);
    const end = Date.parse(params.untilTime!);
    buckets.length = 0;
    for (let at = start, hour = 0; at < end; at += 3_600_000, hour += 1) {
      if (hour % 5 === 4) continue;
      const stamp = new Date(at).toISOString().replace(/\.\d{3}Z$/, "Z");
      const day = stamp.slice(0, 10);
      buckets.push({ ...mockUsageBucket(day, "claude", "claude-fable-5-1", 0.08 + 0.05 * Math.abs(Math.sin(hour))), hourStart: stamp });
      if (hour % 2 === 0) buckets.push({ ...mockUsageBucket(day, "codex", "gpt-5.6-luna", 0.05), hourStart: stamp });
    }
  }
  return {
    buckets,
    resolution: params.resolution,
    sinceDay: params.sinceDay,
    untilDay: params.untilDay,
    timeZone: params.timeZone ?? "UTC",
    liveRecords: buckets.filter(bucket => bucket.harness !== "opencode").reduce((sum, bucket) => sum + bucket.records, 0),
    importedRecords: buckets.filter(bucket => bucket.harness === "opencode").reduce((sum, bucket) => sum + bucket.records, 0),
    duplicatesDropped: params.includeImported ? 6 : 0,
    scanDurationMs: 12,
    pricing: { ...mockUsagePricing, overrides: mockPriceOverrides.length },
    sources: params.includeImported ? [
      { id: "claude:~/.claude/projects", agent: "claude", provider: "anthropic", coverageState: "complete", coverageReason: null, lastSuccessfulScanAt: now, recordsImported: 44_740, recordsSkipped: 12 },
      { id: "opencode:opencode.db", agent: "opencode", provider: "opencode", coverageState: "partial", coverageReason: "scan stopped at the record cap; run again to continue", lastSuccessfulScanAt: now, recordsImported: 10_019, recordsSkipped: 0 },
    ] : [],
  };
}
const mockHistorySources: UsageHistorySource[] = [
  { id: "claude:~/.claude/projects", agent: "claude", provider: "anthropic", capability: "supported", location: "~/.claude/projects", detectedVersion: "2.1.261", coverageState: "complete", coverageReason: null, coverageStartAt: "2026-03-02T09:10:00Z", coverageEndAt: now, lastSuccessfulScanAt: now, lastError: null, recordsImported: 44_740, recordsSkipped: 12 },
  { id: "codex:~/.codex/sessions", agent: "codex", provider: "openai", capability: "supported", location: "~/.codex/sessions", detectedVersion: null, coverageState: "stale", coverageReason: "files changed since the last scan", coverageStartAt: "2026-04-11T08:00:00Z", coverageEndAt: "2026-09-01T17:42:00Z", lastSuccessfulScanAt: "2026-09-01T17:42:00Z", lastError: null, recordsImported: 38_350, recordsSkipped: 214 },
  { id: "opencode:opencode.db", agent: "opencode", provider: "opencode", capability: "supported", location: "~/.local/share/opencode/opencode.db", detectedVersion: null, coverageState: "partial", coverageReason: "scan stopped at the record cap; run again to continue", coverageStartAt: "2026-05-20T12:00:00Z", coverageEndAt: now, lastSuccessfulScanAt: now, lastError: null, recordsImported: 10_019, recordsSkipped: 0 },
  { id: "cursor:~/.cursor/chats", agent: "cursor", provider: "cursor", capability: "unsupported", location: "~/.cursor/chats", detectedVersion: null, coverageState: "unsupported", coverageReason: "Cursor's local stores hold no token counts", coverageStartAt: null, coverageEndAt: null, lastSuccessfulScanAt: null, lastError: null, recordsImported: 0, recordsSkipped: 0 },
];
const mockMeterRegistry: MeterRegistry = {
  providers: [
    { id: "codex", label: "Codex", supported: true, plannedSource: null },
    { id: "claude", label: "Claude", supported: true, plannedSource: null },
    { id: "openrouter", label: "OpenRouter", supported: false, plannedSource: "API token credit tracking" },
  ],
  adaptiveDefaultSeconds: 300,
  nominalIntervalSeconds: 300,
  attribution: "Meter math ported from steipete/CodexBar (MIT)",
};
// Browser-mode stand-in for the Insights tab: the shape a real run returns,
// with figures that exercise every chart. Never shown inside Tauri.
let mockInsights: UsageInsightsResult | null = null;
function mockUsageInsights(params: InsightsParams): UsageInsightsResult {
  // Like the daemon: without `refresh` this only reads, and a fresh install
  // has nothing to read.
  if (!params.refresh) return mockInsights ? structuredClone(mockInsights) : { status: "empty", windowDays: params.windowDays };
  const days = Array.from({ length: Math.min(params.windowDays, 30) }, (_, index) => {
    const at = new Date(Date.now() - (Math.min(params.windowDays, 30) - 1 - index) * 86_400_000);
    const wave = 0.5 + 0.5 * Math.abs(Math.sin(index * 1.3));
    return { day: at.toISOString().slice(0, 10), processedTokens: Math.round(2_400_000 * wave), prompts: Math.round(14 * wave) };
  });
  const hours = Array.from({ length: 24 }, (_, hour) => ({ hour, prompts: hour < 8 ? 0 : Math.round(12 * Math.exp(-((hour - 15) ** 2) / 18)) }));
  mockInsights = {
    status: "ready",
    windowDays: params.windowDays,
    generatedAt: new Date().toISOString(),
    harness: "claude",
    model: "sonnet",
    report: {
      headline: "Afternoons on Claude, mornings on Codex",
      summary: "Most of your prompting lands between two and six in the afternoon, and Claude carries two thirds of the tokens. Codex handles the shorter morning asks. Cached input covers most of what Claude reads, which is keeping the estimated cost flat while the token count climbs.",
      highlights: [
        { title: "Cache is doing the work", detail: "Roughly two thirds of Claude's input tokens were read from cache, so longer sessions cost little more than short ones.", tone: "good" },
        { title: "Two PRs waiting on checks", detail: "Two open pull requests have failing checks and neither has moved in the window.", tone: "watch" },
        { title: "Refactors dominate", detail: "Four in ten prompts ask for a refactor or a cleanup rather than a new feature.", tone: "neutral" },
      ],
      themes: [
        { label: "Refactors and cleanup", share: 0.4, example: "Tidy the meter code and remove the footer text" },
        { label: "UI polish", share: 0.25, example: "Make the usage chart read better in dark mode" },
        { label: "Bug fixes", share: 0.2, example: "The tray shows the wrong percentage" },
        { label: "Reviews", share: 0.15, example: "Review these seven pull requests for readiness" },
      ],
      recommendations: [
        "Route review-only asks to Codex: they are short and your Codex weekly window is barely used.",
        "Start long Claude sessions from the same project so cached context keeps carrying over.",
        "Clear the two failing PRs before opening new ones; they are the oldest open work you have.",
      ],
      harnesses: [
        { harness: "claude", processedTokens: 41_200_000, costMicrousd: 38_400_000, records: 412, sessions: 38, prompts: 214 },
        { harness: "codex", processedTokens: 18_900_000, costMicrousd: 12_100_000, records: 260, sessions: 27, prompts: 122 },
        { harness: "opencode", processedTokens: 3_100_000, costMicrousd: 1_400_000, records: 40, sessions: 6, prompts: 18 },
      ],
      hours,
      days,
      github: { repositories: 3, openPrs: 7, draftPrs: 2, failingChecks: 2, awaitingReview: 3 },
      promptsAnalysed: 60,
    },
  };
  return structuredClone(mockInsights);
}
const mockWorktreeUsage: WorktreeUsage = {
  totalCount: 4, totalBytes: 2_780_823_552,
  reclaimableCount: 1, reclaimableBytes: 2_684_354_560, retainedCount: 1,
  maxTotalBytes: 10 * 1024 * 1024 * 1024, maxPerRepo: 12,
  workerIdleTtlSeconds: 86_400, orchestratorIdleTtlSeconds: 604_800, githubIdleTtlSeconds: 604_800,
  repositories: [
    { repoRoot: "/tmp/bridge/demo", count: 1, sizeBytes: 2_684_354_560, reclaimableBytes: 2_684_354_560, overBudget: false },
    { repoRoot: "/tmp/bridge/session-supervisor", count: 1, sizeBytes: 41_943_040, reclaimableBytes: 0, overBudget: false },
    { repoRoot: "/tmp/bridge/scratch", count: 1, sizeBytes: 20_971_520, reclaimableBytes: 0, overBudget: false },
  ],
};

function mockForest(sessionId: string): SessionForestSnapshot {
  const existing = mockForests[sessionId];
  if (existing) return structuredClone(existing);
  const session = mockState.sessions.find(item => item.id === sessionId);
  const entry = forestEntry(`${sessionId}-root`, sessionId, 1, "branch.summary", { summary: "Session started" }, null);
  const created: SessionForestSnapshot = { sessionId, entries: [entry], head: { sessionId, activeEntryId: entry.id, nativeProviderSessionId: session?.providerSessionId ?? null, restorationMode: session?.restorationMode ?? "fresh", resumeEligibility: session?.providerSessionId ? "native" : "fresh", latestCheckpointEntryId: null, updatedAt: now }, leaves: [entry], workerLeases: [], workerRuntimes: [], workerQueue: [], usage: [], reasons: [], policyLimits: { maxWorkersPerTurn: 3, maxStrongWorkersPerTurn: 1,maxCapabilityUnitsPerTurn: 24 }, repositoryDivergence: { status:"unknown", selectedState:null, currentState:{status:"unavailable"} }, completion: null, entryWindow: { returned: 1, total: 1, trimmedPayloads: 0 } };
  mockForests[sessionId] = created;
  return structuredClone(created);
}
function mockContextBreakdown(sessionId: string): ContextBreakdownResult {
  const reason = "no prompt compilation recorded";
  const inventoryReason = "adapter runtime has not reported context inventory";
  return {
    sessionId,
    segments: [
      { origin: "conversation", segmentClass: "conversation", names: [], state: "estimated", method: "bridge-context-projector", itemCount: 2, tokens: 900, capped: false },
      { origin: "promptCompilation", segmentClass: "prompt-stable", names: [], state: "unavailable", reason, capped: false },
      { origin: "promptCompilation", segmentClass: "prompt-variable", names: [], state: "unavailable", reason, capped: false },
      { origin: "adapterInventory", segmentClass: "agentDefinitions", names: [], state: "unavailable", reason: inventoryReason, capped: false },
      { origin: "adapterInventory", segmentClass: "mcpDynamicTools", names: [], state: "unavailable", reason: inventoryReason, capped: false },
      { origin: "adapterInventory", segmentClass: "providerBaseInstructions", names: [], state: "unavailable", reason: inventoryReason, capped: false },
      { origin: "adapterInventory", segmentClass: "skillsPlugins", names: [], state: "unavailable", reason: inventoryReason, capped: false },
      { origin: "adapterInventory", segmentClass: "toolSchemas", names: [], state: "unavailable", reason: inventoryReason, capped: false },
    ],
    totals: { tokens: 900, unavailableSources: 7 },
    conversation: { entryCount: 2, renderedEntryCount: 2, tokenEstimate: 900, contextPressure: 1, contextWindowTokens: 128000 },
    digest: `mock-breakdown-${sessionId}`,
  };
}
function snapshot() { return structuredClone(mockState); }
function emitState() { stateListeners.forEach(listener => listener()); }
function emitMemoryChanged(scopeKey: string) { memoryListeners.forEach(listener => listener({ scopeKey })); }
function appendAgent(sessionId: string, kind: string, fields: Partial<AgentEvent> = {}) {
  const event = agentEvent(nextEventId++, sessionId, kind, fields);
  event.sequence = Math.max(0, ...mockState.agentEvents.filter(item => item.sessionId === sessionId).map(item => item.sequence)) + 1;
  mockState.agentEvents.push(event);
  // Cloned per listener so no subscriber can mutate the stored row or another
  // subscriber's copy — the same isolation a wire round-trip gives.
  agentListeners.forEach(listener => listener(structuredClone(event)));
}

const mockHealth: Health = {
  ok: true, version: "0.1.0-demo", harnesses: { claude: true, codex: true, cursor: true, opencode: true, shell: true }, database: "demo", snapshot_directory: "demo-snapshots", snapshot_count: 3, snapshot_total_bytes: 12_288, telemetry_database: "demo-telemetry", warnings: [],
  adapters: [
    { id: "codex", label: "Codex", available: true, authState: "signed_in", version: "mock", capabilities: ["messages", "streaming", "reasoning", "plans", "tools", "commands", "file_changes", "approvals", "usage", "history", "interrupt"], unavailableReason: null, models: [{ id: "gpt-5.6-luna", label: "GPT Luna", tier: "fast", defaultForTier: true }, { id: "gpt-5.6-terra", label: "GPT Terra", tier: "standard", defaultForTier: true }, { id: "gpt-5.6-sol", label: "GPT Sol", tier: "strong", defaultForTier: true }, { id: "gpt-5.3-codex", label: "GPT-5.3 Codex", tier: "standard", defaultForTier: false }], defaultModel: "gpt-5.6-luna" },
    { id: "claude", label: "Claude Code", available: true, authState: "signed_in", version: "mock", capabilities: ["messages", "streaming", "reasoning", "tools", "commands", "file_changes", "approvals", "usage", "interrupt", "steering"], unavailableReason: null, models: [{ id: "sonnet", label: "Claude Sonnet", tier: "standard", defaultForTier: true }, { id: "opus", label: "Claude Opus", tier: "strong", defaultForTier: false, supportedEffortLevels: ["low", "medium", "high", "xhigh", "max"] }, { id: "haiku", label: "Claude Haiku", tier: "fast", defaultForTier: true }, { id: "fable", label: "Claude Fable", tier: "strong", defaultForTier: true }], defaultModel: "sonnet" },
    { id: "cursor", label: "Cursor", available: true, authState: "signed_in", version: "mock", capabilities: ["messages", "streaming", "reasoning", "plans", "tools", "commands", "file_changes", "approvals", "usage", "history", "interrupt"], unavailableReason: null, models: [{ id: "auto", label: "Auto", tier: "standard", defaultForTier: true }, { id: "composer-2.5", label: "Composer 2.5", tier: "fast", defaultForTier: true }, { id: "gpt-5.3-codex", label: "Codex 5.3", tier: "standard", defaultForTier: false }, { id: "claude-opus-5-thinking-high", label: "Claude Opus 5 1M Thinking", tier: "strong", defaultForTier: true }], defaultModel: "auto" },
    { id: "opencode", label: "OpenCode", available: true, authState: "signed_in", version: "mock", capabilities: ["messages", "streaming", "reasoning", "plans", "tools", "commands", "file_changes", "approvals", "usage", "history", "interrupt"], unavailableReason: null, models: [{ id: "opencode/deepseek-v4-flash-free", label: "DeepSeek V4 Flash", tier: "fast", defaultForTier: true }, { id: "opencode/north-mini-code-free", label: "North Mini Code", tier: "standard", defaultForTier: true }, { id: "opencode/big-pickle", label: "Big Pickle", tier: "strong", defaultForTier: true }], defaultModel: "opencode/north-mini-code-free" }
  ]
};

const mockMarketplace: MarketplaceCatalog = { providers: [
  { provider: "codex", available: true, error: null, variants: [{ provider: "codex", pluginId: "vercel@official", name: "Vercel", description: "Deploy and inspect Vercel projects", marketplace: "official", version: "1.0.0", source: "https://github.com/vercel/mcp", repository: "https://github.com/vercel/mcp", iconDataUrl: null, publisher: "Vercel", capabilities: ["deployments"], mcpEndpoint: null, connectorType: "app", appConnectorIds: ["connector_vercel"], installed: true, enabled: true, authenticationState: "required", sharedAuthMechanism: null, portableMcp: false, compatibilityNotes: [], supportedActions: ["install", "update", "uninstall", "authenticate"], providerMetadata: {} }] },
  { provider: "claude", available: true, error: null, variants: [{ provider: "claude", pluginId: "vercel@official", name: "Vercel", description: "Deploy and inspect Vercel projects", marketplace: "official", version: "1.0.0", source: "https://github.com/vercel/mcp", repository: "https://github.com/vercel/mcp", iconDataUrl: null, publisher: "Vercel", capabilities: ["deployments"], mcpEndpoint: "https://mcp.vercel.com", connectorType: "connector", appConnectorIds: ["plugin:vercel:vercel"], installed: false, enabled: false, authenticationState: "required", sharedAuthMechanism: null, portableMcp: false, compatibilityNotes: [], supportedActions: ["install", "enable", "disable", "update", "uninstall", "authenticate"], providerMetadata: {} }] },
] };

const mockSkills: SkillCatalog = {
  installer: "skills@1.5.19",
  community: [{
    id: "vercel-labs/agent-skills:react-best-practices", slug: "react-best-practices", name: "React Best Practices",
    description: "Review React code for performance and maintainability.", source: "vercel-labs/agent-skills", sourceUrl: "https://github.com/vercel-labs/agent-skills",
    pinnedRef: "8b8c76004956f0e01e4f6c88ff6fb342258461f5", installs: 124000, official: true, compatibility: ["codex", "claude", "opencode"], fileCount: 3,
    permissions: ["Read project files"], risk: "low", riskSummary: "Read-only project guidance.", categories: ["code-review", "react"],
    providerStates: [{ provider: "codex", installed: false, managed: false, installedRef: null, updateAvailable: false, rollbackAvailable: false, receiptError: null }, { provider: "claude", installed: false, managed: false, installedRef: null, updateAvailable: false, rollbackAvailable: false, receiptError: null }, { provider: "opencode", installed: false, managed: false, installedRef: null, updateAvailable: false, rollbackAvailable: false, receiptError: null }],
  }],
  personal: [{ id: "personal:my-workflow", name: "my-workflow", description: "A skill you maintain locally.", providers: ["codex"], source: "Personal skill" }],
};
const mockSkillConsents = new Map<string, { skillId: string; action: SkillAction; targets: SkillProvider[] }>();

const mockAutomations: AutomationCatalog = {
  automations: [
    {
      id: "task-1", provider: "claude", name: "Summarize overnight CI failures", prompt: "Summarize overnight CI failures and file issues for new ones.",
      schedule: { kind: "cron", expression: "7 9 * * 1-5", human: "Weekdays at 09:07" }, status: "active", recurring: true,
      createdAt: Date.now() - 86_400_000, nextRunAt: null, lastRunAt: Date.now() - 3_600_000, cwds: [], model: null, effort: null, runs: [],
    },
    {
      id: "auto-1", provider: "codex", name: "Nightly dependency audit", prompt: "Audit dependencies for CVEs and report anything actionable.",
      schedule: { kind: "rrule", expression: "FREQ=DAILY;BYHOUR=3;BYMINUTE=15", human: "Daily at 03:15" }, status: "paused", recurring: true,
      createdAt: Date.now() - 172_800_000, nextRunAt: Date.now() + 43_200_000, lastRunAt: null, cwds: ["/Users/you/project"], model: "gpt-5.3-codex", effort: "high",
      runs: [{ id: "thread-1", automationId: "auto-1", status: "COMPLETED", title: "Deps clean", summary: "No CVEs found", createdAt: Date.now() - 90_000_000 }],
    },
  ],
  providers: [
    { provider: "claude", available: true, detail: "~/.claude/scheduled_tasks.json", count: 1, capabilities: ["create", "edit", "delete"] },
    { provider: "codex", available: true, detail: "~/.codex/sqlite/codex.db", count: 1, capabilities: ["pause", "resume", "delete"] },
    { provider: "cursor", available: false, detail: "Cursor has no native automations feature", count: 0, capabilities: [] },
    { provider: "opencode", available: false, detail: "OpenCode has no native automations feature", count: 0, capabilities: [] },
  ],
};

function saveMockProfiles(profiles: ModelProfileDraft[]): ModelSetupState {
  const version = (mockModelSetup.activeVersion ?? 0) + 1;
  mockModelSetup = {
    complete: true,
    activeVersion: version,
    profiles: profiles.map(profile => ({
      ...structuredClone(profile),
      schemaVersion: 1,
      version,
      profileId: profile.purpose,
      canonicalRole: profile.purpose === "implementer" ? "implementation"
        : ["verifier", "reviewer", "evaluator"].includes(profile.purpose) ? "verification"
          : profile.purpose === "research" ? "research"
            : profile.purpose === "documentation" ? "documentation" : "planning",
      createdAt: new Date().toISOString(),
    })),
  };
  return structuredClone(mockModelSetup);
}

// Legacy task-action fixtures are separate from the empty browser activity feed.
const workBoardObserved = (secondsAgo: number): string =>
  new Date(Date.now() - secondsAgo * 1000).toISOString();

/// Suggested-work rows the browser fallback can act on, so the task half of the board is
/// developable without the desktop app. Mutable on purpose: an action has to visibly do
/// something or the affordance cannot be exercised.
const mockWorkTasks: WorkTask[] = [
  {
    id: "task-v1:slack-work-1",
    fingerprint: "v1:slack-work-1",
    connectorInstanceId: "slack-work",
    canonicalResourceId: "slack:slack-work:1723459200.123",
    sourceKind: "slack.message",
    title: "Priya is blocked on the migration flag you own",
    why: "Asked twice in two hours in #eng-releases and nobody has replied.",
    rank: 1,
    confidenceBps: 8_600,
    state: "active",
    pinned: false,
    snoozedUntil: null,
    evidenceDigest: "a".repeat(64),
    evidenceTarget: { kind: "externalLink", url: "https://app.slack.com/archives/C1/p1723459200123", host: "app.slack.com" },
    evidenceObservedAt: workBoardObserved(240),
    missCount: 0,
    workspaceId: null,
    createdAt: workBoardObserved(7_200),
    updatedAt: workBoardObserved(240),
  },
  {
    id: "task-v1:github-1",
    fingerprint: "v1:github-1",
    connectorInstanceId: "github-1",
    canonicalResourceId: "github:github-1:PR_418",
    sourceKind: "github.item",
    title: "3 review requests older than two days",
    why: "One is on the release branch, so it is probably holding a deploy.",
    rank: 2,
    confidenceBps: 5_200,
    state: "active",
    pinned: false,
    snoozedUntil: null,
    evidenceDigest: "b".repeat(64),
    evidenceTarget: { kind: "externalLink", url: "https://github.com/o/r/pulls", host: "github.com" },
    evidenceObservedAt: workBoardObserved(240),
    missCount: 0,
    workspaceId: null,
    createdAt: workBoardObserved(10_800),
    updatedAt: workBoardObserved(240),
  },
];

// Work settings for the browser fallback. Starts unconfigured, the fresh-install
// state, and flips to configured when the mock write runs — so the Settings
// surface's whole round-trip is exercisable without the desktop app.
let mockWorkSettings: WorkSettingsSnapshot = {
  configured: false,
  settings: {
    briefing: null,
    enabledConnectorInstances: [],
    refreshOnFocus: false,
    refreshIntervalMinutes: null,
    cooldownMinutes: 15,
    limits: { maxWallSeconds: 600, maxTurns: 12, maxToolCalls: 24, maxOutputTokens: null, costCeilingMicrousd: null },
  },
};

// Suggestion (inline typeahead) settings for the browser fallback. Off by
// default, same as a fresh install's stored default.
let mockSuggestionSettings: SuggestionSettingsSnapshot = {
  configured: false,
  settings: { enabled: false, provider: "claude", model: "haiku" },
};

const mockBriefingOptions: WorkBriefingOptions = {
  harnesses: [
    {
      id: "claude", label: "Claude Code", available: true, supported: true, reason: null,
      defaultModel: "haiku",
      models: [
        { id: "haiku", label: "Claude Haiku", tier: "fast", defaultForBriefing: true },
        { id: "sonnet", label: "Claude Sonnet", tier: "standard", defaultForBriefing: false },
      ],
      connectors: [
        { id: "claude.ai Slack", family: "slack", connected: true },
        { id: "claude.ai GitHub", family: "github", connected: true },
        { id: "claude.ai Gmail", family: "gmail", connected: false },
      ],
    },
    {
      id: "codex", label: "Codex", available: true, supported: false,
      reason: "the app-server protocol has no per-tool authority, so an exact connector read cannot be isolated from a mutation",
      defaultModel: null, models: [], connectors: [],
    },
  ],
};

function browserWorkBoard(): WorkBoard {
  return {
    facts: [],
    tasks: [],
    latestRun: null,
    generatedAt: new Date().toISOString(),
    sources: [],
    settings: structuredClone(mockWorkSettings.settings),
    suggestions: mockWorkSettings.settings.briefing
      ? { state: "ready", detail: null }
      : { state: "not_configured", detail: null },
  };
}

// ── Connector surface mocks ─────────────────────────────────────────────────
// What `bun run dev` and the component tests see. Synthetic on purpose: the
// real surface reads a live account, and a fixture that quoted one would put a
// stranger's message in this repository. Shapes match the wire contract exactly.

function mockConnectorList(): ConnectorListResult {
  return {
    connectors: [
      { family: "slack", displayName: "Slack", server: "claude.ai Slack", harness: "claude", hasInbox: true, available: true, reason: null, explanation: null },
      {
        family: "gmail", displayName: "Gmail", server: "claude.ai Gmail", harness: "claude", hasInbox: false,
        available: false, reason: "noResolver",
        explanation: "Gmail has no in-app inbox yet — its ingress query and card template are not written.",
      },
      {
        family: "linear", displayName: "Linear", server: null, harness: null, hasInbox: false,
        available: false, reason: "notConfigured",
        explanation: "No Linear MCP server is configured in this harness.",
      },
    ],
  };
}

function mockInboxItem(
  key: string,
  overrides: Partial<ConnectorInboxItem> & Pick<ConnectorInboxItem, "channelLabel" | "author" | "text" | "receivedAt">,
): ConnectorInboxItem {
  return {
    itemKey: key,
    family: "slack",
    channelId: key.split(":")[1] ?? "C000",
    kind: "directMessage",
    permalink: null,
    state: "rendered",
    card: null,
    renderRejection: null,
    resolution: null,
    ...overrides,
  } as ConnectorInboxItem;
}

const mockConnectorItems: ConnectorInboxItem[] = [
  mockInboxItem("slack:D09KQ2M4A1X:1757756400.000100", {
    channelLabel: "Nina Alvarez",
    author: "Nina Alvarez",
    kind: "directMessage",
    text: "can you take a look at the release checklist before standup? the updater step is the one I'm unsure about",
    receivedAt: new Date(Date.now() - 4 * 60_000).toISOString(),
    state: "rendered",
    card: {
      itemKey: "slack:D09KQ2M4A1X:1757756400.000100",
      headline: "Nina wants the release checklist reviewed before standup",
      blocks: [
        {
          kind: "message",
          author: "Nina Alvarez",
          text: "can you take a look at the release checklist before standup? the updater step is the one I'm unsure about",
          timestamp: new Date(Date.now() - 4 * 60_000).toISOString(),
        },
        { kind: "summary", text: "She is blocked on the updater step and standup is in 20 minutes." },
        { kind: "fact", label: "Asked", value: "4 minutes ago" },
      ],
      suggestedReplies: [
        "On it — reading the updater step now.",
        "Looking before standup. The updater step changed on Tuesday, I'll flag anything stale.",
      ],
      harnessRendered: true,
    },
  }),
  mockInboxItem("slack:C07R4TQ8ZKD:1757756100.000300", {
    channelLabel: "#eng-alerts",
    author: "Devesh Kumar",
    kind: "mention",
    text: "@atharva the nightly bundle job failed on the notarisation step again — same signature as last week?",
    receivedAt: new Date(Date.now() - 11 * 60_000).toISOString(),
    state: "rendered",
    card: {
      itemKey: "slack:C07R4TQ8ZKD:1757756100.000300",
      headline: "Devesh is asking whether the notarisation failure repeats last week's",
      blocks: [
        {
          kind: "message",
          author: "Devesh Kumar",
          text: "@atharva the nightly bundle job failed on the notarisation step again — same signature as last week?",
          timestamp: new Date(Date.now() - 11 * 60_000).toISOString(),
        },
        { kind: "context", text: "3 earlier messages in #eng-alerts about the nightly bundle." },
        { kind: "fact", label: "Channel", value: "#eng-alerts" },
      ],
      suggestedReplies: ["Checking the signature now.", "Same one — it's the expired notarisation profile."],
      harnessRendered: true,
    },
  }),
  mockInboxItem("slack:C0A469VRHMH:1757755500.000900", {
    channelLabel: "Design sync",
    author: "Rinako Yoshizawa",
    kind: "threadReply",
    text: "the dock pane spacing looks right to me now, shipping it",
    receivedAt: new Date(Date.now() - 21 * 60_000).toISOString(),
    // Deliberately un-carded: this is what a notification looks like while its
    // render run is still in flight, and what it stays as if that run fails.
    state: "pending",
  }),
];

const connectorArrivalListeners = new Set<(payload: ConnectorItemArrivedPayload) => void>();
const connectorCardListeners = new Set<(payload: ConnectorCardReadyPayload) => void>();
const connectorResolvedListeners = new Set<(payload: ConnectorItemResolvedPayload) => void>();
const connectorInboxListeners = new Set<(payload: { family: string }) => void>();

/**
 * Replay one arrival in mock mode so `bun run dev` shows the actual sequence —
 * a toast in Bridge's own wording, then the harness-rendered headline replacing
 * it in place a beat later. Without this, mock mode could only ever show the
 * resting state, and the part of the feature most worth reviewing is the part
 * that happens when nobody asked for it.
 */
let mockArrivalScheduled = false;
function scheduleMockConnectorArrival(): void {
  if (mockArrivalScheduled || typeof window === "undefined") return;
  mockArrivalScheduled = true;
  const item = mockConnectorItems[0];
  window.setTimeout(() => {
    for (const listener of connectorArrivalListeners) {
      listener({
        family: "slack",
        itemKey: item.itemKey,
        headline: `${item.author} sent you a direct message`,
        channelLabel: item.channelLabel,
        author: item.author,
      });
    }
    window.setTimeout(() => {
      for (const listener of connectorCardListeners) {
        listener({
          family: "slack",
          itemKey: item.itemKey,
          headline: item.card?.headline ?? `${item.author} sent you a direct message`,
          harnessRendered: true,
        });
      }
      for (const listener of connectorInboxListeners) listener({ family: "slack" });
    }, 1_400);
  }, 900);
}

let mockIncludeReadMentions = false;

function mockConnectorInbox(): ConnectorInboxResult {
  const items = mockConnectorItems.filter(item => item.state !== "resolved");
  return {
    items,
    unreadCount: items.length,
    includeReadMentions: mockIncludeReadMentions,
    poll: [
      {
        family: "slack",
        lastAttemptAt: new Date(Date.now() - 20_000).toISOString(),
        lastSuccessAt: new Date(Date.now() - 20_000).toISOString(),
        degraded: null,
      },
    ],
  };
}

function mockConnectorAct(itemKey: string, action: ConnectorActionRequest, approved?: boolean): ConnectorActResult {
  const item = mockConnectorItems.find(candidate => candidate.itemKey === itemKey);
  if (!item) return { status: "refused", reason: "that message is no longer in the inbox" };
  if (item.state === "resolved") return { status: "refused", reason: "this message has already been dealt with" };
  // The same two-call shape as the host: an undecided call is refused and hands
  // back the effect, so the mock exercises the real approval flow rather than
  // letting the UI shortcut it.
  if (approved === undefined) {
    const destination = item.kind === "directMessage" || item.channelLabel === item.author
      ? item.author
      : `${item.author} in ${item.channelLabel}`;
    const effect = action.kind === "reply"
      ? `Send to ${destination}:\n${action.text}`
      : `React :${action.emoji}: to ${destination}'s message`;
    return { status: "approvalRequired", effect };
  }
  if (!approved) return { status: "refused", reason: "the action was denied" };
  item.state = "resolved";
  item.resolution = action.kind === "reply" ? "replied" : "reacted";
  return { status: "sent", itemKey };
}

function mockConnectorDismiss(itemKey: string): ConnectorDismissResult {
  const item = mockConnectorItems.find(candidate => candidate.itemKey === itemKey);
  if (!item || item.state === "resolved") return { dismissed: false };
  item.state = "resolved";
  item.resolution = "dismissed";
  return { dismissed: true };
}

/// The default reviewer instructions the mock reports; the real text lives in
/// `bridge_core::reviewer_settings` and reaches the UI through the result.
const MOCK_REVIEWER_PROMPT = "Review pull request #{number} in this repository and post a concise, constructive review as a comment. Do not approve, merge, request changes, or close the PR.";

export const bridgeApi = {
  discoverExternalImport: (params: DiscoverExternalImportParams): Promise<ExternalImportDiscovery> => {
    if (isTauri()) return call("imports/discover_external_import", params);
    const discoveredAt = new Date().toISOString();
    const root = params.selectedExport ?? params.approvedRoots[0] ?? "/mock/.claude";
    return Promise.resolve({
      discoveryId: "mock-claude-discovery",
      provider: params.provider,
      approvedRoots: params.approvedRoots,
      sourceVersion: params.sourceVersion ?? null,
      formatVersions: params.formatVersions,
      discoveredAt,
      diagnostics: [],
      artifacts: [
        { artifactId: "mock-instructions", canonicalSourceRef: `${root}/CLAUDE.md`, sourceLabel: "CLAUDE.md", kind: "instruction", classification: "documented", stability: "stable", estimatedBytes: 820, modifiedAt: discoveredAt, requiredSchemaGate: null },
        { artifactId: "mock-memory", canonicalSourceRef: `${root}/projects/demo/memory/MEMORY.md`, sourceLabel: "projects/demo/memory/MEMORY.md", kind: "memory", classification: "version_gated_private", stability: "version_gated", estimatedBytes: 430, modifiedAt: discoveredAt, requiredSchemaGate: "claude-auto-memory-v1" },
        { artifactId: "mock-history", canonicalSourceRef: `${root}/projects/demo/session.jsonl`, sourceLabel: "projects/demo/session.jsonl", kind: "conversation", classification: "version_gated_private", stability: "version_gated", estimatedBytes: 2_400, modifiedAt: discoveredAt, requiredSchemaGate: "claude-jsonl-v1" },
      ],
    });
  },
  previewExternalImport: (discovery: ExternalImportDiscovery, artifactIds: string[]): Promise<ExternalImportPreview> => {
    if (isTauri()) return call("imports/preview_external_import", { discoveryId: discovery.discoveryId, artifactIds } satisfies PreviewExternalImportParams);
    const selected = new Set(artifactIds);
    const source = (artifactId: string) => {
      const artifact = discovery.artifacts.find(item => item.artifactId === artifactId)!;
      return {
        provider: "claude_code",
        adapterVersion: "1",
        sourceVersion: discovery.sourceVersion ?? null,
        schemaVersion: artifact.requiredSchemaGate,
        canonicalSourceRef: artifact.canonicalSourceRef,
        sourcePathFingerprint: `mock-${artifactId}`,
        discoveredAt: discovery.discoveredAt,
        sourceMetadata: { classification: artifact.classification },
      };
    };
    const candidate = (artifactId: string, candidate: Partial<ExternalImportCandidate>): ExternalImportCandidate => ({
      candidateId: `candidate-${artifactId}`,
      source: source(artifactId),
      sourceNativeId: null,
      kind: discovery.artifacts.find(item => item.artifactId === artifactId)!.kind,
      title: discovery.artifacts.find(item => item.artifactId === artifactId)!.sourceLabel,
      createdAt: null,
      updatedAt: null,
      projectHint: null,
      contentHash: `sha256:mock-${artifactId}`,
      stability: discovery.artifacts.find(item => item.artifactId === artifactId)!.stability,
      confidenceBps: 9_000,
      selectedByDefault: false,
      redactionSummary: { structuredFieldsExcluded: 0, textValuesRedacted: 0, categories: [], safelyRepresentable: true },
      diagnostics: [],
      normalizedPayload: { text: "Local browser preview" },
      ...candidate,
    });
    const candidates: ExternalImportCandidate[] = [];
    if (selected.has("mock-instructions")) candidates.push(candidate("mock-instructions", { kind: "instruction" }));
    if (selected.has("mock-memory")) candidates.push(candidate("mock-memory", { kind: "memory", title: "Claude auto memory" }));
    if (selected.has("mock-history")) candidates.push(candidate("mock-history", { kind: "conversation", title: "Historical Claude session", normalizedPayload: { messages: [] } }));
    return Promise.resolve({ candidates });
  },
  commitExternalImport: (discoveryId: string, candidates: ExternalImportCandidate[], plan: ExternalImportPlan): Promise<ExternalImportCommit> => {
    if (isTauri()) return call("imports/commit_external_import", { discoveryId, plan } satisfies CommitExternalImportParams);
    const selected = candidates.filter(candidate => plan.selectedCandidateIds.includes(candidate.candidateId));
    return Promise.resolve({
      importId: crypto.randomUUID(),
      candidateResults: selected.map(candidate => ({ candidateId: candidate.candidateId, status: plan.dryRun ? "dry_run" : "imported", createdBridgeIds: plan.dryRun ? [] : [crypto.randomUUID()], revisionOf: null, diagnostics: [] })),
      createdBridgeIds: [],
      imported: plan.dryRun ? 0 : selected.length,
      skipped: 0,
      changed: 0,
      conflicted: 0,
      rejected: 0,
      unsupported: 0,
      rollbackState: plan.dryRun ? "dry_run" : "committed",
      diagnostics: [],
      createdAt: new Date().toISOString(),
    });
  },
  connectorList: (refresh = false): Promise<ConnectorListResult> =>
    isTauri() ? call("connectors/connector_list", { refresh }) : Promise.resolve(mockConnectorList()),
  connectorInbox: (limit?: number): Promise<ConnectorInboxResult> =>
    isTauri() ? call("connectors/connector_inbox", { limit: limit ?? null }) : Promise.resolve(mockConnectorInbox()),
  connectorAct: (itemKey: string, action: ConnectorActionRequest, approved?: boolean): Promise<ConnectorActResult> =>
    isTauri()
      ? call("connectors/connector_act", { itemKey, action, approved: approved ?? null })
      : Promise.resolve(mockConnectorAct(itemKey, action, approved)),
  connectorDismiss: (itemKey: string): Promise<ConnectorDismissResult> =>
    isTauri() ? call("connectors/connector_dismiss", { itemKey }) : Promise.resolve(mockConnectorDismiss(itemKey)),
  connectorRefresh: (family: string): Promise<ConnectorRefreshResult> =>
    isTauri() ? call("connectors/connector_refresh", { family }) : Promise.resolve({ announced: 0 }),
  connectorSetSettings: (includeReadMentions: boolean): Promise<ConnectorSetSettingsResult> =>
    isTauri()
      ? call("connectors/connector_set_settings", { includeReadMentions })
      : Promise.resolve(((mockIncludeReadMentions = includeReadMentions), { includeReadMentions })),
  githubStatus: (workspaceId: string, refresh = false): Promise<GithubStatusResult> =>
    isTauri() ? call("github/github_status", { workspaceId, refresh }) : Promise.resolve(mockGithubStatus(workspaceId)),
  githubPullRequests: (workspaceId: string): Promise<GithubPullRequestsResult> =>
    isTauri() ? call("github/github_prs", { workspaceId }) : Promise.resolve(mockGithubPullRequests(workspaceId)),
  githubPullRequest: (workspaceId: string, number: number): Promise<GithubPullRequestResult> =>
    isTauri() ? call("github/github_pr", { workspaceId, number }) : Promise.resolve(mockGithubPullRequest(workspaceId, number)),
  githubChecks: (workspaceId: string, number: number): Promise<GithubChecksResult> =>
    isTauri() ? call("github/github_checks", { workspaceId, number }) : Promise.resolve(mockGithubChecks(workspaceId, number)),
  githubIssues: (workspaceId: string): Promise<GithubIssuesResult> =>
    isTauri() ? call("github/github_issues", { workspaceId }) : Promise.resolve(mockGithubIssues()),
  githubIssue: (workspaceId: string, number: number): Promise<GithubIssueResult> =>
    isTauri() ? call("github/github_issue", { workspaceId, number }) : Promise.resolve(mockGithubIssue(number)),
  githubRepository: (workspaceId: string): Promise<GithubRepositoryResult> =>
    isTauri() ? call("github/github_repository", { workspaceId }) : Promise.resolve(mockGithubRepository()),
  githubMergeConfig: (workspaceId: string): Promise<GithubMergeConfigResult> =>
    isTauri() ? call("github/github_merge_config", { workspaceId }) : Promise.resolve(mockGithubMergeConfig()),
  githubAct: (workspaceId: string, action: GithubAction, confirmed: boolean): Promise<GithubActResult> =>
    isTauri() ? call("github/github_act", { workspaceId, action, confirmed }) : Promise.resolve(mockGithubAct(action, confirmed)),
  githubReview: (workspaceId: string, number: number, harness: string, sessionId?: string): Promise<GithubReviewResult> =>
    isTauri() ? call("github/github_review", { workspaceId, number, harness, sessionId }) : Promise.resolve(mockGithubReview(number, harness)),
  githubCheckout: (workspaceId: string, number: number): Promise<GithubCheckoutResult> =>
    isTauri() ? call("github/github_checkout", { workspaceId, number }) : Promise.resolve(mockGithubCheckout(workspaceId, number)),
  githubConnect: (workspaceId: string, remoteUrl: string): Promise<GithubConnectResult> =>
    isTauri() ? call("github/github_connect", { workspaceId, remoteUrl }) : Promise.resolve(mockGithubConnect(remoteUrl)),
  searchGithubRepos: (query: string): Promise<SearchGithubReposResult> =>
    isTauri() ? call("workspaces/search_github_repos", { query }) : Promise.resolve(mockSearchGithubRepos(query)),
  browserBridgeState: (): Promise<BrowserBridgeSnapshot> => isTauri() ? call("browser/browser_bridge_state") as Promise<BrowserBridgeSnapshot> : Promise.resolve(structuredClone(mockBrowserBridge)),
  browserFrame: (afterRevision: number): Promise<BrowserFrame | null> => isTauri()
    ? call("browser/browser_frame", { afterRevision })
    : Promise.resolve(mockBrowserBridge.lease && mockBrowserBridge.screenshot && afterRevision < 1 ? {
      revision: 1, leaseId: mockBrowserBridge.lease.id, dataUrl: mockBrowserBridge.screenshot,
      redactedRegions: mockBrowserBridge.screenshotRedactedRegions,
    } : null),
  installBrowserNativeHost: async (): Promise<string> => {
    if (isTauri()) return call("browser/install_browser_native_host");
    mockBrowserBridge.nativeHostInstalled = true; mockBrowserBridge.nativeHostManifestPath = "/mock/dev.bridge.deck.browser.json";
    return mockBrowserBridge.nativeHostManifestPath;
  },
  browserAction: async (request: BrowserActionRequest): Promise<string> => {
    if (isTauri()) return call("browser/browser_action", { request });
    if (request.kind === "list_tabs") mockBrowserBridge.tabs = [{ id: 1, title: "Bridge test tab", url: "https://example.com", domain: "example.com", favIconUrl: null, attached: false }];
    if (request.kind === "attach" && request.tabId) {
      mockBrowserBridge.transportConnected = true; mockBrowserBridge.tabs = mockBrowserBridge.tabs.map(tab => ({ ...tab, attached: tab.id === request.tabId }));
      mockBrowserBridge.lease = { id: crypto.randomUUID(), tabId: request.tabId, domain: "example.com", status: "active", permission: "read_only", attachedAt: new Date().toISOString(), expiresAt: new Date(Date.now() + 1_800_000).toISOString(), lastActivityAt: new Date().toISOString() };
      mockBrowserBridge.status = "reading";
    }
    return crypto.randomUUID();
  },
  setBrowserPermission: async (permission: "read_only" | "interact"): Promise<void> => {
    if (isTauri()) return unit(call("browser/set_browser_permission", { permission }));
    if (mockBrowserBridge.lease) mockBrowserBridge.lease.permission = permission;
  },
  resolveBrowserApproval: async (approvalId: string, allow: boolean): Promise<void> => {
    if (isTauri()) return unit(call("browser/resolve_browser_approval", { approvalId, allow }));
    mockBrowserBridge.pendingApproval = null; mockBrowserBridge.status = allow ? "acting" : "paused";
  },
  takeoverBrowser: async (): Promise<void> => { if (isTauri()) return unit(call("browser/takeover_browser")); mockBrowserBridge.status = "paused"; },
  detachBrowser: async (): Promise<string> => { if (isTauri()) return call("browser/detach_browser"); mockBrowserBridge.lease = null; mockBrowserBridge.status = "not_attached"; return crypto.randomUUID(); },
  routeBrowser: (request: BrowserRouteRequest): Promise<BrowserRouteDecision> => {
    if (isTauri()) return call("browser/route_browser", { request });
    const route: BrowserRouteDecision["route"] = request.structuredApiAvailable ? "mcp_api" : request.needsGeoOrProxy || request.unattended || request.needsParallelism && request.remoteProviderConfigured ? "remote_browser" : request.needsUserAuth ? "attached_tab" : request.needsIsolation || request.needsParallelism ? "local_headless" : request.domControlAvailable ? "attached_tab" : "computer_use";
    return Promise.resolve({ route, reason: "Mock routing decision", requiresUserGrant: route === "attached_tab" || route === "computer_use" });
  },
  browserSkills: (): Promise<BrowserSkill[]> => isTauri() ? call("browser/browser_skills") : Promise.resolve([]),
  configureRemoteBrowser: async (config: RemoteBrowserConfig | null): Promise<void> => { if (isTauri()) return unit(call("browser/configure_remote_browser", { config })); mockBrowserBridge.remoteProvider = config; },
  startRemoteBrowser: (initialUrl: string): Promise<Record<string, unknown>> => isTauri() ? call("browser/start_remote_browser", { initialUrl }) as Promise<Record<string, unknown>> : Promise.resolve({ id: "mock-remote", initialUrl }),
  skillCatalog: (): Promise<SkillCatalog> => isTauri() ? call("skills/skill_catalog") as Promise<SkillCatalog> : Promise.resolve(structuredClone(mockSkills)),
  skillSuggestions: (query: string, provider: SkillProvider): Promise<CapabilitySuggestion[]> => isTauri() ? call("skills/skill_suggestions", { query, provider }) as Promise<CapabilitySuggestion[]> : Promise.resolve(mockSkills.community.filter(skill => skill.providerStates.some(state => state.provider === provider && state.installed) && `${skill.name} ${skill.description} ${skill.categories.join(" ")}`.toLowerCase().includes(query.toLowerCase())).map(skill => ({ id: skill.id, name: skill.name, command: skill.slug, relevance: `Matches “${query}”`, source: skill.source, providers: [provider], permissions: skill.permissions, risk: skill.risk, installed: true }))),
  previewSkillChange: async (skillId: string, action: SkillAction, targets: SkillProvider[]): Promise<SkillPreview> => {
    if (isTauri()) return call("skills/preview_skill_change", { skillId, action, targets }) as Promise<SkillPreview>;
    const skill = mockSkills.community.find(item => item.id === skillId); if (!skill) throw new Error("Skill not found");
    const confirmationId = crypto.randomUUID(); mockSkillConsents.set(confirmationId, { skillId, action, targets });
    return { confirmationId, expiresAt: new Date(Date.now() + 300_000).toISOString(), action, skill: structuredClone(skill), targets, changes: targets.map(provider => `${action} ${skill.name} for ${provider}`), installer: mockSkills.installer };
  },
  executeSkillChange: async (confirmationId: string): Promise<SkillActionResult[]> => {
    if (isTauri()) return call("skills/execute_skill_change", { confirmationId }) as Promise<SkillActionResult[]>;
    const consent = mockSkillConsents.get(confirmationId); if (!consent) throw new Error("Confirmation is invalid or already used"); mockSkillConsents.delete(confirmationId);
    const skill = mockSkills.community.find(item => item.id === consent.skillId)!;
    for (const target of consent.targets) { const state = skill.providerStates.find(item => item.provider === target)!; state.installed = consent.action === "install"; state.managed = consent.action === "install"; state.installedRef = consent.action === "install" ? skill.pinnedRef : null; }
    return consent.targets.map(provider => ({ provider, action: consent.action, success: true, message: `${consent.action} completed`, error: null }));
  },
  automationCatalog: (): Promise<AutomationCatalog> => isTauri() ? call("automations/automation_catalog") as Promise<AutomationCatalog> : Promise.resolve(structuredClone(mockAutomations)),
  saveAutomation: async (draft: SaveAutomationParams): Promise<AutomationSaveResult> => {
    if (isTauri()) return call("automations/save_automation", draft) as Promise<AutomationSaveResult>;
    if (draft.provider !== "claude") throw new Error(`${draft.provider} does not expose native automation saving`);
    const existing = draft.id ? mockAutomations.automations.find(item => item.provider === draft.provider && item.id === draft.id) : undefined;
    if (draft.id && !existing) throw new Error(`No Claude Code scheduled task with id ${draft.id}`);
    const id = existing?.id ?? crypto.randomUUID();
    if (existing) {
      existing.name = draft.prompt.split(/[.;:\n]/)[0].trim(); existing.prompt = draft.prompt;
      existing.schedule = { kind: "cron", expression: draft.scheduleExpression, human: draft.scheduleExpression };
      existing.recurring = draft.recurring;
    } else {
      mockAutomations.automations.unshift({
        id, provider: "claude", name: draft.prompt.split(/[.;:\n]/)[0].trim(), prompt: draft.prompt,
        schedule: { kind: "cron", expression: draft.scheduleExpression, human: draft.scheduleExpression }, status: "active", recurring: draft.recurring,
        createdAt: Date.now(), nextRunAt: null, lastRunAt: null, cwds: [], model: null, effort: null, runs: [],
      });
    }
    return { provider: "claude", id, created: !existing, message: existing ? "Updated in Claude Code's schedule file" : "Created in Claude Code's schedule file" };
  },
  executeAutomationAction: async (provider: AutomationProvider, id: string, action: AutomationAction): Promise<AutomationActionResult> => {
    if (isTauri()) return call("automations/execute_automation_action", { provider, id, action }) as Promise<AutomationActionResult>;
    const state = mockAutomations.providers.find(item => item.provider === provider);
    if (!state?.capabilities.includes(action)) throw new Error(`${provider} does not expose native automation ${action} support`);
    const automation = mockAutomations.automations.find(item => item.provider === provider && item.id === id);
    if (!automation) throw new Error(`No ${provider} automation with id ${id}`);
    if (action === "delete") mockAutomations.automations = mockAutomations.automations.filter(item => item !== automation);
    else automation.status = action === "pause" ? "paused" : "active";
    return { provider, id, action, success: true, message: `${action} completed` };
  },
  marketplaceCatalog: (): Promise<MarketplaceCatalog> => isTauri() ? call("marketplace/marketplace_catalog") as Promise<MarketplaceCatalog> : Promise.resolve(structuredClone(mockMarketplace)),
  marketplaceAppAuthStates: (): Promise<MarketplaceAppAuthState[]> => {
    if (isTauri()) return call("marketplace/marketplace_app_auth_states") as Promise<MarketplaceAppAuthState[]>;
    const variant = mockMarketplace.providers.find(item => item.provider === "codex")?.variants.find(item => item.appConnectorIds.includes("connector_vercel"));
    const authenticationState = variant?.authenticationState === "connected" ? "connected" : "required";
    return Promise.resolve([
      { provider: "codex", connectorId: "connector_vercel", displayName: null, nativeConnector: false, authenticationState },
      { provider: "claude", connectorId: "plugin:vercel:vercel", displayName: null, nativeConnector: false, authenticationState: "required" },
      { provider: "claude", connectorId: "claude.ai Notion", displayName: "Notion", nativeConnector: true, authenticationState: "connected" },
    ]);
  },
  marketplaceAction: async (provider: MarketplaceProvider, pluginId: string, marketplace: string | null, action: MarketplaceAction): Promise<MarketplaceActionResult> => {
    if (isTauri()) return call("marketplace/marketplace_action", { provider, pluginId, marketplace, action }) as Promise<MarketplaceActionResult>;
    const entry = mockMarketplace.providers.find(item => item.provider === provider)?.variants.find(item => item.pluginId === pluginId);
    if (!entry) throw new Error(`${provider} plugin not found`);
    if (action === "install") entry.installed = true;
    if (action === "enable") entry.enabled = true;
    if (action === "disable") entry.enabled = false;
    if (action === "uninstall") { entry.installed = false; entry.enabled = false; entry.authenticationState = "unknown"; }
    if (action === "authenticate") entry.authenticationState = "connected";
    return { provider, pluginId, action, success: true, message: `${action} completed`, error: null };
  },
  // ── agents: the managed runtime lifecycle ─────────────────────────────────
  //
  // Thin pass-throughs. Every question the UI asks — which copy would launch, is
  // it Bridge's to remove — is answered by a field in these responses, so no
  // ownership logic is reimplemented here.
  listManagedAgents: (): Promise<ManagedAgentList> =>
    isTauri() ? call("agents/list_managed_agents") : Promise.resolve(structuredClone(mockManagedAgents)),
  inspectManagedAgent: (agentId: string): Promise<ManagedAgentInspection> => {
    if (isTauri()) return call("agents/inspect_managed_agent", { agentId });
    const status = mockManagedAgents.agents.find(agent => agent.agentId === agentId);
    // Reject rather than substituting another agent: silently answering about the
    // wrong runtime is the kind of mock that hides a real bug.
    if (!status) return Promise.reject(new Error(`${agentId} is not a built-in agent`));
    return Promise.resolve({ status: structuredClone(status), receipt: null, externalRuntime: null });
  },
  installManagedAgent: (agentId: string): Promise<ManagedAgentOperationResult> =>
    isTauri() ? call("agents/install_managed_agent", { agentId }) : mockManagedOperation(agentId, "install"),
  repairManagedAgent: (agentId: string): Promise<ManagedAgentOperationResult> =>
    isTauri() ? call("agents/repair_managed_agent", { agentId }) : mockManagedOperation(agentId, "repair"),
  uninstallManagedAgent: (agentId: string): Promise<ManagedAgentOperationResult> =>
    isTauri() ? call("agents/uninstall_managed_agent", { agentId }) : mockManagedOperation(agentId, "uninstall"),

  health: (): Promise<Health> => isTauri() ? call("health/health") : Promise.resolve(structuredClone(mockHealth)),
  refreshModelCatalogs: (): Promise<Health> => isTauri() ? call("health/refresh_model_catalogs") : Promise.resolve(structuredClone(mockHealth)),
  state: (): Promise<BridgeState> => isTauri() ? call("state/get_state") : Promise.resolve(snapshot()),
  modelSetup: (): Promise<ModelSetupState> => isTauri() ? call("models/get_model_setup") as Promise<ModelSetupState> : Promise.resolve(structuredClone(mockModelSetup)),
  recommendedModelProfiles: (): Promise<ModelProfileDraft[]> => isTauri() ? call("models/recommended_model_profiles") : Promise.resolve(recommendedProfileDrafts(mockHealth.adapters)),
  saveModelProfiles: (profiles: ModelProfileDraft[]): Promise<ModelSetupState> => isTauri() ? call("models/save_model_profiles", { profiles }) as Promise<ModelSetupState> : Promise.resolve(saveMockProfiles(profiles)),
  resetModelProfiles: (): Promise<ModelSetupState> => isTauri() ? call("models/reset_model_profiles") as Promise<ModelSetupState> : Promise.resolve(saveMockProfiles(recommendedProfileDrafts(mockHealth.adapters))),
  // Token and cost usage. One summary per window; the screen never polls.
  usageSummary: (params: SummaryParams): Promise<UsageSummaryResult> =>
    isTauri() ? call("usage/summary", params) : Promise.resolve(mockUsageSummary(params)),
  // The Insights tab. Blocking while a harness turn runs; the stored report
  // comes back at once when `refresh` is false.
  usageInsights: (params: InsightsParams): Promise<UsageInsightsResult> =>
    isTauri() ? call("usage/insights", params) : new Promise(resolve => setTimeout(() => resolve(mockUsageInsights(params)), params.refresh ? 900 : 0)),
  listUsagePriceOverrides: (): Promise<UsagePriceOverride[]> =>
    isTauri() ? call("usage/list_price_overrides") : Promise.resolve(structuredClone(mockPriceOverrides)),
  setUsagePriceOverride: (params: SetPriceOverrideParams): Promise<UsagePriceOverride[]> => {
    if (isTauri()) return call("usage/set_price_override", params);
    const override: UsagePriceOverride = { model: params.model, inputMicrousdPerMtok: params.inputMicrousdPerMtok, outputMicrousdPerMtok: params.outputMicrousdPerMtok, cacheReadMicrousdPerMtok: params.cacheReadMicrousdPerMtok ?? null, cacheWriteMicrousdPerMtok: params.cacheWriteMicrousdPerMtok ?? null, updatedAt: new Date().toISOString() };
    mockPriceOverrides = [...mockPriceOverrides.filter(item => item.model !== params.model), override].sort((a, b) => a.model.localeCompare(b.model));
    return Promise.resolve(structuredClone(mockPriceOverrides));
  },
  clearUsagePriceOverride: (model: string): Promise<UsagePriceOverride[]> => {
    if (isTauri()) return call("usage/clear_price_override", { model });
    mockPriceOverrides = mockPriceOverrides.filter(item => item.model !== model);
    return Promise.resolve(structuredClone(mockPriceOverrides));
  },
  refreshUsageRates: (): Promise<UsagePricingStatus> =>
    isTauri() ? call("usage/refresh_rates") : Promise.resolve({ ...mockUsagePricing, status: "refreshed", fetchedAt: new Date().toISOString(), overrides: mockPriceOverrides.length }),
  listUsageHistorySources: (): Promise<UsageHistorySource[]> =>
    isTauri() ? call("usage/list_history_sources") : Promise.resolve(structuredClone(mockHistorySources)),
  scanUsageHistory: (params: ScanHistoryParams = {}): Promise<ScanHistoryResult> => {
    if (isTauri()) return call("usage/scan_history", params);
    const scanned = mockHistorySources.filter(source => !params.sourceIds || params.sourceIds.includes(source.id));
    for (const source of scanned) {
      if (source.capability !== "supported") continue;
      source.coverageState = "complete";
      source.coverageReason = null;
      source.lastSuccessfulScanAt = new Date().toISOString();
    }
    return Promise.resolve({
      durationMs: 840,
      recordsImported: scanned.some(source => source.agent === "codex") ? 122 : 0,
      recordsSkipped: 0,
      sources: scanned.map(source => ({ sourceId: source.id, agent: source.agent, provider: source.provider, capability: source.capability, location: source.location, coverage: source.coverageState, recordsImported: source.agent === "codex" ? 122 : 0, recordsSkipped: 0, nextCursor: null, warning: source.capability === "unsupported" ? source.coverageReason : null })),
    });
  },
  // ── meter: the CodexBar-style menu-bar companion ──────────────────────────
  //
  // Live windows ride the existing account-usage channel; these two calls only
  // fetch the static provider registry and trigger a refresh. Pace math lives
  // in `src/meter.ts`, ported from the same CodexBar sources as the Rust core.
  getMeterSnapshot: (): Promise<MeterRegistry> =>
    isTauri() ? call("meter/get_meter_snapshot") : Promise.resolve(structuredClone(mockMeterRegistry)),
  // Raising the main window is the shell's job, not the webview's. Going
  // through `@tauri-apps/api/window` made this an ACL-gated IPC call that the
  // capability file never granted, so every caller got `window.show not
  // allowed` at the click. It also could not work from the meter panel, whose
  // `getCurrentWindow()` is the panel, not `main`. The shell owns the handle
  // and shows it natively, which needs no permission and targets the right
  // window — same channel pattern as `notifyLayoutFullscreen`.
  revealMainWindow: async (): Promise<void> => {
    if (!isTauri()) return;
    const { emit } = await import("@tauri-apps/api/event");
    await emit("bridge-reveal-main");
  },
  refreshMeter: (): Promise<void> => {
    if (isTauri()) return call("meter/refresh_meter").then(() => undefined);
    return Promise.resolve();
  },
  getMenuBarSettings: (): Promise<MenuBarSettings> => isTauri()
    ? call("menu_bar/get_menu_bar_settings") : Promise.resolve(structuredClone(mockMenuBarSettings)),
  saveMenuBarSettings: async (settings: MenuBarSettings): Promise<MenuBarSettings> => {
    if (!isTauri()) { mockMenuBarSettings = structuredClone(settings); return structuredClone(settings); }
    const saved = await call("menu_bar/save_menu_bar_settings", { settings });
    const { emit } = await import("@tauri-apps/api/event");
    // Persistence already succeeded. A missed presentation hint must not
    // misreport the save; the native host also re-reads preferences on its tick.
    await emit("bridge-menu-bar-settings-changed").catch(() => undefined);
    return saved;
  },
  getProviderUsageOverviews: (): Promise<ProviderUsageOverviews | null> => isTauri()
    ? call("usage/get_provider_usage_overviews") : Promise.resolve(mockProviderUsageOverviews()),
  refreshProviderUsageOverviews: async (): Promise<ProviderUsageOverviews | null> => {
    if (!isTauri()) return null;
    const snapshot = await call("usage/refresh_provider_usage_overviews_interactive");
    const { emit } = await import("@tauri-apps/api/event");
    await emit("bridge-provider-usage-overviews", snapshot).catch(() => undefined);
    await emit("bridge-menu-bar-settings-changed").catch(() => undefined);
    return snapshot;
  },
  onProviderUsageOverviews: (handler: (snapshot: ProviderUsageOverviews) => void): Promise<UnlistenFn> => isTauri()
    ? listen<ProviderUsageOverviews>("bridge-provider-usage-overviews", event => handler(event.payload)) : Promise.resolve(() => undefined),
  connectMenuBarOpenCode: async (): Promise<void> => {
    if (!isTauri()) throw new Error("Open Bridge desktop to connect OpenCode.");
    const { emit } = await import("@tauri-apps/api/event");
    await emit("bridge-menu-bar-connect-opencode");
  },
  onMenuBarConnection: (handler: (message: string) => void): Promise<UnlistenFn> => isTauri()
    ? listen<string>("bridge-menu-bar-connection", event => handler(event.payload)) : Promise.resolve(() => undefined),
  onMenuBarSettingsChanged: (handler: () => void): Promise<UnlistenFn> => isTauri()
    ? listen("bridge-menu-bar-settings-changed", handler) : Promise.resolve(() => undefined),
  getUsageOverview: (): Promise<UsageOverviewSnapshot | null> => isTauri()
    ? call("usage/get_usage_overview") : Promise.resolve(null),
  refreshUsageOverview: async (): Promise<UsageOverviewSnapshot | null> => {
    if (!isTauri()) return null;
    const snapshot = await call("usage/refresh_usage_overview");
    const { emit } = await import("@tauri-apps/api/event");
    await Promise.all([
      emit("bridge-usage-overview", snapshot).catch(() => undefined),
      emit("bridge-menu-bar-settings-changed").catch(() => undefined),
    ]);
    return snapshot;
  },
  onUsageOverview: (handler: (snapshot: UsageOverviewSnapshot) => void): Promise<UnlistenFn> => isTauri()
    ? listen<UsageOverviewSnapshot>("bridge-usage-overview", event => handler(event.payload)) : Promise.resolve(() => undefined),
  onMenuBarSettings: (handler: () => void): Promise<UnlistenFn> => isTauri()
    ? listen("bridge-menu-bar-settings", handler) : Promise.resolve(() => undefined),
  // Opening and closing the meter is window work, so the shell does it. Same
  // channel pattern as `revealMainWindow`: the panel is positioned against the
  // status item's rect, which only the tray handler knows.
  openMeterPanel: async (): Promise<void> => {
    if (!isTauri()) return;
    const { emit } = await import("@tauri-apps/api/event");
    await emit("bridge-meter-panel", "toggle");
  },
  hideMeterPanel: async (): Promise<void> => {
    if (!isTauri()) return;
    const { emit } = await import("@tauri-apps/api/event");
    await emit("bridge-meter-panel", "hide");
  },
  // The desktop shell owns this channel (native tray menu/left-click), not the
  // protocol — same exemption as MENU_COMMAND_EVENT in api.boundary.test.ts.
  onMeterTray: (handler: (action: "refresh") => void): Promise<UnlistenFn> => {
    if (!isTauri()) return Promise.resolve(() => undefined);
    return listen<string>("bridge-meter-tray", event => {
      if (event.payload === "refresh") handler(event.payload);
    });
  },
  // The composer's inline typeahead. Off by default; `configured: false` is a
  // fresh install reading defaults, same distinction Work's settings make.
  getSuggestionSettings: (): Promise<SuggestionSettingsSnapshot> =>
    isTauri() ? call("models/get_suggestion_settings") : Promise.resolve(structuredClone(mockSuggestionSettings)),
  // Validation is Rust's; this surface may pre-empt an obvious mistake, but a
  // payload that bypasses it is refused server-side by the same rules.
  saveSuggestionSettings: (settings: SuggestionSettings): Promise<SuggestionSettingsSnapshot> => {
    if (isTauri()) return call("models/save_suggestion_settings", { settings });
    mockSuggestionSettings = { configured: true, settings: structuredClone(settings) };
    return Promise.resolve(structuredClone(mockSuggestionSettings));
  },
  // Ask the typeahead engine to continue the composer's current draft. The
  // caller is expected to gate this on the setting being enabled and the
  // draft being non-empty — this call does not re-check either for the mock.
  suggestCompletion: (text: string): Promise<SuggestCompletionResult> => {
    if (isTauri()) return call("models/suggest_completion", { text });
    const suggestion = text.trim().endsWith("?") || text.length < 3 ? "" : " …";
    return Promise.resolve({ suggestion, usedFallback: false, fallbackReason: null });
  },
  configState: (): Promise<ConfigState> => isTauri() ? call("config/get_config_state") : Promise.resolve(structuredClone(mockConfigState)),
  saveHarnessConfig: (config: HarnessConfig): Promise<ConfigState> => {
    if (isTauri()) return call("config/save_harness_config", { config });
    mockConfigState.harnesses = mockConfigState.harnesses.map(item => item.id === config.id ? { ...structuredClone(config), isOverride: true } : item);
    return Promise.resolve(structuredClone(mockConfigState));
  },
  resetHarnessConfig: (id: HarnessConfig["id"]): Promise<ConfigState> => {
    if (isTauri()) return call("config/reset_harness_config", { id });
    mockConfigState.harnesses = mockConfigState.harnesses.map(item => item.id === id ? { ...item, enabled: true, defaultModel: null, effort: null, systemPrompt: "", advanced: {}, isOverride: false } : item);
    return Promise.resolve(structuredClone(mockConfigState));
  },
  refreshOpenCodeCatalog: (directory?: string): Promise<OpenCodeCatalog> => {
    if (isTauri()) return call("config/refresh_opencode_catalog", { directory: directory || null }) as Promise<OpenCodeCatalog>;
    return Promise.resolve(structuredClone(mockOpenCodeCatalog));
  },
  setOpenCodeProviderApiKey: (providerId: string, apiKey: string, directory?: string): Promise<OpenCodeCatalog> => {
    if (isTauri()) return call("config/set_opencode_provider_api_key", { providerId, apiKey, directory: directory || null }) as Promise<OpenCodeCatalog>;
    void apiKey;
    mockOpenCodeCatalog = { ...mockOpenCodeCatalog, providers: mockOpenCodeCatalog.providers.map(provider => provider.id === providerId ? { ...provider, connected: true } : provider) };
    return Promise.resolve(structuredClone(mockOpenCodeCatalog));
  },
  removeOpenCodeProviderAuth: (providerId: string, directory?: string): Promise<OpenCodeCatalog> => {
    if (isTauri()) return call("config/remove_opencode_provider_auth", { providerId, directory: directory || null }) as Promise<OpenCodeCatalog>;
    mockOpenCodeCatalog = { ...mockOpenCodeCatalog, providers: mockOpenCodeCatalog.providers.map(provider => provider.id === providerId ? { ...provider, connected: false, models: [] } : provider) };
    return Promise.resolve(structuredClone(mockOpenCodeCatalog));
  },
  saveAgentConfig: (agent: AgentDefinition): Promise<ConfigState> => {
    if (isTauri()) return call("config/save_agent_config", { agent });
    const value = { ...structuredClone(agent), id: agent.id || `custom-${crypto.randomUUID()}`, isDefault: false, updatedAt: new Date().toISOString() };
    const index = mockConfigState.agents.findIndex(item => item.id === value.id);
    if (index >= 0) mockConfigState.agents[index] = value; else mockConfigState.agents.push(value);
    return Promise.resolve(structuredClone(mockConfigState));
  },
  deleteAgentConfig: (id: string): Promise<ConfigState> => {
    if (isTauri()) return call("config/delete_agent_config", { id });
    const original = mockConfigState.agents.find(item => item.id === id);
    if (original?.isBuiltIn) mockConfigState.agents = mockConfigState.agents.map(item => item.id === id ? { ...item, systemPrompt: "", enabled: true, model: null } : item);
    else mockConfigState.agents = mockConfigState.agents.filter(item => item.id !== id);
    if (mockConfigState.defaultAgentId === id) mockConfigState.defaultAgentId = "bridge-orchestrator";
    mockConfigState.agents = mockConfigState.agents.map(item => ({ ...item, isDefault: item.id === mockConfigState.defaultAgentId }));
    return Promise.resolve(structuredClone(mockConfigState));
  },
  setDefaultAgent: (id: string): Promise<ConfigState> => {
    if (isTauri()) return call("config/set_default_agent", { id });
    mockConfigState.defaultAgentId = id;
    mockConfigState.agents = mockConfigState.agents.map(item => ({ ...item, isDefault: item.id === id }));
    return Promise.resolve(structuredClone(mockConfigState));
  },
  savePermissionPolicy: (policy: PermissionPolicy): Promise<ConfigState> => {
    if (isTauri()) return call("config/save_permission_policy", { policy });
    mockConfigState.permissionPolicy = { ...policy, updatedAt: new Date().toISOString() };
    return Promise.resolve(structuredClone(mockConfigState));
  },
  resetAllConfig: (): Promise<ConfigState> => {
    if (isTauri()) return call("config/reset_all_config");
    mockConfigState.harnesses = mockConfigState.harnesses.map(item => ({ ...item, enabled: true, defaultModel: null, effort: null, systemPrompt: "", advanced: {}, isOverride: false }));
    mockConfigState.agents = mockConfigState.agents.filter(item => item.isBuiltIn).map(item => ({ ...item, enabled: true, model: null, systemPrompt: "", isDefault: item.id === "bridge-orchestrator" }));
    mockConfigState.defaultAgentId = "bridge-orchestrator";
    // Reset clears every configuration row on the real path, the policy included.
    mockConfigState.permissionPolicy = { autoApproveProviderPermissions: false, workerPromptProposalRoles: [], updatedAt: "" };
    // Prompt-section overrides are configuration rows too: cleared, but their
    // history survives as appended reset revisions.
    mockPromptResetAll();
    return Promise.resolve(structuredClone(mockConfigState));
  },
  promptStack: (target: PromptTargetChoice, depth?: number): Promise<PromptStackView> =>
    isTauri() ? call("config/get_prompt_stack", { target, depth: depth ?? null }) : Promise.resolve(mockPromptStack(target, mockPromptDepth(depth))),
  savePromptSection: (target: PromptTargetChoice, sectionId: string, text: string, depth?: number): Promise<PromptSectionMutationResult> => {
    if (isTauri()) return call("config/save_prompt_section", { target, sectionId, text, depth: depth ?? null });
    return Promise.resolve().then(() => mockPromptMutation(target, sectionId, mockPromptDepth(depth), { state: "overridden", text }));
  },
  resetPromptSection: (target: PromptTargetChoice, sectionId: string, depth?: number): Promise<PromptSectionMutationResult> => {
    if (isTauri()) return call("config/reset_prompt_section", { target, sectionId, depth: depth ?? null });
    return Promise.resolve().then(() => mockPromptMutation(target, sectionId, mockPromptDepth(depth), { state: "default" }));
  },
  restorePromptRevision: (target: PromptTargetChoice, sectionId: string, revisionId: number, depth?: number): Promise<PromptSectionMutationResult> => {
    if (isTauri()) return call("config/restore_prompt_revision", { target, sectionId, revisionId, depth: depth ?? null });
    return Promise.resolve().then(() => {
      const resolved = mockPromptDepth(depth);
      const key = `${target}:${sectionId}`;
      const record = mockPromptSections.get(key);
      const restored = record?.revisions.find(revision => revision.id === revisionId);
      if (!restored) throw new Error(`mock prompt revision ${revisionId} does not belong to ${key}`);
      return mockPromptMutation(target, sectionId, resolved, structuredClone(restored.state), revisionId);
    });
  },
  previewCompiledPrompt: (target: PromptTargetChoice, depth?: number): Promise<CompiledPromptPreviewResult> => {
    if (isTauri()) return call("config/preview_compiled_prompt", { target, depth: depth ?? null });
    return mockPromptPreview(target, mockPromptDepth(depth));
  },
  learningState: (workspaceId: string): Promise<LearningState> => isTauri() ? call("learning/get_learning_state", { workspaceId }) as Promise<LearningState> : Promise.resolve(structuredClone(mockLearningState)),
  runLearning: (triggerKind: LocalLearningTriggerKind = "manual", workspaceId: string): Promise<LearningRun> => {
    if (isTauri()) return call("learning/run_learning", { triggerKind, workspaceId }) as Promise<LearningRun>;
    if (mockLearningState.latestRun) {
      const duplicate = { ...structuredClone(mockLearningState.latestRun), triggerKind, duplicate: true };
      return Promise.resolve(duplicate);
    }
    const createdAt = new Date().toISOString();
    const run: LearningRun = { id: crypto.randomUUID(), jobId: "default", triggerKind, idempotencyKey: "default:0:1", evidenceBoundary: 0, basePolicyVersion: 1, status: "noop", report: { reason: "insufficient evidence: 0/5 outcomes", evidenceBoundary: 0, evidenceCount: 0, basePolicyVersion: 1, candidatePolicyVersion: null, qualityBps: null, averageCostMicrousd: null, averageLatencyMs: null, retryRateBps: null, interventionRateBps: null, averageConfidenceBps: null, costComplete: false, evaluatedSpendMicrousd: 0, evaluatedTokens: 0, evaluationExecution: "not_run", replayPassed: null, promotionStatus: "not_requested", policyDiff: {}, recommendationOnly: true }, candidatePolicyVersion: null, cancellationRequested: false, leaseExpiresAt: null, replayPassed: null, promotionStatus: "not_requested", duplicate: false, createdAt, completedAt: createdAt };
    mockLearningState.latestRun = run;
    return Promise.resolve(structuredClone(run));
  },
  cancelLearningRun: (runId: string): Promise<LearningRun> => {
    if (isTauri()) return call("learning/cancel_learning_run", { runId }) as Promise<LearningRun>;
    if (!mockLearningState.latestRun || mockLearningState.latestRun.id !== runId) return Promise.reject(new Error("Learning run not found"));
    mockLearningState.latestRun = { ...mockLearningState.latestRun, status: "cancelled", cancellationRequested: true, promotionStatus: "cancelled", completedAt: new Date().toISOString() };
    return Promise.resolve(structuredClone(mockLearningState.latestRun));
  },
  updateLearningSchedule: (schedule: LearningSchedule): Promise<LearningSchedule> => {
    if (isTauri()) return call("learning/update_learning_schedule", { schedule });
    mockLearningState.schedule = structuredClone(schedule);
    return Promise.resolve(structuredClone(schedule));
  },
  approveLearningRun: (runId: string): Promise<LearningRun> => {
    if (isTauri()) return call("learning/approve_learning_run", { runId }) as Promise<LearningRun>;
    if (!mockLearningState.latestRun || mockLearningState.latestRun.id !== runId || mockLearningState.latestRun.promotionStatus !== "awaiting_approval") return Promise.reject(new Error("Learning run is not awaiting approval"));
    mockLearningState.activePolicyVersion = mockLearningState.latestRun.candidatePolicyVersion ?? mockLearningState.activePolicyVersion;
    mockLearningState.latestRun = { ...mockLearningState.latestRun, promotionStatus: "promoted" };
    return Promise.resolve(structuredClone(mockLearningState.latestRun));
  },
  rollbackRoutingPolicy: (workspaceId: string, targetVersion: number, explanation: string): Promise<LearningState> => {
    if (isTauri()) return call("routing/rollback_routing_policy", { workspaceId, targetVersion, explanation }) as Promise<LearningState>;
    void targetVersion;
    void explanation;
    mockLearningState.activePolicyVersion += 1;
    mockLearningState.canaryPolicyVersion = null;
    return Promise.resolve(structuredClone(mockLearningState));
  },
  registerLearningTrigger: (kind: ExternalLearningTriggerKind, registrationId: string, credentialRef: string | null, expiresAt: string | null = null): Promise<void> => isTauri()
    ? unit(call("learning/register_learning_trigger", { kind, registrationId, credentialRef, expiresAt }))
    : Promise.resolve(),
  learningTriggerInstructions: (kind: ExternalLearningTriggerKind, databasePath: string, registrationId: string): Promise<string> => isTauri()
    ? call("learning/get_learning_trigger_instructions", { kind, databasePath, registrationId })
    : Promise.resolve(`Run \`bridge learning run --database "${databasePath}" --trigger ${kind === "open_code" ? "opencode" : kind}:${registrationId}\` locally as a wake-up trigger only. Bridge owns replay, approval, promotion, and rollback.`),
  enableLearningTrigger: (kind: ExternalLearningTriggerKind, registrationId: string): Promise<void> => isTauri()
    ? unit(call("learning/enable_learning_trigger", { kind, registrationId }))
    : Promise.resolve(),
  routerPreferences: (workspaceId: string): Promise<RouterPreferences> => isTauri()
    ? call("routing/get_router_preferences", { workspaceId })
    : Promise.resolve(structuredClone(mockRouterPreferences.get(workspaceId) ?? { mode: "shadow", minimumPassBps: 6500, pinnedHarness: null, pinnedModel: null, excludedHarnesses: [], excludedModels: [] })),
  updateRouterPreferences: (workspaceId: string, preferences: RouterPreferences): Promise<RouterPreferences> => {
    if (isTauri()) return call("routing/update_router_preferences", { workspaceId, preferences });
    mockRouterPreferences.set(workspaceId, structuredClone(preferences));
    return Promise.resolve(structuredClone(preferences));
  },
  sessionForest: (sessionId: string): Promise<SessionForestSnapshot> => isTauri() ? call("sessions/get_session_forest", { sessionId }) as Promise<SessionForestSnapshot> : Promise.resolve(mockForest(sessionId)),
  // Tens of bytes per poll instead of the entire history; equal digests mean
  // sessionForest would return unchanged store content.
  sessionForestDigest: (sessionId: string): Promise<string> => isTauri() ? call("sessions/get_session_forest_digest", { sessionId }).then(result => result.digest) : Promise.resolve(`mock-${sessionId}`),
  contextBreakdown: (sessionId: string): Promise<ContextBreakdownResult> => isTauri() ? call("sessions/get_context_breakdown", { sessionId }) : Promise.resolve(mockContextBreakdown(sessionId)),
  // Same change-token contract as sessionForestDigest, scoped to breakdown
  // inputs: compilations, config revisions, adapter observations, branch.
  contextBreakdownDigest: (sessionId: string): Promise<string> => isTauri() ? call("sessions/get_context_breakdown_digest", { sessionId }).then(result => result.digest) : Promise.resolve(mockContextBreakdown(sessionId).digest),
  /** Durable backfill of one session's event log — any session id, including a
   * worker child's. Cursor semantics: pass the last sequence already held. */
  replaySessionEvents: (sessionId: string, afterSequence = 0, limit?: number, tail?: boolean): Promise<AgentEvent[]> => {
    if (isTauri()) return call("sessions/replay_session_events", { sessionId, afterSequence, limit, tail }) as unknown as Promise<AgentEvent[]>;
    const all = mockState.agentEvents.filter(event => event.sessionId === sessionId && event.sequence > afterSequence).sort((a, b) => a.sequence - b.sequence);
    const page = tail ? all.slice(Math.max(0, all.length - (limit ?? all.length))) : all.slice(0, limit ?? all.length);
    return Promise.resolve(structuredClone(page));
  },
  createCompletionPlan: async (sessionId: string, acceptanceCriteria: string[], changedPaths: string[], repositoryCommands: string[], markdownProjection: string | null = null, markdownCommitted = false): Promise<CompletionSummary> => {
    if (isTauri()) return call("completion/create_completion_plan", { sessionId, acceptanceCriteria, changedPaths, repositoryCommands, markdownProjection, markdownCommitted });
    const forest = mockForest(sessionId); if (!forest.completion) throw new Error("Mock completion plan is available only on the demo orchestrator"); return forest.completion;
  },
  recordCompletionCheck: async (attemptId: string, run: CompletionCheckRun): Promise<CompletionSummary> => {
    if (isTauri()) return call("completion/record_completion_check", { attemptId, run });
    const forest = Object.values(mockForests).find(item => item.completion?.attemptId === attemptId); if (!forest?.completion) throw new Error("Completion attempt not found");
    const index = forest.completion.checks.findIndex(check => check.checkId === run.checkId); if (index < 0) throw new Error("Completion check not found"); forest.completion.checks[index] = structuredClone(run); forest.completion.passedRequired = forest.completion.checks.filter(check => check.required && check.status === "passed").length; return structuredClone(forest.completion);
  },
  waiveCompletion: async (attemptId: string, checkIds: string[], reason: string): Promise<CompletionSummary> => {
    if (isTauri()) return call("completion/waive_completion", { attemptId, checkIds, reason });
    const forest = Object.values(mockForests).find(item => item.completion?.attemptId === attemptId); if (!forest?.completion) throw new Error("Completion attempt not found"); const unresolved = forest.completion.checks.filter(check => check.required && check.status !== "passed").map(check => check.checkId); if (!unresolved.every(checkId => checkIds.includes(checkId))) throw new Error("Waiver must cover every unresolved required check"); forest.completion.verdict = "waived"; forest.completion.waiverReason = reason; return structuredClone(forest.completion);
  },
  // Local state on a suggested task. None of these reaches a connector: marking a task
  // done does not close the thread it came from, and dismissing it does not archive
  // anything. They are notes Bridge makes to itself about something it read.
  workTaskAction: async (taskId: string, action: "done" | "snooze" | "dismiss" | "restore", snoozedUntil: string | null = null): Promise<void> => {
    if (isTauri()) return unit(call("work/task_action", { taskId, action, snoozedUntil }));
    const task = mockWorkTasks.find(item => item.id === taskId);
    if (!task) throw new Error("Task not found");
    task.state = action === "done" ? "done" : action === "snooze" ? "snoozed" : action === "dismiss" ? "dismissed" : "active";
    return undefined;
  },
  workTaskPin: async (taskId: string, pinned: boolean): Promise<void> => {
    if (isTauri()) return unit(call("work/task_pin", { taskId, pinned }));
    const task = mockWorkTasks.find(item => item.id === taskId);
    if (task) task.pinned = pinned;
    return undefined;
  },
  // Prepares a draft and returns. Sending is the user's move, which is why nothing here
  // returns a turn id or a run.
  workTaskPrepareSession: async (taskId: string, harness: Harness, model: string | null): Promise<WorkTaskDraft> => {
    if (isTauri()) return call("work/task_prepare_session", { taskId, harness, model });
    const task = mockWorkTasks.find(item => item.id === taskId);
    if (!task) throw new Error("Task not found");
    return {
      sessionId: "session-prepared",
      title: task.title,
      draft: `From ${task.sourceKind}: ${task.title}\n\n${task.why}`,
    };
  },
  workTaskOpenEvidence: async (taskId: string) => {
    if (isTauri()) return call("work/task_open_evidence", { taskId });
    const task = mockWorkTasks.find(item => item.id === taskId);
    if (!task?.evidenceTarget) throw new Error("This task has no evidence to open");
    return structuredClone(task.evidenceTarget);
  },
  // The Work board. Read-only and store-only by construction on the Rust side, so
  // this is the whole of what opening the Work screen does — no session is selected,
  // no model starts, and nothing touches the network. The browser fallback below is
  // what vitest and a `bun run dev` preview render, so the screen can be developed
  // and tested without the desktop app.
  workBoard: async (): Promise<WorkBoard> => {
    if (isTauri()) return call("work/get_work_board");
    return browserWorkBoard();
  },
  // Work's configuration. `configured: false` is a fresh install reading defaults;
  // `configured: true` with `briefing: null` is briefing explicitly switched off —
  // the write path keeps those two states distinguishable.
  readWorkSettings: async (): Promise<WorkSettingsSnapshot> => {
    if (isTauri()) return call("work/read_settings");
    return structuredClone(mockWorkSettings);
  },
  // Validation is Rust's. The Settings surface may pre-empt an obvious mistake,
  // but a payload that bypasses it is refused by the same rules server-side.
  writeWorkSettings: async (settings: WorkSettings): Promise<WorkSettingsSnapshot> => {
    if (isTauri()) return call("work/write_settings", { settings });
    mockWorkSettings = { configured: true, settings: structuredClone(settings) };
    return structuredClone(mockWorkSettings);
  },
  // Which harnesses passed the briefing conformance gate, why the others were
  // refused, and the cheapest capable default model for each.
  workBriefingOptions: async (): Promise<WorkBriefingOptions> => {
    if (isTauri()) return call("work/briefing_options");
    return structuredClone(mockBriefingOptions);
  },
  // Trigger a briefing run. Returns a receipt immediately — a claimed run lands
  // on the run row, and the board's suggestions.state is how the screen follows it.
  runWorkBriefing: async (trigger: "manual" | "focus" | "schedule"): Promise<WorkBriefReceipt> => {
    if (isTauri()) return call("work/run_briefing", { trigger });
    if (!mockWorkSettings.configured || !mockWorkSettings.settings.briefing) {
      return { outcome: "refused", runId: null, code: "not_configured", detail: "Work has never been configured" };
    }
    return { outcome: "refused", runId: null, code: "desktop_required", detail: "Reading connected integrations requires the desktop app." };
  },
  cancelWorkBriefing: async (): Promise<WorkBriefReceipt> => {
    if (isTauri()) return call("work/cancel_briefing");
    return { outcome: "refused", runId: null, code: "not_running", detail: "no briefing run is active" };
  },
  // A workspace far behind its base branch produces changes and completion
  // stamps against stale code; `refresh` is the explicit choice the warning offers.
  workspaceBaseDivergence: async (sessionId: string, fetch: boolean): Promise<BaseBranchDivergence> => {
    if (isTauri()) return call("worktrees/workspace_base_divergence", { sessionId, fetch });
    return { baseRef: null, baseCommit: null, head: null, branch: null, ahead: 0, behind: 0, refAgeSeconds: null, fetchAttempted: false, fetched: false, dirty: false, unavailableReason: "Base-branch comparison needs the desktop app" };
  },
  refreshWorkspaceBase: async (sessionId: string): Promise<BaseBranchDivergence> => {
    if (isTauri()) return call("worktrees/refresh_workspace_base", { sessionId });
    throw new Error("Refreshing the workspace needs the desktop app");
  },
  // Worker output that lives only in a child worktree has not reached the user's
  // task checkout; adopting or discarding it is an explicit decision.
  pendingWorkerAdoptions: async (sessionId: string): Promise<WorkerRepositoryBinding[]> => {
    if (isTauri()) return call("worktrees/pending_worker_adoptions", { sessionId });
    return sessionId === "session-1" ? [structuredClone(mockPendingAdoption)] : [];
  },
  adoptWorkerWorktree: async (sessionId: string): Promise<WorkerRepositoryBinding> => {
    if (isTauri()) return call("worktrees/adopt_worker_worktree", { sessionId });
    throw new Error("Adopting a worker worktree needs the desktop app");
  },
  discardWorkerWorktree: async (sessionId: string, reason: string): Promise<WorkerRepositoryBinding> => {
    if (isTauri()) return call("worktrees/discard_worker_worktree", { sessionId, reason });
    throw new Error("Discarding a worker worktree needs the desktop app");
  },
  // Put one chat away and reclaim the checkout it owns — never its workspace's,
  // which belongs to every other chat in it. History is kept; the conversation
  // is simply no longer listed.
  archiveChat: async (sessionId: string): Promise<ArchiveChatResult> => {
    if (isTauri()) return call("sessions/archive_chat", { sessionId });
    const owned = mockWorktrees.find(item => item.ownerSessionId === sessionId);
    mockState.sessions = mockState.sessions.filter(session => session.id !== sessionId);
    if (owned && owned.disposition === "reclaimable") {
      mockWorktrees = mockWorktrees.filter(item => item.id !== owned.id);
      emitState();
      return { archived: true, bytesFreed: owned.sizeBytes ?? 0, worktreeDetail: null };
    }
    emitState();
    return { archived: true, bytesFreed: 0, worktreeDetail: owned?.retainedReason ?? null };
  },
  listArchivedChats: async (query = "", offset = 0, rootSessionId: string | null = null): Promise<ArchivedChatsResult> => {
    if (isTauri()) return call("sessions/list_archived_chats", { query, offset, rootSessionId });
    return { chats: [], hasMore: false };
  },
  workerSettings: async (workspaceId: string): Promise<WorkerSettings> => {
    if (isTauri()) return call("config/get_worker_settings", { workspaceId });
    return { defaultHarness: null, maxConcurrentWorkers: 2, maxWorkersPerTurn: 3, stallTimeoutSeconds: 600, warmRetentionMinutes: 5, automaticRetry: true, providerFailover: true };
  },
  saveWorkerSettings: async (workspaceId: string, settings: WorkerSettings): Promise<WorkerSettings> => {
    if (isTauri()) return call("config/save_worker_settings", { workspaceId, settings });
    return structuredClone(settings);
  },
  reviewerSettings: async (): Promise<ReviewerSettingsResult> => {
    if (isTauri()) return call("config/get_reviewer_settings");
    return { settings: { harnesses: {}, systemPrompt: "" }, defaultSystemPrompt: MOCK_REVIEWER_PROMPT };
  },
  saveReviewerSettings: async (settings: ReviewerSettings): Promise<ReviewerSettingsResult> => {
    if (isTauri()) return call("config/save_reviewer_settings", { settings });
    return { settings: structuredClone(settings), defaultSystemPrompt: MOCK_REVIEWER_PROMPT };
  },
  unarchiveChat: async (sessionId: string): Promise<void> => {
    if (isTauri()) { await call("sessions/unarchive_chat", { sessionId }); return; }
    throw new Error("Unarchiving a chat needs the desktop app");
  },
  // What the worktrees cost. Read-only on purpose: reclaiming is the
  // retention sweep's decision, taken against a fresh safety classification,
  // not something a client can ask for out of band.
  listWorktrees: async (): Promise<WorktreeInventoryEntry[]> => {
    if (isTauri()) return call("worktrees/list_worktrees");
    return structuredClone(mockWorktrees);
  },
  worktreeUsage: async (): Promise<WorktreeUsage> => {
    if (isTauri()) return call("worktrees/worktree_usage");
    return structuredClone(mockWorktreeUsage);
  },
  // A refusal is a result, not a thrown error: the caller renders "no, and
  // here is why" next to the row it asked about.
  reclaimWorktree: async (worktreeId: string, force = false): Promise<WorktreeReclaimResult> => {
    if (isTauri()) return call("worktrees/reclaim_worktree", { worktreeId, force });
    const entry = mockWorktrees.find(item => item.id === worktreeId);
    if (!entry) throw new Error(`no worktree ${worktreeId} is recorded`);
    // Mirrors is_removable: force overrides only at_risk/unverifiable, never
    // retained (external, live session, unadopted output).
    const forcible = force && (entry.disposition === "at_risk" || entry.disposition === "unverifiable");
    if (entry.disposition !== "reclaimable" && entry.disposition !== "pushed_unmerged" && !forcible) {
      return { reclaimed: false, bytesFreed: 0, disposition: entry.disposition ?? "retained", detail: entry.retainedReason };
    }
    mockWorktrees = mockWorktrees.filter(item => item.id !== worktreeId);
    return { reclaimed: true, bytesFreed: entry.sizeBytes ?? 0, disposition: entry.disposition ?? "reclaimable", detail: null };
  },
  sweepWorktrees: async (): Promise<WorktreeSweepResult> => {
    if (isTauri()) return call("worktrees/sweep_worktrees");
    const collectable = mockWorktrees.filter(item => item.disposition === "reclaimable");
    mockWorktrees = mockWorktrees.filter(item => item.disposition !== "reclaimable");
    return {
      removed: collectable.length,
      removedBytes: collectable.reduce((total, item) => total + (item.sizeBytes ?? 0), 0),
      retained: mockWorktrees.length,
      retainedBytes: mockWorktrees.reduce((total, item) => total + (item.sizeBytes ?? 0), 0),
      overBudgetBytes: 0,
      skipped: 0,
      measurementsTruncated: 0,
    };
  },
  registerVerifierManifest: async (source: string, manifest: VerifierManifest): Promise<void> => {
    if (isTauri()) return unit(call("completion/register_verifier_manifest", { source, manifest }));
    mockVerifierManifests.set(manifest.id, structuredClone(manifest));
  },
  verifierCandidates: async (changeLabels: string[], availableCapabilities: string[]): Promise<VerifierCandidate[]> => {
    if (isTauri()) return call("completion/verifier_candidates", { changeLabels, availableCapabilities });
    return [...mockVerifierManifests.values()].map(manifest => {
      const triggerMatch = !manifest.triggers?.length || manifest.triggers.some(trigger => changeLabels.includes(trigger));
      const missing = (manifest.requiredCapabilities ?? []).filter(capability => !availableCapabilities.includes(capability));
      const exclusionReasons = [...(!triggerMatch ? ["change triggers do not match"] : []), ...(missing.length ? [`missing capabilities: ${missing.join(", ")}`] : [])];
      return { manifest: structuredClone(manifest), eligible: exclusionReasons.length === 0, exclusionReasons };
    });
  },
  activateSessionEntry: async (sessionId: string, entryId: string): Promise<SessionForestSnapshot> => {
    if (isTauri()) return call("sessions/activate_session_entry", { sessionId, entryId }) as Promise<SessionForestSnapshot>;
    if (!mockForests[sessionId]) mockForest(sessionId);
    const forest = mockForests[sessionId];
    if (!forest.entries.some(entry => entry.id === entryId)) throw new Error("Entry is not in this session");
    if (forest.head) forest.head.activeEntryId = entryId;
    forest.reasons.unshift({ id: nextEventId++, source: "session-forest", kind: "session.head_moved", entityId: sessionId, body: `Conversation head moved to ${entryId}; files were not changed`, createdAt: new Date().toISOString() });
    emitState(); return structuredClone(forest);
  },
  resolveReference: async (id: string): Promise<ResolveReferenceResult> => {
    if (isTauri()) return call("sessions/resolve_reference", { id }) as Promise<ResolveReferenceResult>;
    const bare = id.replace(/^@session:/, "").replace(/^brio_/, "");
    const session = mockState.sessions.find(candidate => candidate.id === bare || (candidate.id.replace(/-/g, "").startsWith(bare) && bare.length === 8));
    if (session) {
      const head = mockForests[session.id]?.head ?? null;
      return {
        kind: "session",
        session_id: session.id,
        label: session.label,
        harness: session.harness,
        workspace_id: session.workspaceId ?? null,
        parent_session_id: session.parentSessionId ?? null,
        depth: session.depth ?? 0,
        restoration_mode: session.restorationMode,
        continuation_fidelity: session.continuationFidelity,
        active_entry_id: head?.activeEntryId ?? null,
        latest_checkpoint_entry_id: head?.latestCheckpointEntryId ?? null,
        updated_at: session.startedAt ?? null,
        authorized: true,
      };
    }
    for (const forest of Object.values(mockForests)) {
      const entry = forest.entries.find(candidate => candidate.id === bare);
      if (entry) {
        const summary = String(entry.payload?.text ?? entry.payload?.summary ?? entry.payload?.title ?? "") || "";
        return {
          kind: "entry",
          session_id: forest.sessionId,
          entry_id: entry.id,
          entry_kind: entry.kind,
          sequence: Number(entry.sequence),
          summary,
          created_at: entry.createdAt,
          authorized: true,
        };
      }
    }
    return { kind: "unknown", authorized: false };
  },
  forkSession: async (sessionId: string, entryId: string, title?: string | null, harness?: string | null, model?: string | null, worktreePolicy?: string | null): Promise<ForkSessionResult> => {
    if (isTauri()) return call("sessions/fork_session", { sessionId, entryId, title, harness, model, worktreePolicy: worktreePolicy ?? "shared" }) as Promise<ForkSessionResult>;
    const source = mockState.sessions.find(session => session.id === sessionId);
    if (!source) throw new Error("Session to fork does not exist");
    if (source.kind === "worker") throw new Error("Worker sessions cannot be forked; fork an orchestrator or direct chat");
    const forest = mockForest(sessionId);
    const cutoff = forest.entries.findIndex(entry => entry.id === entryId);
    if (cutoff < 0) throw new Error("Entry is not in this session");
    const prefix = forest.entries.slice(0, cutoff + 1);
    const forkId = `fork-${nextEventId++}`;
    const entries = prefix.map((entry, index) => ({
      ...entry,
      sessionId: forkId,
      parentEntryId: index === 0 ? null : prefix[index - 1].id,
      sequence: index + 1,
      providerEventId: null,
      createdAt: new Date().toISOString(),
    }));
    const checkpoint = [...entries].reverse().find(entry => entry.kind === "checkpoint");
    const head: SessionForestSnapshot["head"] = {
      sessionId: forkId,
      activeEntryId: entries.at(-1)!.id,
      nativeProviderSessionId: null,
      restorationMode: "checkpoint_restored",
      resumeEligibility: "checkpoint_restored",
      latestCheckpointEntryId: checkpoint?.id ?? null,
      updatedAt: new Date().toISOString(),
    };
    mockForests[forkId] = { ...forest, sessionId: forkId, entries, head, leaves: [entries.at(-1)!] };
    mockState.sessions.push({
      ...source,
      id: forkId,
      label: title?.trim() || `Fork of ${source.label}`,
      title: title?.trim() || `Fork of ${source.label}`,
      harness: harness ?? source.harness,
      model: model ?? source.model,
      status: "idle",
      activeTurnId: null,
      providerSessionId: null,
      // A fork is a top-level conversation, not a delegated worker: the
      // agent-tree fields stay as the source had them and the lineage goes in
      // the fork fields. Mirrors `fork_session_records`.
      parentSessionId: null,
      depth: source.depth ?? 0,
      forkParentSessionId: sessionId,
      forkParentEntryId: entryId,
      restorationMode: "checkpoint_restored",
      continuationFidelity: "projected_at_boundary",
    });
    mockState.events.unshift(
      { id: nextEventId, source: "session-forest", kind: "fork.created", entityId: forkId, body: `Forked from ${sessionId} at ${entryId}`, createdAt: new Date().toISOString() },
    );
    nextEventId += 1;
    emitState();
    return { state: structuredClone(mockState) as BridgeState, sessionId: forkId, snapshot: structuredClone(mockForests[forkId]), fidelity: "projected_at_boundary" };
  },
  compactSession: async (sessionId: string): Promise<void> => {
    if (isTauri()) return unit(call("sessions/compact_session", { sessionId }));
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
  searchSessionEntries: async (sessionId: string, query: string, limit?: number | null, offset?: number | null): Promise<SearchSessionEntriesResult> => {
    if (isTauri()) {
      return call("sessions/search_session_entries", {
        sessionId,
        query,
        ...(limit != null ? { limit } : {}),
        ...(offset ? { offset } : {}),
      });
    }
    if (!sessionId.trim()) throw new Error("Recall needs a session id; search cannot run across a workspace");
    const tokens = query.toLowerCase().split(/[^a-z0-9]+/).filter(Boolean);
    if (!tokens.length) throw new Error("Recall needs a word to search for in this chat");
    const forest = mockForest(sessionId);
    const kinds = new Set(["user.message", "assistant.message", "worker.result", "compaction", "checkpoint", "branch.summary"]);
    const hits = forest.entries
      .filter(entry => kinds.has(entry.kind))
      .filter(entry => {
        const body = `${entry.payload.text ?? ""} ${entry.payload.title ?? ""} ${entry.payload.summary ?? ""}`.toLowerCase();
        return tokens.every(token => body.includes(token));
      })
      .map(entry => ({
        entryId: entry.id,
        kind: entry.kind,
        sequence: entry.sequence,
        snippet: String(entry.payload.text ?? entry.payload.summary ?? entry.payload.title ?? ""),
        createdAt: entry.createdAt,
      }));
    // Page the mock the same way the server does, so the Show more affordance
    // is exercised by `bun run dev` and not only by the desktop app.
    const size = limit ?? 20;
    const start = offset ?? 0;
    return { sessionId, query, hits: hits.slice(start, start + size), offset: start, hasMore: hits.length > start + size };
  },
  /**
   * Write one session's durable record out as JSONL.
   *
   * The result is a path, never the transcript itself: a long session is
   * megabytes, and the point of the export is an artifact you can grep, diff
   * and hand to something that is not Bridge.
   */
  exportSessionTranscript: async (
    sessionId: string,
    options: { scope?: TranscriptExportScope; includeHidden?: boolean; destinationPath?: string } = {},
  ): Promise<ExportSessionTranscriptResult> => {
    if (isTauri()) {
      return call("sessions/export_session_transcript", {
        sessionId,
        ...(options.scope ? { scope: options.scope } : {}),
        ...(options.includeHidden != null ? { includeHidden: options.includeHidden } : {}),
        ...(options.destinationPath ? { destinationPath: options.destinationPath } : {}),
      });
    }
    // Mock mode has no filesystem. Reporting a plausible path would be a lie a
    // reader could only catch by going to look for the file, so say plainly
    // that the export needs the desktop app.
    throw new Error("Exporting a transcript needs the desktop app; the browser preview has no session store to read.");
  },
  saveMemoryRecord: async (body: string, kind?: string | null, sessionId?: string | null): Promise<MemoryRecord> => {
    if (isTauri()) {
      return call("memory/save_memory_record", {
        body,
        ...(kind ? { kind } : {}),
        ...(sessionId ? { sessionId } : {}),
      });
    }
    const trimmed = body.trim();
    if (!trimmed) throw new Error("A memory pin needs some text. Empty bodies are not stored.");
    const now = new Date().toISOString();
    const record: MemoryRecord = {
      id: crypto.randomUUID(),
      scopeKey: "account:local",
      kind: kind?.trim() || "preference",
      body: trimmed,
      provenance: "user_explicit",
      status: "active",
      sourceSessionId: sessionId?.trim() || undefined,
      validFrom: now,
      createdAt: now,
      updatedAt: now,
    };
    mockMemoryRecords.unshift(record);
    emitMemoryChanged(record.scopeKey);
    return structuredClone(record);
  },
  listMemoryRecords: async (scopeKey: string, status?: string): Promise<ListMemoryRecordsResult> => {
    if (isTauri()) return call("memory/list_memory_records", { scopeKey, ...(status ? { status } : {}) });
    const trimmed = scopeKey.trim();
    if (!trimmed) throw new Error("Memory scope is required; it cannot be empty or NULL");
    const wanted = status ?? "active";
    if (wanted !== "active" && wanted !== "proposed") throw new Error(`Memory list can show active or proposed records, not '${wanted}'.`);
    return {
      scopeKey: trimmed,
      records: mockMemoryRecords
        .filter(record => record.scopeKey === trimmed && record.status === wanted)
        .slice(0, 50)
        .map(record => structuredClone(record)),
    };
  },
  supersedeMemoryRecord: async (recordId: string, body: string, kind?: string | null): Promise<MemoryRecord> => {
    if (isTauri()) return call("memory/supersede_memory_record", { recordId, body, ...(kind ? { kind } : {}) });
    const old = mockMemoryRecords.find(item => item.id === recordId && item.status === "active");
    if (!old) throw new Error("Only an active memory record can be superseded.");
    const trimmed = body.trim();
    if (!trimmed) throw new Error("A memory pin needs some text. Empty bodies are not stored.");
    const now = new Date().toISOString();
    old.status = "superseded";
    old.validTo = now;
    old.updatedAt = now;
    const record: MemoryRecord = {
      id: crypto.randomUUID(),
      scopeKey: old.scopeKey,
      kind: kind?.trim() || old.kind,
      body: trimmed,
      provenance: "user_explicit",
      status: "active",
      sourceSessionId: old.sourceSessionId,
      supersedes: old.id,
      validFrom: now,
      createdAt: now,
      updatedAt: now,
    };
    mockMemoryRecords.unshift(record);
    emitMemoryChanged(record.scopeKey);
    return structuredClone(record);
  },
  approveMemoryRecord: async (recordId: string): Promise<MemoryRecord> => {
    if (isTauri()) return call("memory/approve_memory_record", { recordId });
    const record = mockMemoryRecords.find(item => item.id === recordId && item.status === "proposed");
    if (!record) throw new Error("Only a proposed memory record can be approved.");
    record.status = "active";
    record.updatedAt = new Date().toISOString();
    emitMemoryChanged(record.scopeKey);
    return structuredClone(record);
  },
  rejectMemoryRecord: async (recordId: string): Promise<MemoryRecord> => {
    if (isTauri()) return call("memory/reject_memory_record", { recordId });
    const record = mockMemoryRecords.find(item => item.id === recordId && item.status === "proposed");
    if (!record) throw new Error("Only a proposed memory record can be rejected.");
    record.status = "rejected";
    record.updatedAt = new Date().toISOString();
    emitMemoryChanged(record.scopeKey);
    return structuredClone(record);
  },
  getExtractionSettings: async (): Promise<MemoryExtractionSettings> => {
    if (isTauri()) return call("memory/get_extraction_settings");
    return structuredClone(mockExtractionSettings);
  },
  updateExtractionSettings: async (mode: string, harness?: string | null, model?: string | null): Promise<MemoryExtractionSettings> => {
    if (isTauri()) {
      return call("memory/update_extraction_settings", {
        mode,
        ...(harness ? { harness } : {}),
        ...(model ? { model } : {}),
      });
    }
    if (mode !== "remember" && mode !== "propose" && mode !== "auto_apply") {
      throw new Error(`Unknown extraction mode '${mode}'. Use remember, propose, or auto_apply.`);
    }
    if (Boolean(harness) !== Boolean(model)) throw new Error("Pin both a helper and a model, or neither to run on each chat's own model.");
    mockExtractionSettings = { ...mockExtractionSettings, mode, harness: harness ?? undefined, model: model ?? undefined };
    return structuredClone(mockExtractionSettings);
  },
  getMemoryInjection: async (): Promise<MemoryInjectionSettings> => {
    if (isTauri()) return call("memory/get_memory_injection");
    return { scopeKey: "account:local", enabled: mockMemoryInjection };
  },
  setMemoryInjection: async (enabled: boolean): Promise<MemoryInjectionSettings> => {
    if (isTauri()) return call("memory/set_memory_injection", { enabled });
    mockMemoryInjection = enabled;
    return { scopeKey: "account:local", enabled };
  },
  getPacketAudit: async (sessionId: string): Promise<MemoryPacketAudit> => {
    if (isTauri()) return call("memory/get_packet_audit", { sessionId });
    return { sessionId, selected: [], tokenEstimate: 0 };
  },
  getMemoryCapabilities: async (): Promise<MemoryCapabilities> => {
    if (isTauri()) return call("memory/get_memory_capabilities");
    return {
      ledger: { exists: true, scopeKey: "account:local", maxBodyChars: 4000, kinds: ["preference", "fact", "decision", "constraint"] },
      providerNative: [
        { harness: "claude", command: "memory", description: "Edit CLAUDE.md memory files" },
        { harness: "codex", command: "memories", description: "Configure memory use and generation" },
      ],
    };
  },
  deleteMemoryRecord: async (recordId: string): Promise<MemoryRecord> => {
    if (isTauri()) return call("memory/delete_memory_record", { recordId });
    const record = mockMemoryRecords.find(item => item.id === recordId && item.status === "active");
    if (!record) throw new Error("That memory pin is not active (unknown id or already forgotten).");
    const closedAt = new Date().toISOString();
    record.status = "deleted";
    record.validTo = closedAt;
    record.updatedAt = closedAt;
    emitMemoryChanged(record.scopeKey);
    return structuredClone(record);
  },
  // Memory read-only aggregations. Display-only: they grant nothing and rank
  // nothing. The protocol-first `memory.recall_stats` Rust+daemon method is the
  // tracked follow-up; on the desktop host these return empty until that lands,
  // and in the mock host they fold the deterministic audit above so the surface
  // is fully exercisable.
  memoryRecallStats: async (scopeKey: string): Promise<MemoryRecallStats> => {
    const trimmed = scopeKey.trim();
    if (!trimmed) throw new Error("Memory scope is required; it cannot be empty or NULL");
    if (isTauri()) {
      return { perRecord: [], injectionsPerDay: Array<number>(14).fill(0), budgetCharsUsed: 0, budgetCharsMax: PACKET_BUDGET_CHARS };
    }
    return deriveRecallStats(mockMemoryRecords.filter(record => record.scopeKey === trimmed), mockPacketAudit);
  },
  memoryConsolidationLog: async (scopeKey: string): Promise<MemoryConsolidationEntry[]> => {
    if (!scopeKey.trim()) throw new Error("Memory scope is required; it cannot be empty or NULL");
    if (isTauri()) return [];
    return structuredClone(mockConsolidationLog);
  },
  addProject: async (path: string): Promise<BridgeState> => {
    if (isTauri()) return call("projects/add_project", { path });
    const name = path.split("/").filter(Boolean).at(-1) || "Repository";
    mockState.projects.push({ id: crypto.randomUUID(), name, path, createdAt: new Date().toISOString() }); emitState(); return snapshot();
  },
  createWorkspace: async (title: string): Promise<BridgeState> => {
    if (isTauri()) return call("workspaces/create_workspace", { title });
    const id = crypto.randomUUID();
    mockState.workspaces.push({ id, projectId: null, city: null, title, branch: null, path: null, status: "idle", dirtyFiles: 0, additions: 0, deletions: 0, createdAt: new Date().toISOString() });
    emitState(); return snapshot();
  },
  createChat: async (harness: Harness, model: string | null, title: string | null): Promise<BridgeState> => {
    if (isTauri()) return call("sessions/create_chat", { harness, model, title });
    const id = crypto.randomUUID();
    mockState.sessions.push({ id, workspaceId: null, harness, label: title || "New chat", status: "idle", startedAt: null, endedAt: null, contextPercent: null, usagePercent: null, metricSource: "estimated", providerSessionId: null, activeTurnId: null, model, requestedTier: "fast", restorationMode: "fresh", continuationFidelity: "native", title, kind: "direct", cwd: null }); emitState(); return snapshot();
  },
  createAsideChat: async (sourceSessionId: string, harness: Harness, model: string | null, title: string | null): Promise<CreateAsideChatResult> => {
    if (isTauri()) return call("sessions/create_aside_chat", { sourceSessionId, harness, model, title });
    const source = mockState.sessions.find(item => item.id === sourceSessionId);
    if (!source) throw new Error("Aside source session does not exist");
    const id = crypto.randomUUID();
    mockState.sessions.push({
      id,
      workspaceId: source.workspaceId,
      harness,
      label: title || "New aside",
      status: "idle",
      startedAt: null,
      endedAt: null,
      contextPercent: null,
      usagePercent: null,
      metricSource: "estimated",
      providerSessionId: null,
      activeTurnId: null,
      model,
      requestedTier: "standard",
      restorationMode: "fresh",
      continuationFidelity: "projected_at_boundary",
      title,
      kind: "direct",
      cwd: source.cwd ?? null,
    });
    emitState();
    return {
      state: snapshot(),
      sourceSessionId,
      sessionId: id,
      handoffStatus: "carried",
      fidelity: "projected_at_boundary",
    };
  },
  createWorkspaceSession: async (workspaceId: string, createWorktree = false): Promise<BridgeState> => {
    if (isTauri()) return call("sessions/create_workspace_session", { workspaceId, createWorktree });
    const id = crypto.randomUUID();
    const workspace = mockState.workspaces.find(item => item.id === workspaceId);
    if (createWorktree && !workspace?.projectId) throw new Error("Connect a Git repository before creating an isolated worktree");
    const cwd = createWorktree ? `/tmp/bridge/worktrees/${id}` : workspace?.path ?? null;
    mockState.sessions.push({ id, workspaceId, harness: "codex", label: "Orchestrator", status: "idle", startedAt: null, endedAt: null, contextPercent: null, usagePercent: null, metricSource: "estimated", providerSessionId: null, activeTurnId: null, model: null, requestedTier: "fast", restorationMode: "fresh", continuationFidelity: "native", title: null, kind: "orchestrator", cwd }); emitState(); return snapshot();
  },
  updateChatModel: async (sessionId: string, harness: Harness, model: string | null, effort?: string | null): Promise<BridgeState> => {
    if (isTauri()) return call("sessions/update_chat_model", { sessionId, harness, model, effort });
    const session = mockState.sessions.find(item => item.id === sessionId);
    if (session?.activeTurnId) throw new Error("Wait for the current response before switching models");
    if (session && ["direct", "orchestrator"].includes(session.kind ?? "")) { session.harness = harness; session.model = model; if (effort) session.effort = effort; session.status = "idle"; session.providerSessionId = null; session.restorationMode = "fresh"; }
    emitState(); return snapshot();
  },
  carrySessionHandoff: async (targetSessionId: string, sourceSessionId: string): Promise<boolean> => {
    if (isTauri()) return (await call("sessions/carry_session_handoff", { targetSessionId, sourceSessionId })).carried;
    return false;
  },
  listSlashCommands: async (sessionId?: string): Promise<SlashCommand[]> => {
    if (isTauri()) return call("slash/list_slash_commands", { sessionId: sessionId ?? null });
    return [];
  },
  resolveSlashCommand: async (sessionId: string, text: string): Promise<SlashCommandResolve | null> => {
    if (isTauri()) return call("slash/resolve_slash_command", { sessionId, text });
    return null;
  },
  listWorkspaceFiles: (sessionId: string): Promise<string[]> => isTauri()
    ? call("workspaces/list_workspace_files", { sessionId })
    : Promise.resolve(["src/App.tsx", "src/api.ts", "src/types.ts", "src-tauri/src/lib.rs", "README.md"]),
  connectWorkspaceFolder: async (workspaceId: string, path: string): Promise<BridgeState> => {
    if (isTauri()) return call("workspaces/connect_workspace_folder", { workspaceId, path });
    const workspace = mockState.workspaces.find(item => item.id === workspaceId); if (workspace) { workspace.path = path; workspace.branch = "main"; }
    emitState(); return snapshot();
  },
  cloneWorkspaceRepo: async (url: string, destination?: string): Promise<BridgeState> => {
    if (isTauri()) return call("workspaces/clone_workspace_repo", { url, destination: destination || null });
    const title = url.trim().replace(/\/$/, "").split(/[/:]/).pop()?.replace(/\.git$/, "") || "project";
    const id = crypto.randomUUID(); mockState.workspaces.push({ id, title, status: "idle", path: destination || `/tmp/bridge/projects/${title}`, projectId: id, city: "Kyoto", branch: "main", dirtyFiles: 0, additions: 0, deletions: 0, createdAt: new Date().toISOString() }); emitState(); return snapshot();
  },
  startChat: async (sessionId: string): Promise<BridgeState> => {
    if (isTauri()) return call("sessions/start_chat", { sessionId });
    const session = mockState.sessions.find(item => item.id === sessionId);
    if (session) { session.status = "working"; session.startedAt = new Date().toISOString(); session.endedAt = null; session.providerSessionId = session.providerSessionId ?? `mock-${crypto.randomUUID()}`; session.restorationMode = "fresh"; appendAgent(session.id, "session.started", { status: "working" }); }
    emitState(); return snapshot();
  },
  startSession: async (workspaceId: string, harness?: Harness | null, model?: string | null): Promise<BridgeState> => {
    if (isTauri()) return call("sessions/start_session", { workspaceId, harness: harness ?? null, model: model ?? null });
    const resolvedHarness = harness ?? "codex";
    let session = mockState.sessions.find(item => item.workspaceId === workspaceId && item.harness === resolvedHarness);
    if (!session) { session = { id: crypto.randomUUID(), workspaceId, harness: resolvedHarness, label: "Orchestrator", status: "idle", startedAt: null, endedAt: null, contextPercent: null, usagePercent: null, metricSource: "estimated", model: model ?? "gpt-5.6-luna", requestedTier: "fast", restorationMode: "fresh", continuationFidelity: "native", kind: "orchestrator" }; mockState.sessions.push(session); }
    session.restorationMode = session.providerSessionId ? "native" : "fresh"; session.status = "working"; session.startedAt = new Date().toISOString(); session.endedAt = null; session.providerSessionId = session.providerSessionId ?? `mock-${crypto.randomUUID()}`; session.model = model ?? session.model ?? "gpt-5.6-luna"; session.label = "Orchestrator";
    const workspace = mockState.workspaces.find(item => item.id === workspaceId); if (workspace) workspace.status = "working";
    appendAgent(session.id, "session.started", { status: "working" }); emitState(); return snapshot();
  },
  stopSession: async (sessionId: string): Promise<BridgeState> => {
    if (isTauri()) return call("sessions/stop_session", { sessionId });
    const session = mockState.sessions.find(item => item.id === sessionId); if (session) { session.status = "stopped"; session.endedAt = new Date().toISOString(); session.activeTurnId = null; }
    emitState(); return snapshot();
  },
  prepareTurn: (sessionId: string, text: string): Promise<SanitizedTurn> => isTauri()
    ? call("sessions/prepare_turn", { sessionId, text })
    : Promise.resolve({ text, interceptions: [] }),
  sendTurn: async (sessionId: string, text: string): Promise<void> => {
    if (isTauri()) return unit(call("sessions/send_turn", { sessionId, text }));
    const session = mockState.sessions.find(item => item.id === sessionId); if (!session) throw new Error("Structured adapter session is not running");
    session.status = "working"; session.activeTurnId = `mock-turn-${nextEventId}`;
    appendAgent(sessionId, "message.completed", { itemId: `user-${nextEventId}`, role: "user", status: "completed", text });
    const assistantItemId = `assistant-${nextEventId}`;
    appendAgent(sessionId, "message.delta", { itemId: assistantItemId, role: "assistant", status: "streaming", text: "I’ll handle that through the normalized adapter layer. " });
    appendAgent(sessionId, "message.completed", { itemId: assistantItemId, role: "assistant", status: "completed", text: "I’ll handle that through the normalized adapter layer. The GUI remains provider-neutral, and no agent TUI is rendered." });
    session.status = "ready"; session.activeTurnId = null; emitState();
  },
  // The active-turn input contract. Unlike sendTurn this is safe to call while
  // the agent is working: the backend decides between starting a turn, steering
  // the live one, and durably queueing, and says which it did. Attachments ride
  // beside the text; a provider that cannot take them refuses explicitly, which
  // is how the composer surfaces "not supported" instead of dropping bytes.
  submitInput: async (sessionId: string, text: string, attachments?: readonly ComposerAttachment[]): Promise<SubmitInputResult> => {
    const images: TurnImage[] = (attachments ?? []).map(attachment => ({
      mediaType: attachment.mediaType,
      base64Data: attachment.dataUri.split(",")[1] ?? "",
    }));
    if (isTauri()) return call("sessions/submit_input", { sessionId, text, attachments: images.length > 0 ? images : undefined });
    const session = mockState.sessions.find(item => item.id === sessionId); if (!session) throw new Error("Structured adapter session is not running");
    if (session.activeTurnId) {
      const steering = mockHealth.adapters.some(adapter => adapter.id === session.harness && adapter.capabilities.includes("steering"));
      appendAgent(sessionId, "message.completed", { itemId: `user-${nextEventId}`, role: "user", status: "completed", text, data: { delivery: steering ? "steered" : "queued", ...(images.length > 0 ? { attachments: images.map(image => ({ mediaType: image.mediaType, dataUri: `data:${image.mediaType};base64,${image.base64Data}` })) } : {}) } });
      emitState();
      return { disposition: steering ? "steeredActiveTurn" : "queuedForPhaseBoundary", queuedInputId: steering ? undefined : `mock-queue-${nextEventId}`, interceptions: [] };
    }
    await bridgeApi.sendTurn(sessionId, text);
    return { disposition: "startedNewTurn", interceptions: [] };
  },
  dispatchAgentShortcut: async (sessionId: string, token: string, objective: string): Promise<DispatchAgentShortcutResult> => {
    if (isTauri()) return call("sessions/dispatch_agent_shortcut", { sessionId, token, objective });
    if (!objective.trim()) throw new Error("Agent shortcut objective cannot be empty; add what the specialist should do");
    const session = mockState.sessions.find(item => item.id === sessionId);
    if (!session?.workspaceId) throw new Error("Agent shortcuts need a connected workspace");
    const normalized = normalizeAgentToken(token);
    const aliases: Record<string, string> = { researcher: "research", implementer: "implementation", verifier: "verification", reviewer: "verification", planner: "planning", documenter: "documentation", docs: "documentation" };
    const role = aliases[normalized] ?? normalized;
    const exact = mockConfigState.agents.filter(agent => normalizeAgentToken(agent.id ?? "") === normalized || normalizeAgentToken(agent.name) === normalized);
    const matches = exact.length > 0 ? exact : mockConfigState.agents.filter(agent => agent.role === role);
    if (matches.length !== 1) throw new Error(matches.length > 1 ? `Agent shortcut #${normalized} is ambiguous` : `Unknown agent shortcut #${normalized}`);
    const selected = matches[0];
    if (!selected.enabled) throw new Error(`Agent shortcut #${normalized} targets disabled agent ${selected.name}`);
    if (selected.role === "orchestrator") throw new Error(`Agent shortcut #${normalized} cannot target an orchestrator`);
    if (!mockConfigState.harnesses.some(harness => harness.id === selected.harness && harness.enabled)) throw new Error(`Agent shortcut #${normalized} uses disabled harness ${selected.harness}`);
    appendAgent(sessionId, "message.completed", { itemId: `user-${nextEventId}`, role: "user", status: "completed", text: objective.trim(), data: { delivery: "directAgent", directDispatch: true, agentId: selected.id, agentRole: selected.role } });
    emitState();
    return {
      disposition: selected.role === "implementation" ? "awaitingApproval" : "launched",
      childSessionId: selected.role === "implementation" ? undefined : `mock-worker-${nextEventId}`,
      agentId: selected.id ?? "",
      agentName: selected.name,
      role: selected.role,
      interceptions: [],
    };
  },
  interruptTurn: async (sessionId: string): Promise<void> => {
    if (isTauri()) return unit(call("sessions/interrupt_turn", { sessionId }));
    const session = mockState.sessions.find(item => item.id === sessionId);
    if (!session) throw new Error("Session not found");
    session.status = "stopped";
    session.activeTurnId = null;
    appendAgent(sessionId, "turn.completed", { status: "cancelled", title: "Stopped", data: { reason: "user_stopped" } });
    emitState();
  },
  // The user's half of the retry decision. Bridge stopped taking this turn on
  // its own for a cause it cannot show has changed.
  retryWorkerTask: (childSessionId: string): Promise<void> => isTauri() ? unit(call("sessions/retry_worker_task", { childSessionId })) : Promise.resolve(),
  refreshAccountUsage: (): Promise<void> => isTauri() ? unit(call("sessions/refresh_account_usage")) : Promise.resolve(),
  resolveApproval: async (sessionId: string, eventId: number, decision: ApprovalDecision, optionId?: string): Promise<InteractionResolutionResult> => {
    if (isTauri()) return call("approvals/resolve_approval", { sessionId, eventId, decision, optionId });
    const request = mockState.agentEvents.find(item => item.id === eventId);
    if (request) appendAgent(request.sessionId, readWireKind(request.kind) === "permission.requested" ? "permission.resolved" : "approval.resolved", { status: decision, data: { requestEventId: eventId, decision, optionId, resolvedBy: "human" } });
    emitState();
    return { disposition: "resolved", interactionKind: "permission", status: decision, resolvedBy: "human", decision };
  },
  resolveQuestion: async (sessionId: string, eventId: number, action: QuestionAction, answers: Record<string, string[]> = {}): Promise<InteractionResolutionResult> => {
    if (isTauri()) return call("approvals/resolve_question", { sessionId, eventId, action, answers });
    const request = mockState.agentEvents.find(item => item.id === eventId);
    if (request) appendAgent(request.sessionId, "question.resolved", { status: action === "answer" ? "answered" : action, data: { requestEventId: eventId, decision: action, resolvedBy: "human" } });
    emitState();
    return { disposition: "resolved", interactionKind: "question", status: action === "answer" ? "answered" : action, resolvedBy: "human", decision: action };
  },
  startProviderLogin: (provider: string): Promise<{ workspaceId: string; terminalId: string }> =>
    isTauri() ? call("auth/start_provider_login", { provider }) : Promise.resolve({ workspaceId: "provider-login", terminalId: provider }),
  cancelProviderLogin: (provider: string): Promise<void> =>
    isTauri() ? unit(call("auth/cancel_provider_login", { provider })) : Promise.resolve(),
  createTerminal: async (params: CreateTerminalParams): Promise<TerminalRecord> => isTauri() ? call("terminal/create_terminal", { ...params, restart: params.restart ?? false }) : mockCreateTerminal(params),
  terminalSnapshot: async (workspaceId: string, terminalId: string): Promise<TerminalSnapshot> => isTauri() ? call("terminal/get_terminal_snapshot", { workspaceId, terminalId }) : mockSnapshot(workspaceId, terminalId),
  terminalWorkspace: async (workspaceId: string): Promise<TerminalWorkspace> => isTauri() ? call("terminal/get_terminal_workspace", { workspaceId }) : mockTerminalWorkspace(workspaceId),
  saveTerminalLayout: async (workspaceId: string, layout: unknown): Promise<void> => isTauri() ? unit(call("terminal/save_terminal_workspace", { workspaceId, layout })) : mockSaveLayout(workspaceId, layout),
  renameTerminal: async (workspaceId: string, terminalId: string, title: string): Promise<TerminalRecord> => isTauri() ? call("terminal/rename_terminal", { workspaceId, terminalId, title }) : mockRenameTerminal(workspaceId, terminalId, title),
  onTerminalFrame: async (handler: (frame: TerminalFrame) => void): Promise<UnlistenFn> => isTauri() ? subscribe<TerminalFrame>("terminal-frame", handler) : () => undefined,
  onTerminalLagged: async (handler: () => void): Promise<UnlistenFn> => isTauri() ? subscribe("stream-lagged", handler) : () => undefined,
  openTerminal: (workspaceId: string, terminalId: string): Promise<void> => isTauri() ? unit(call("terminal/open_terminal", { workspaceId, terminalId })) : Promise.resolve(),
  writeTerminal: (workspaceId: string, terminalId: string, data: string): Promise<void> => isTauri() ? unit(call("terminal/write_terminal", { workspaceId, terminalId, data })) : Promise.resolve(),
  resizeTerminal: (workspaceId: string, terminalId: string, rows: number, cols: number): Promise<void> => isTauri() ? unit(call("terminal/resize_terminal", { workspaceId, terminalId, rows, cols })) : Promise.resolve(),
  closeTerminal: (workspaceId: string, terminalId: string): Promise<void> => isTauri() ? unit(call("terminal/close_terminal", { workspaceId, terminalId })) : Promise.resolve(mockCloseTerminal(workspaceId, terminalId)),
  listTerminals: async (workspaceId: string): Promise<string[]> => {
    if (isTauri()) return ((await call("terminal/list_terminals", { workspaceId })) as { terminalIds: string[] }).terminalIds;
    return [];
  },
  refreshWorkspace: (workspaceId: string): Promise<BridgeState> => isTauri() ? call("workspaces/refresh_workspace", { workspaceId }) : Promise.resolve(snapshot()),
  listWorkspaceBranches: (workspaceId: string): Promise<ListWorkspaceBranchesResult> => {
    if (isTauri()) return call("workspaces/list_workspace_branches", { workspaceId });
    const current = mockState.workspaces.find(workspace => workspace.id === workspaceId)?.branch ?? null;
    return Promise.resolve({
      current,
      branches: [...new Set([current, "main", "feat/sidebar-polish"].filter((branch): branch is string => !!branch))].sort(),
    });
  },
  checkoutWorkspaceBranch: async (workspaceId: string, branch: string): Promise<BridgeState> => {
    if (isTauri()) return call("workspaces/checkout_workspace_branch", { workspaceId, branch });
    const workspace = mockState.workspaces.find(item => item.id === workspaceId);
    if (workspace) workspace.branch = branch;
    emitState();
    return snapshot();
  },
  /** Menu picks from the shell. Not a protocol method — the native menu
   *  speaks command ids from `src/keymap.ts`, not RPC. Outside Tauri there is
   *  no menu, so this resolves to a no-op unsubscribe. */
  onMenuCommand: (handler: (id: CommandId) => void): Promise<UnlistenFn> =>
    isTauri()
      ? listen<CommandId>(MENU_COMMAND_EVENT, event => handler(event.payload))
      : Promise.resolve(() => {}),
  /** In-app ⌥⌘F. Not a protocol method — the shell listens on this event name. */
  notifyLayoutFullscreen: (on: boolean): void => {
    if (!isTauri()) return;
    void import("@tauri-apps/api/event").then(({ emit }) => {
      void emit("bridge-layout-fullscreen", on);
    });
  },
  archiveWorkspace: async (workspaceId: string): Promise<BridgeState> => {
    if (isTauri()) return call("workspaces/archive_workspace", { workspaceId });
    mockState.sessions = mockState.sessions.filter(session => session.workspaceId !== workspaceId); mockState.workspaces = mockState.workspaces.filter(workspace => workspace.id !== workspaceId); emitState(); return snapshot();
  },
  workspaceChanges: (workspaceId: string): Promise<WorkspaceChangesResult> =>
    isTauri() ? call("workspaces/workspace_changes", { workspaceId }) : Promise.resolve(mockWorkspaceChanges()),
  listWorkspaceTree: (workspaceId: string): Promise<string[]> =>
    isTauri() ? call("workspaces/list_workspace_tree", { workspaceId }) : Promise.resolve(mockTreePaths()),
  // `async` rather than `Promise.resolve(mock…())`: the mocks throw on the
  // refusal paths, and wrapping the *call* would throw synchronously instead
  // of rejecting — the one way a mock can behave unlike the daemon.
  readWorkspaceFile: async (workspaceId: string, path: string): Promise<ReadWorkspaceFileResult> =>
    isTauri() ? call("workspaces/read_workspace_file", { workspaceId, path }) : mockReadFile(path),
  writeWorkspaceFile: async (workspaceId: string, path: string, content: string, baseSha256: string | null): Promise<WriteWorkspaceFileResult> =>
    isTauri() ? call("workspaces/write_workspace_file", { workspaceId, path, content, baseSha256 }) : mockWriteFile(path, content, baseSha256),
  onTerminal: async (handler: (chunk: TerminalChunk) => void): Promise<UnlistenFn> => isTauri() ? subscribe<TerminalChunk>("session-output", handler) : () => undefined,
  onTerminalExited: async (handler: (exit: TerminalExit) => void): Promise<UnlistenFn> => isTauri() ? subscribe<TerminalExit>("terminal-exited", handler) : () => undefined,
  /**
   * The live conversation stream, one call per frame.
   *
   * The shell coalesces frames on their way across the process boundary — a
   * hundred-step turn is roughly four hundred of them, and one IPC message
   * each woke the webview four hundred times while it was trying to draw. The
   * batch is unpacked here so nothing above this line can tell: every
   * subscriber still sees one frame at a time, in order.
   */
  onAgentEvent: async (handler: (event: AgentEvent) => void): Promise<UnlistenFn> => {
    if (isTauri()) {
      return listen<AgentEvent[]>(AGENT_EVENT_BATCH, event => {
        for (const frame of event.payload) { recordStreamReceipt(frame); handler(frame); }
      });
    }
    agentListeners.add(handler);
    return () => agentListeners.delete(handler);
  },
  onAccountUsage: async (handler: (payload: AccountUsagePayload) => void): Promise<UnlistenFn> => {
    if (isTauri()) return subscribe<AccountUsagePayload>("account-usage", handler);
    return () => undefined;
  },
  onStateChanged: async (handler: () => void): Promise<UnlistenFn> => {
    if (isTauri()) return subscribe("state-changed", handler); stateListeners.add(handler); return () => stateListeners.delete(handler);
  },
  onAdaptersChanged: async (handler: () => void): Promise<UnlistenFn> => {
    if (isTauri()) return subscribe("adapters-changed", handler);
    return () => undefined;
  },
  onLearningJobChanged: async (handler: () => void): Promise<UnlistenFn> => {
    if (isTauri()) return subscribe("learning-job-changed", handler);
    return () => undefined;
  },
  onMemoryChanged: async (handler: (payload: MemoryChangedPayload) => void): Promise<UnlistenFn> => {
    if (isTauri()) return subscribe<MemoryChangedPayload>("memory-changed", handler);
    memoryListeners.add(handler);
    return () => memoryListeners.delete(handler);
  },
  /** Cold-start phase narration. Live-only — a client that missed one simply
   *  never shows that phase, so browser/mock mode has nothing to replay. */
  onSessionStartup: async (handler: (payload: SessionStartupPayload) => void): Promise<UnlistenFn> => {
    if (isTauri()) return subscribe<SessionStartupPayload>("session-startup", handler);
    return () => undefined;
  },
  onConnectorItemArrived: async (handler: (payload: ConnectorItemArrivedPayload) => void): Promise<UnlistenFn> => {
    if (isTauri()) return subscribe<ConnectorItemArrivedPayload>("connectors/item_arrived", handler);
    connectorArrivalListeners.add(handler);
    scheduleMockConnectorArrival();
    return () => connectorArrivalListeners.delete(handler);
  },
  onConnectorCardReady: async (handler: (payload: ConnectorCardReadyPayload) => void): Promise<UnlistenFn> => {
    if (isTauri()) return subscribe<ConnectorCardReadyPayload>("connectors/card_ready", handler);
    connectorCardListeners.add(handler);
    return () => connectorCardListeners.delete(handler);
  },
  onConnectorItemResolved: async (handler: (payload: ConnectorItemResolvedPayload) => void): Promise<UnlistenFn> => {
    if (isTauri()) return subscribe<ConnectorItemResolvedPayload>("connectors/item_resolved", handler);
    connectorResolvedListeners.add(handler);
    return () => connectorResolvedListeners.delete(handler);
  },
  onConnectorInboxChanged: async (handler: (payload: { family: string }) => void): Promise<UnlistenFn> => {
    if (isTauri()) return subscribe<{ family: string }>("connectors/inbox_changed", handler);
    connectorInboxListeners.add(handler);
    return () => connectorInboxListeners.delete(handler);
  },
  onGithubChecksChanged: async (handler: (payload: GithubChecksChangedPayload) => void): Promise<UnlistenFn> => {
    if (isTauri()) return subscribe<GithubChecksChangedPayload>("github/checks_changed", handler);
    return () => undefined;
  },
  onGithubCiFinished: async (handler: (payload: GithubCiFinishedPayload) => void): Promise<UnlistenFn> => {
    if (isTauri()) return subscribe<GithubCiFinishedPayload>("github/ci_finished", handler);
    githubCiListeners.add(handler);
    return () => githubCiListeners.delete(handler);
  },
};

/* ── Browser-mode file system ──────────────────────────────────────────────
   An in-memory tree so the editor is fully usable — open, edit, save, reopen —
   in `bun run dev` without a daemon. Writes are checked against the same
   hash-conflict rule the Rust side enforces, so the conflict path is
   reachable in the browser too. */

const mockFiles = new Map<string, string>([
  ["README.md", "# Bridge\n\nLocal control room for supervised coding-agent workspaces.\n"],
  ["package.json", "{\n  \"name\": \"bridge-deck\",\n  \"private\": true\n}\n"],
  ["src/main.tsx", "import { createRoot } from \"react-dom/client\";\nimport App from \"./App\";\n\ncreateRoot(document.getElementById(\"root\")!).render(<App />);\n"],
  ["src/App.tsx", "export default function App() {\n  return <main>Bridge</main>;\n}\n"],
  ["src/theme.ts", "export type Theme = \"system\" | \"light\" | \"dark\";\n\nexport function systemTheme(): Theme {\n  return matchMedia(\"(prefers-color-scheme: dark)\").matches ? \"dark\" : \"light\";\n}\n"],
  ["src/components/Markdown.tsx", "export function Markdown({ text }: { text: string }) {\n  return <div>{text}</div>;\n}\n"],
  ["src-tauri/src/lib.rs", "pub fn run() {\n    tauri::Builder::default().run(tauri::generate_context!()).unwrap();\n}\n"],
  ["src-tauri/bridge-core/src/policy.rs", "pub fn allow(path: &str, owner: &str) -> bool {\n    !path.is_empty() && !owner.is_empty()\n}\n"],
  ["scripts/prepare-daemon.sh", "#!/bin/sh\nset -eu\ncargo build --release --bin bridged\n"],
]);

// Browser-mode CI simulation: the first PR-list read arms one CI-finished
// event a few seconds out, so the toast → deep-link flow is exercisable in
// `bun run dev` without a daemon. Never in vitest — a stray timer there would
// fire into an unmounted tree.
let mockCiSimulated = false;
const simulateMockCiFinished = (workspaceId: string) => {
  // `process` exists under vitest (node and jsdom pools) but not in the Vite
  // browser build, so this arms in `bun run dev` only.
  if (mockCiSimulated || typeof process !== "undefined") return;
  mockCiSimulated = true;
  window.setTimeout(() => {
    const payload: GithubCiFinishedPayload = {
      workspaceId,
      number: 340,
      headBranch: "feat/github-surface-core",
      title: "Add the deterministic gh reader",
      failed: 1,
      total: 2,
    };
    githubCiListeners.forEach(listener => listener(payload));
  }, 6000);
};

const mockCheckouts = new Map<number, GithubCheckoutResult>();
const mockGithubCheckout = (_workspaceId: string, number: number): GithubCheckoutResult => {
  const existing = mockCheckouts.get(number);
  if (existing) return { ...existing, reused: true };
  const branch = mockGithubPullRequests(_workspaceId).pullRequests.find(pr => pr.number === number)?.headBranch ?? "main";
  const fresh: GithubCheckoutResult = {
    workspaceId: `ws-pr-${number}`,
    path: `~/.bridge/worktrees/github/pr-${number}-${branch.replace(/[^a-z0-9]+/gi, "-")}`,
    branch,
    reused: false,
  };
  mockCheckouts.set(number, fresh);
  return fresh;
};

const mockGithubPullRequests = (workspaceId: string): GithubPullRequestsResult => (simulateMockCiFinished(workspaceId), {
  pullRequests: [
    { number: 341, title: "Render the native GitHub read surface", state: "open", isDraft: false, author: { login: "atharva" }, headBranch: "feat/github-read-surface", reviewDecision: "reviewRequired", mergeability: "mergeable", mergeStateStatus: "CLEAN", checks: { total: 3, queued: 0, inProgress: 1, passed: 2, failed: 0, skipped: 0, cancelled: 0 }, url: "https://github.com/Atharva-Kanherkar/bridge-harness/pull/341" },
    { number: 340, title: "Add the deterministic gh reader", state: "open", isDraft: false, author: { login: "bridge" }, headBranch: "feat/github-surface-core", reviewDecision: "approved", mergeability: "conflicting", mergeStateStatus: "DIRTY", checks: { total: 3, queued: 0, inProgress: 1, passed: 1, failed: 1, skipped: 0, cancelled: 0 }, url: "https://github.com/Atharva-Kanherkar/bridge-harness/pull/340" },
  ],
});

const mockGithubStatus = (_workspaceId: string): GithubStatusResult => ({ availability: { status: "available" }, repository: { host: "github.com", owner: "Atharva-Kanherkar", name: "bridge-harness" } });
const mockSearchGithubRepos = (query: string): SearchGithubReposResult => ({
  repositories: ["Atharva-Kanherkar/bridge-harness", "Atharva-Kanherkar/animevocab", "rimo/rimo-frontend"]
    .filter(nameWithOwner => nameWithOwner.toLowerCase().includes(query.trim().toLowerCase()))
    .map(nameWithOwner => ({
      nameWithOwner,
      url: `https://github.com/${nameWithOwner}`,
      sshUrl: `git@github.com:${nameWithOwner}.git`,
      isPrivate: true,
      pushedAt: new Date().toISOString(),
    })),
});
const mockGithubConnect = (remoteUrl: string): GithubConnectResult => {
  const [owner = "bridge", name = "harness"] = remoteUrl.replace(/\.git$/, "").replace(/\/$/, "").split(/[/:]/).slice(-2);
  return { repository: { host: "github.com", owner, name }, initialized: false, replacedRemote: false };
};
const mockGithubMergeConfig = (): GithubMergeConfigResult => ({ strategies: { merge: true, squash: true, rebase: false }, defaultStrategy: "squash" });
const mockGithubAct = (action: GithubAction, confirmed: boolean): GithubActResult =>
  confirmed ? { executed: true, message: `Ran ${action.kind}.` } : { executed: false, message: `Declined: ${action.kind}` };
const mockGithubReview = (number: number, harness: string): GithubReviewResult =>
  ({ status: "launched", sessionId: "mock-review", message: `Review started with ${harness} — comments will post to PR #${number} shortly.` });
const mockGithubChecks = (_workspaceId: string, number: number): GithubChecksResult => ({ checks: [
  { name: "test", status: "completed", conclusion: number === 340 ? "failure" : "success", logUrl: "https://github.com/Atharva-Kanherkar/bridge-harness/actions", workflow: "CI" },
  { name: "typecheck", status: "completed", conclusion: "success", logUrl: "", workflow: "CI" },
  { name: "bundle size", status: "inProgress", conclusion: null, logUrl: "", workflow: "Size" },
] });
const mockGithubPullRequest = (workspaceId: string, number: number): GithubPullRequestResult => {
  const summary = mockGithubPullRequests(workspaceId).pullRequests.find(pr => pr.number === number) ?? mockGithubPullRequests(workspaceId).pullRequests[0];
  return {
    pullRequest: {
      summary,
      body: [
        "<!-- this template comment never renders -->",
        "Manage GitHub **without leaving Bridge**. Fixes #339, thanks @atharva.",
        "",
        "- [x] read surface",
        "- [ ] commit list",
        "",
        "See https://github.com/Atharva-Kanherkar/bridge-harness for the design notes.",
        "",
        "GitHub content remains plain text, including <script>alert('inert')</script>.",
        "",
        "A disguised link — <a href=\"javascript:void%200\">click for the logs</a> — renders as text, not an anchor.",
      ].join("\n"),
      baseBranch: "main",
      comments: [{ id: "conversation-1", author: { login: "maintainer" }, body: "This is the main PR conversation.\n\n<details><summary>CI output</summary>\n\n```\nok 12 passed\n```\n\n</details>", createdAt: now, url: `${summary.url}#issuecomment-1` }],
      labels: [{ name: "enhancement", color: "a2eeef", description: "New feature" }],
      commits: [
        { oid: "8f2a1c9d4e5b6a7c8d9e0f1a2b3c4d5e6f708192", abbreviatedOid: "8f2a1c9", messageHeadline: "feat(github): render the read surface", messageBody: "The pane owns its own height now.", committedAt: now, authors: [{ login: "atharva" }] },
        { oid: "1b0c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e", abbreviatedOid: "1b0c3d4", messageHeadline: "test(github): cover the inert render", messageBody: "", committedAt: now, authors: [{ login: "bridge" }] },
      ],
      additions: 18,
      deletions: 4,
      changedFiles: 2,
    },
    reviewThreads: [{ id: "thread-1", isResolved: false, isOutdated: false, path: "src/api.ts", line: 42, originalLine: null, comments: [{ id: "comment-1", databaseId: 1, author: { login: "reviewer" }, body: "Please keep <b>remote HTML</b> inert.", createdAt: now, url: summary.url, replyToId: null }] }],
    files: [
      { path: "src/api.ts", previousPath: null, status: "modified", additions: 14, deletions: 4, patch: "@@ -1,2 +1,3 @@\n-old line\n+new line\n context" },
      { path: "assets/github.png", previousPath: null, status: "added", additions: 0, deletions: 0, patch: null },
    ],
  };
};

const mockGithubIssues = (): GithubIssuesResult => ({ issues: [{
  number: 339,
  title: "Native GitHub surface",
  state: "open",
  author: { login: "atharva" },
  labels: [{ name: "enhancement", color: "a2eeef", description: "New feature" }],
  createdAt: now,
  updatedAt: now,
  url: "https://github.com/Atharva-Kanherkar/bridge-harness/issues/339",
}] });

const mockGithubIssue = (number: number): GithubIssueResult => {
  const summary = mockGithubIssues().issues.find(issue => issue.number === number) ?? mockGithubIssues().issues[0];
  return { issue: {
    summary,
    body: "Manage GitHub without leaving Bridge. Remote <script>HTML stays inert</script>.",
    comments: [{ id: "issue-comment-1", author: { login: "reviewer" }, body: "Issue comment", createdAt: now, url: summary.url }],
  } };
};

const mockGithubRepository = (): GithubRepositoryResult => ({
  nameWithOwner: "Atharva-Kanherkar/bridge-harness",
  description: "A native control room for coding-agent work.",
  visibility: "PUBLIC",
  defaultBranch: "main",
  primaryLanguage: "Rust",
  url: "https://github.com/Atharva-Kanherkar/bridge-harness",
  openIssues: 24,
  openPullRequests: 5,
  labels: [
    { name: "bug", color: "d73a4a", description: "Something is broken" },
    { name: "enhancement", color: "a2eeef", description: "New feature" },
  ],
});

/** Not SHA-256 — just a stable content token with the same conflict semantics. */
function mockHash(content: string): string {
  let hash = 0x811c9dc5;
  for (let index = 0; index < content.length; index += 1) {
    hash = Math.imul(hash ^ content.charCodeAt(index), 0x01000193) >>> 0;
  }
  return hash.toString(16).padStart(8, "0");
}

const mockTreePaths = (): string[] => [...mockFiles.keys()].sort();

function mockReadFile(path: string): ReadWorkspaceFileResult {
  const content = mockFiles.get(path);
  if (content === undefined) throw new Error(`${path} does not exist`);
  return { path, content, sha256: mockHash(content), tooLarge: false, binary: false, sizeBytes: content.length };
}

function mockWriteFile(path: string, content: string, baseSha256: string | null): WriteWorkspaceFileResult {
  const existing = mockFiles.get(path);
  if (baseSha256 === null && existing !== undefined) throw new Error(`${path} already exists`);
  if (baseSha256 !== null && existing === undefined) throw new Error(`${path} no longer exists on disk`);
  if (baseSha256 !== null && mockHash(existing!) !== baseSha256) throw new Error(`${path} changed on disk since it was opened`);
  mockFiles.set(path, content);
  return { sha256: mockHash(content) };
}

/** Browser-mode fixture for the Changes tab: one file per importance tier,
 * plus a lockfile, so the collapse-by-default affordance has something to
 * hide even without a daemon. */
function mockWorkspaceChanges(): WorkspaceChangesResult {
  return {
    baseCommit: "a1b2c3d4",
    repositoryState: "normal",
    totalFiles: 4,
    filesTruncated: false,
    files: [
      {
        path: "src-tauri/bridge-core/src/policy.rs",
        previousPath: null,
        changeKind: "modified",
        additions: 18,
        deletions: 4,
        patch: "@@ -10,7 +10,21 @@\n-fn allow(path: &str) -> bool {\n+fn allow(path: &str, owner: &str) -> bool {\n     true\n }\n",
        patchTruncated: false,
        binary: false,
        importance: "high",
        labels: ["rust"],
        lowSignal: false,
      },
      {
        path: "src/components/ChangesPanel.tsx",
        previousPath: null,
        changeKind: "modified",
        additions: 42,
        deletions: 6,
        patch: "@@ -1,3 +1,5 @@\n+import { useState } from \"react\";\n export function ChangesPanel() {\n   return null;\n }\n",
        patchTruncated: false,
        binary: false,
        importance: "medium",
        labels: ["frontend"],
        lowSignal: false,
      },
      {
        path: "src/utils.ts",
        previousPath: null,
        changeKind: "modified",
        additions: 3,
        deletions: 1,
        patch: "@@ -4,5 +4,7 @@\n export function slug(value: string) {\n-  return value;\n+  return value.toLowerCase();\n }\n",
        patchTruncated: false,
        binary: false,
        importance: "low",
        labels: ["frontend"],
        lowSignal: false,
      },
      {
        path: "bun.lock",
        previousPath: null,
        changeKind: "modified",
        additions: 240,
        deletions: 12,
        patch: "",
        patchTruncated: false,
        binary: false,
        importance: "low",
        labels: [],
        lowSignal: true,
      },
    ],
  };
}

/// Browser-mode fixtures: one managed, one user-managed, one absent, so the
/// three interesting cards are all reachable without a daemon.
///
/// Treated as immutable. An operation returns a fresh status rather than mutating
/// these, so a browser session does not accumulate state that a real daemon would
/// never report.
const mockManagedAgents: ManagedAgentList = {
  agents: [
    {
      agentId: "claude", label: "Claude Code", state: "ready", backing: "managed", removable: true,
      executable: "/managed-runtimes/agents/claude/installations/a1b2c3/payload/node_modules/@anthropic-ai/claude-agent-sdk-darwin-arm64/claude",
      version: "0.3.209", consecutiveFailures: 0,
    },
    {
      agentId: "codex", label: "Codex", state: "external", backing: "external", removable: false,
      executable: "/opt/homebrew/bin/codex", version: "0.147.0", consecutiveFailures: 0,
    },
    {
      agentId: "cursor", label: "Cursor", state: "external", backing: "external", removable: false,
      executable: "/Users/demo/.local/bin/cursor-agent", consecutiveFailures: 0,
    },
    {
      agentId: "opencode", label: "OpenCode", state: "not_installed", backing: "none", removable: false,
      consecutiveFailures: 0,
    },
  ],
};

function mockManagedOperation(agentId: string, kind: ManagedAgentOperationKind): Promise<ManagedAgentOperationResult> {
  const agent = mockManagedAgents.agents.find(item => item.agentId === agentId);
  if (!agent) return Promise.reject(new Error(`${agentId} is not a built-in agent`));
  if (kind === "uninstall") {
    if (!agent.removable) return Promise.reject(new Error(`${agent.label} is user-managed; Bridge will not remove it`));
    const status: ManagedAgentStatus = {
      ...structuredClone(agent), state: "not_installed", backing: "none", removable: false,
      executable: undefined, version: undefined,
    };
    return Promise.resolve({ agentId, kind, outcome: "removed", status });
  }
  const status: ManagedAgentStatus = {
    ...structuredClone(agent), state: "ready", backing: "managed", removable: true, version: "0.0.0-mock",
  };
  return Promise.resolve({ agentId, kind, outcome: kind === "repair" ? "repaired" : "installed", status });
}
