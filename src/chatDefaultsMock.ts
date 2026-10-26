// Preview-only implementation. The shared core remains authoritative with a backend.
import type { AdapterDescriptor, AgentDefinition, BridgeState } from "./types";
import type { ChatDefaults, ChatDefaultsRecord, ChatDefaultsView, ChatSessionSettings, ResolveChatSettingsParams, ResolvedChatSettings, SaveChatDefaultsParams } from "./protocol/generated/protocol";
import { layerChatDefaults, sameDefaultsRevision, WORKER_EFFORTS } from "./chatDefaults";

const KEY = "bridge.mock.chatDefaults.v1";
function records(): Record<string, ChatDefaultsRecord> {
  const text = localStorage.getItem(KEY);
  if (!text) return {};
  return JSON.parse(text) as Record<string, ChatDefaultsRecord>;
}
function key(projectId?: string | null) { return projectId ? `project:${projectId}` : "application"; }
const empty = (): ChatDefaultsRecord => ({ revision: 0, defaults: {} });
export function mockDefaultsView(state: BridgeState, projectId?: string | null): ChatDefaultsView {
  if (projectId && !state.projects.some(p => p.id === projectId) && !state.workspaces.some(w => w.projectId === projectId)) throw new Error("Project no longer exists");
  const stored = records();
  return { application: stored.application ?? empty(), project: projectId ? stored[key(projectId)] ?? empty() : null };
}
export function mockSaveDefaults(state: BridgeState, params: SaveChatDefaultsParams): ChatDefaultsView {
  const current = mockDefaultsView(state, params.projectId);
  const record = params.projectId ? current.project! : current.application;
  if (record.revision !== params.expectedRevision) throw new Error("Defaults changed in another window. Reload before saving");
  const stored = records(); stored[key(params.projectId)] = { revision: record.revision + 1, defaults: structuredClone(params.defaults) };
  localStorage.setItem(KEY, JSON.stringify(stored));
  return mockDefaultsView(state, params.projectId);
}
export function mockResolveSettings(state: BridgeState, adapters: AdapterDescriptor[], agents: AgentDefinition[], params: ResolveChatSettingsParams): ResolvedChatSettings {
  const workspace = state.workspaces.find(w => w.id === params.workspaceId);
  if (params.workspaceId && !workspace) throw new Error("Workspace no longer exists");
  const stored = mockDefaultsView(state, workspace?.projectId);
  const revision = { application: stored.application.revision, project: stored.project?.revision ?? 0 };
  if (params.expectedRevision && !sameDefaultsRevision(revision,params.expectedRevision)) throw new Error("New chat defaults changed. Review the refreshed choices before sending");
  const sources: Record<string, string> = {};
  for (const [source, values] of [["application", stored.application.defaults], ["project", stored.project?.defaults ?? {}], ["chat", params.overrides]] as const) {
    for (const [field, value] of Object.entries(values)) if (value != null) sources[field] = source;
  }
  const values = layerChatDefaults(stored.application.defaults, stored.project?.defaults ?? {}, params.overrides);
  const target = values.target ?? { kind: workspace ? "orchestrator" as const : "direct" as const };
  if (!workspace && target.kind !== "direct") throw new Error("This default needs a project. Choose a project or Direct chat");
  const agent = target.kind === "agent" ? agents.find(a => a.id === target.agentId && a.enabled && a.role !== "orchestrator") : undefined;
  if (target.kind === "agent" && !agent) throw new Error("Selected agent was removed or disabled");
  const adapter = adapters.find(a => a.id === (values.model?.harness ?? (agent?.harness !== "bridge" ? agent?.harness : undefined)) && a.available) ?? (!values.model ? adapters.find(a => a.available) : undefined);
  if (!adapter) throw new Error("Selected harness is unavailable");
  const model = values.model ?? { harness: adapter.id, model: agent?.model ?? adapter.models.find(m => m.tier === "standard" && m.defaultForTier)?.id ?? adapter.defaultModel ?? null };
  const selected = adapter.models.find(m => m.id === model.model);
  if (model.model && (!selected || selected.available === false || selected.compatible === false)) throw new Error("Selected model is unavailable or incompatible");
  const effort = values.effort ? values.effort.value ?? null : !values.model && selected?.supportedEffortLevels?.includes(agent?.effort ?? "medium") ? agent?.effort ?? "medium" : null;
  if (effort && !selected?.supportedEffortLevels?.includes(effort)) throw new Error("Selected model does not support this thinking effort");
  if (agent && effort && !WORKER_EFFORTS.includes(effort)) throw new Error(`Specialist agents support low, medium, high or xhigh thinking effort, not ${effort}. Choose one of those or Provider default`);
  const access = values.access ?? "ask";
  if (adapter.sandboxModes?.length && !adapter.sandboxModes.includes(access === "ask" ? "workspace_write" : "danger_full_access")) throw new Error("This harness cannot enforce this access choice");
  if (values.createWorktree && !workspace?.projectId) throw new Error("Connect a Git project before choosing an isolated worktree");
  for (const field of ["target", "model", "effort", "access", "createWorktree", "activeTurnInput"]) sources[field] ??= "default";
  return { workspaceId: params.workspaceId, projectId: workspace?.projectId ?? null, revision, target, model, effort, access, createWorktree: values.createWorktree ?? false, activeTurnInput: values.activeTurnInput ?? "steer", sources };
}
export const mockSessionSettings = new Map<string, ChatSessionSettings>();
