import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { AgentEvent, BridgeState, CapabilitySuggestion, CompletionCheckRun, CompletionSummary, Harness, Health, MarketplaceAction, MarketplaceActionResult, MarketplaceAppAuthState, MarketplaceCatalog, MarketplaceProvider, RouterPreferences, SanitizedTurn, SessionEntry, SessionForestSnapshot, SkillAction, SkillActionResult, SkillCatalog, SkillPreview, SkillProvider, SlashCommand, TerminalChunk, VerifierCandidate, VerifierManifest } from "./types";
import type { AccountUsagePayload } from "./usage";

const isTauri = () => typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
const now = new Date().toISOString();
const stateListeners = new Set<() => void>();
const mockRouterPreferences = new Map<string, RouterPreferences>();
const mockVerifierManifests = new Map<string, VerifierManifest>();
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
  return { id, sessionId, parentEntryId, sequence, semanticSchemaVersion: 2, kind, payload, providerEventId: null, contextVisibility: "eligible", tokenEstimate: null, createdAt: now };
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
    }
  }
};
function mockForest(sessionId: string): SessionForestSnapshot {
  const existing = mockForests[sessionId];
  if (existing) return structuredClone(existing);
  const session = mockState.sessions.find(item => item.id === sessionId);
  const entry = forestEntry(`${sessionId}-root`, sessionId, 1, "branch.summary", { summary: "Session started" }, null);
  const created: SessionForestSnapshot = { sessionId, entries: [entry], head: { sessionId, activeEntryId: entry.id, nativeProviderSessionId: session?.providerSessionId ?? null, restorationMode: session?.restorationMode ?? "fresh", resumeEligibility: session?.providerSessionId ? "native" : "none", latestCheckpointEntryId: null, updatedAt: now }, leaves: [entry], workerLeases: [], workerRuntimes: [], workerQueue: [], usage: [], reasons: [], policyLimits: { maxWorkersPerTurn: 3, maxStrongWorkersPerTurn: 1,maxCapabilityUnitsPerTurn: 24 }, repositoryDivergence: { status:"unknown", selectedState:null, currentState:{status:"unavailable"} }, completion: null };
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
    { id: "codex", label: "Codex", available: true, version: "mock", capabilities: ["messages", "streaming", "reasoning", "plans", "tools", "commands", "file_changes", "approvals", "usage", "history", "interrupt"], unavailableReason: null, models: [{ id: "gpt-5.6-luna", label: "GPT Luna", tier: "fast", defaultForTier: true }, { id: "gpt-5.6-terra", label: "GPT Terra", tier: "standard", defaultForTier: true }, { id: "gpt-5.6-sol", label: "GPT Sol", tier: "strong", defaultForTier: true }, { id: "gpt-5.3-codex", label: "GPT-5.3 Codex", tier: "standard", defaultForTier: false }], defaultModel: "gpt-5.6-luna" },
    { id: "claude", label: "Claude Code", available: true, version: "mock", capabilities: ["messages", "streaming", "reasoning", "tools", "commands", "approvals", "usage", "interrupt"], unavailableReason: null, models: [{ id: "sonnet", label: "Claude Sonnet", tier: "standard", defaultForTier: true }, { id: "opus", label: "Claude Opus", tier: "strong", defaultForTier: false }, { id: "haiku", label: "Claude Haiku", tier: "fast", defaultForTier: true }, { id: "fable", label: "Claude Fable", tier: "strong", defaultForTier: true }], defaultModel: "sonnet" }
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
    pinnedRef: "8b8c76004956f0e01e4f6c88ff6fb342258461f5", installs: 124000, official: true, compatibility: ["codex", "claude"], fileCount: 3,
    permissions: ["Read project files"], risk: "low", riskSummary: "Read-only project guidance.", categories: ["code-review", "react"],
    providerStates: [{ provider: "codex", installed: false, managed: false, installedRef: null, updateAvailable: false, rollbackAvailable: false }, { provider: "claude", installed: false, managed: false, installedRef: null, updateAvailable: false, rollbackAvailable: false }],
  }],
  personal: [{ id: "personal:my-workflow", name: "my-workflow", description: "A skill you maintain locally.", providers: ["codex"], source: "Personal skill" }],
};
const mockSkillConsents = new Map<string, { skillId: string; action: SkillAction; targets: SkillProvider[] }>();

export const bridgeApi = {
  skillCatalog: (): Promise<SkillCatalog> => isTauri() ? invoke("skill_catalog") : Promise.resolve(structuredClone(mockSkills)),
  skillSuggestions: (query: string, provider: SkillProvider): Promise<CapabilitySuggestion[]> => isTauri() ? invoke("skill_suggestions", { query, provider }) : Promise.resolve(mockSkills.community.filter(skill => skill.providerStates.some(state => state.provider === provider && state.installed) && `${skill.name} ${skill.description} ${skill.categories.join(" ")}`.toLowerCase().includes(query.toLowerCase())).map(skill => ({ id: skill.id, name: skill.name, command: skill.slug, relevance: `Matches “${query}”`, source: skill.source, providers: [provider], permissions: skill.permissions, risk: skill.risk, installed: true }))),
  previewSkillChange: async (skillId: string, action: SkillAction, targets: SkillProvider[]): Promise<SkillPreview> => {
    if (isTauri()) return invoke("preview_skill_change", { skillId, action, targets });
    const skill = mockSkills.community.find(item => item.id === skillId); if (!skill) throw new Error("Skill not found");
    const confirmationId = crypto.randomUUID(); mockSkillConsents.set(confirmationId, { skillId, action, targets });
    return { confirmationId, expiresAt: new Date(Date.now() + 300_000).toISOString(), action, skill: structuredClone(skill), targets, changes: targets.map(provider => `${action} ${skill.name} for ${provider}`), installer: mockSkills.installer };
  },
  executeSkillChange: async (confirmationId: string): Promise<SkillActionResult[]> => {
    if (isTauri()) return invoke("execute_skill_change", { confirmationId });
    const consent = mockSkillConsents.get(confirmationId); if (!consent) throw new Error("Confirmation is invalid or already used"); mockSkillConsents.delete(confirmationId);
    const skill = mockSkills.community.find(item => item.id === consent.skillId)!;
    for (const target of consent.targets) { const state = skill.providerStates.find(item => item.provider === target)!; state.installed = consent.action === "install"; state.managed = consent.action === "install"; state.installedRef = consent.action === "install" ? skill.pinnedRef : null; }
    return consent.targets.map(provider => ({ provider, action: consent.action, success: true, message: `${consent.action} completed`, error: null }));
  },
  marketplaceCatalog: (): Promise<MarketplaceCatalog> => isTauri() ? invoke("marketplace_catalog") : Promise.resolve(structuredClone(mockMarketplace)),
  marketplaceAppAuthStates: (): Promise<MarketplaceAppAuthState[]> => {
    if (isTauri()) return invoke("marketplace_app_auth_states");
    const variant = mockMarketplace.providers.find(item => item.provider === "codex")?.variants.find(item => item.appConnectorIds.includes("connector_vercel"));
    const authenticationState = variant?.authenticationState === "connected" ? "connected" : "required";
    return Promise.resolve([
      { provider: "codex", connectorId: "connector_vercel", displayName: null, nativeConnector: false, authenticationState },
      { provider: "claude", connectorId: "plugin:vercel:vercel", displayName: null, nativeConnector: false, authenticationState: "required" },
      { provider: "claude", connectorId: "claude.ai Notion", displayName: "Notion", nativeConnector: true, authenticationState: "connected" },
    ]);
  },
  marketplaceAction: async (provider: MarketplaceProvider, pluginId: string, marketplace: string | null, action: MarketplaceAction): Promise<MarketplaceActionResult> => {
    if (isTauri()) return invoke("marketplace_action", { provider, pluginId, marketplace, action });
    const entry = mockMarketplace.providers.find(item => item.provider === provider)?.variants.find(item => item.pluginId === pluginId);
    if (!entry) throw new Error(`${provider} plugin not found`);
    if (action === "install") entry.installed = true;
    if (action === "enable") entry.enabled = true;
    if (action === "disable") entry.enabled = false;
    if (action === "uninstall") { entry.installed = false; entry.enabled = false; entry.authenticationState = "unknown"; }
    if (action === "authenticate") entry.authenticationState = "connected";
    return { provider, pluginId, action, success: true, message: `${action} completed`, error: null };
  },
  health: (): Promise<Health> => isTauri() ? invoke("health") : Promise.resolve(structuredClone(mockHealth)),
  state: (): Promise<BridgeState> => isTauri() ? invoke("get_state") : Promise.resolve(snapshot()),
  routerPreferences: (workspaceId: string): Promise<RouterPreferences> => isTauri()
    ? invoke("get_router_preferences", { workspaceId })
    : Promise.resolve(structuredClone(mockRouterPreferences.get(workspaceId) ?? { mode: "shadow", minimumPassBps: 6500, pinnedHarness: null, pinnedModel: null, excludedHarnesses: [], excludedModels: [] })),
  updateRouterPreferences: (workspaceId: string, preferences: RouterPreferences): Promise<RouterPreferences> => {
    if (isTauri()) return invoke("update_router_preferences", { workspaceId, preferences });
    mockRouterPreferences.set(workspaceId, structuredClone(preferences));
    return Promise.resolve(structuredClone(preferences));
  },
  sessionForest: (sessionId: string): Promise<SessionForestSnapshot> => isTauri() ? invoke("get_session_forest", { sessionId }) : Promise.resolve(mockForest(sessionId)),
  createCompletionPlan: async (sessionId: string, acceptanceCriteria: string[], changedPaths: string[], repositoryCommands: string[], markdownProjection: string | null = null, markdownCommitted = false): Promise<CompletionSummary> => {
    if (isTauri()) return invoke("create_completion_plan", { sessionId, acceptanceCriteria, changedPaths, repositoryCommands, markdownProjection, markdownCommitted });
    const forest = mockForest(sessionId); if (!forest.completion) throw new Error("Mock completion plan is available only on the demo orchestrator"); return forest.completion;
  },
  recordCompletionCheck: async (attemptId: string, run: CompletionCheckRun): Promise<CompletionSummary> => {
    if (isTauri()) return invoke("record_completion_check", { attemptId, run });
    const forest = Object.values(mockForests).find(item => item.completion?.attemptId === attemptId); if (!forest?.completion) throw new Error("Completion attempt not found");
    const index = forest.completion.checks.findIndex(check => check.checkId === run.checkId); if (index < 0) throw new Error("Completion check not found"); forest.completion.checks[index] = structuredClone(run); forest.completion.passedRequired = forest.completion.checks.filter(check => check.required && check.status === "passed").length; return structuredClone(forest.completion);
  },
  waiveCompletion: async (attemptId: string, checkIds: string[], reason: string): Promise<CompletionSummary> => {
    if (isTauri()) return invoke("waive_completion", { attemptId, checkIds, reason });
    const forest = Object.values(mockForests).find(item => item.completion?.attemptId === attemptId); if (!forest?.completion) throw new Error("Completion attempt not found"); const unresolved = forest.completion.checks.filter(check => check.required && check.status !== "passed").map(check => check.checkId); if (!unresolved.every(checkId => checkIds.includes(checkId))) throw new Error("Waiver must cover every unresolved required check"); forest.completion.verdict = "waived"; forest.completion.waiverReason = reason; return structuredClone(forest.completion);
  },
  registerVerifierManifest: async (source: string, manifest: VerifierManifest): Promise<void> => {
    if (isTauri()) return invoke("register_verifier_manifest", { source, manifest });
    mockVerifierManifests.set(manifest.id, structuredClone(manifest));
  },
  verifierCandidates: async (changeLabels: string[], availableCapabilities: string[]): Promise<VerifierCandidate[]> => {
    if (isTauri()) return invoke("verifier_candidates", { changeLabels, availableCapabilities });
    return [...mockVerifierManifests.values()].map(manifest => {
      const triggerMatch = !manifest.triggers.length || manifest.triggers.some(trigger => changeLabels.includes(trigger));
      const missing = manifest.requiredCapabilities.filter(capability => !availableCapabilities.includes(capability));
      const exclusionReasons = [...(!triggerMatch ? ["change triggers do not match"] : []), ...(missing.length ? [`missing capabilities: ${missing.join(", ")}`] : [])];
      return { manifest: structuredClone(manifest), eligible: exclusionReasons.length === 0, exclusionReasons };
    });
  },
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
  createWorkspace: async (title: string): Promise<BridgeState> => {
    if (isTauri()) return invoke("create_workspace", { title });
    const id = crypto.randomUUID();
    mockState.workspaces.push({ id, projectId: null, city: null, title, branch: null, path: null, status: "idle", dirtyFiles: 0, additions: 0, deletions: 0, createdAt: new Date().toISOString() });
    emitState(); return snapshot();
  },
  createChat: async (harness: Harness, model: string | null, title: string | null): Promise<BridgeState> => {
    if (isTauri()) return invoke("create_chat", { harness, model, title });
    const id = crypto.randomUUID();
    mockState.sessions.push({ id, workspaceId: null, harness, label: title || "New chat", status: "idle", startedAt: null, endedAt: null, contextPercent: null, usagePercent: null, metricSource: "estimated", providerSessionId: null, activeTurnId: null, model, requestedTier: "fast", restorationMode: "fresh", title, kind: "direct", cwd: null }); emitState(); return snapshot();
  },
  createWorkspaceSession: async (workspaceId: string): Promise<BridgeState> => {
    if (isTauri()) return invoke("create_workspace_session", { workspaceId });
    const id = crypto.randomUUID();
    mockState.sessions.push({ id, workspaceId, harness: "codex", label: "Orchestrator", status: "idle", startedAt: null, endedAt: null, contextPercent: null, usagePercent: null, metricSource: "estimated", providerSessionId: null, activeTurnId: null, model: null, requestedTier: "fast", restorationMode: "fresh", title: null, kind: "orchestrator", cwd: null }); emitState(); return snapshot();
  },
  updateChatModel: async (sessionId: string, harness: Harness, model: string | null): Promise<BridgeState> => {
    if (isTauri()) return invoke("update_chat_model", { sessionId, harness, model });
    const session = mockState.sessions.find(item => item.id === sessionId); if (session) { session.harness = harness; session.model = model; session.status = "idle"; session.providerSessionId = null; }
    emitState(); return snapshot();
  },
  listSlashCommands: async (): Promise<SlashCommand[]> => {
    if (isTauri()) return invoke("list_slash_commands");
    return [];
  },
  resolveSlashCommand: async (sessionId: string, text: string): Promise<{ name: string; harness: Harness; kind: string; switchHarness: boolean } | null> => {
    if (isTauri()) return invoke("resolve_slash_command", { sessionId, text });
    return null;
  },
  connectWorkspaceFolder: async (workspaceId: string, path: string): Promise<BridgeState> => {
    if (isTauri()) return invoke("connect_workspace_folder", { workspaceId, path });
    const workspace = mockState.workspaces.find(item => item.id === workspaceId); if (workspace) { workspace.path = path; workspace.branch = "main"; }
    emitState(); return snapshot();
  },
  startChat: async (sessionId: string): Promise<BridgeState> => {
    if (isTauri()) return invoke("start_chat", { sessionId });
    const session = mockState.sessions.find(item => item.id === sessionId);
    if (session) { session.status = "working"; session.startedAt = new Date().toISOString(); session.endedAt = null; session.providerSessionId = session.providerSessionId ?? `mock-${crypto.randomUUID()}`; session.restorationMode = "fresh"; appendAgent(session.id, "session.started", { status: "working" }); }
    emitState(); return snapshot();
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
  prepareTurn: (sessionId: string, text: string): Promise<SanitizedTurn> => isTauri()
    ? invoke("prepare_turn", { sessionId, text })
    : Promise.resolve({ text, interceptions: [] }),
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
  refreshAccountUsage: (): Promise<void> => isTauri() ? invoke("refresh_account_usage") : Promise.resolve(),
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
  onAccountUsage: async (handler: (payload: AccountUsagePayload) => void): Promise<UnlistenFn> => {
    if (isTauri()) return listen<AccountUsagePayload>("account-usage", event => handler(event.payload));
    return () => undefined;
  },
  onStateChanged: async (handler: () => void): Promise<UnlistenFn> => {
    if (isTauri()) return listen("state-changed", handler); stateListeners.add(handler); return () => stateListeners.delete(handler);
  }
};
