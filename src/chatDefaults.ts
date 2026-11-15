import type { ChatDefaults, ChatDefaultsRevision, ResolvedChatSettings } from "./protocol/generated/protocol";
import type { AdapterDescriptor, HarnessConfig } from "./types";

/** Only reusable choices. No cwd, branch, browser, prompt, or credential state. */
export function defaultsFromSettings(settings: ResolvedChatSettings): ChatDefaults {
  return { target: settings.target, model: settings.model, effort: { value: settings.effort ?? null }, access: settings.access, createWorktree: settings.createWorktree, activeTurnInput: settings.activeTurnInput };
}

/** A specialist runs as a worker; the worker contract carries these levels only. */
export const WORKER_EFFORTS: readonly string[] = ["low", "medium", "high", "xhigh"];

const SOURCE_LABELS: Record<string, string> = { application: "application default", project: "project default", chat: "this chat", agent: "selected agent", default: "Bridge default" };
/** Host source keys (application, project, chat, agent, default) as user-facing words. */
export function settingSourceLabel(source: string | undefined): string {
  return source ? SOURCE_LABELS[source] ?? source : "this chat";
}

export function layerChatDefaults(...layers: ChatDefaults[]): ChatDefaults {
  const result: ChatDefaults = {};
  for (const layer of layers) for (const key of Object.keys(layer) as Array<keyof ChatDefaults>) {
    const value = layer[key];
    if (value != null) Object.assign(result, { [key]: value });
  }
  return result;
}

export function sameDefaultsRevision(a: ChatDefaultsRevision, b: ChatDefaultsRevision): boolean {
  return a.application === b.application && a.project === b.project;
}

type LaunchAdapter = Pick<AdapterDescriptor, "id" | "models" | "defaultModel">;
type LaunchHarnessConfig = Pick<HarnessConfig, "id" | "defaultModel" | "effort">;

/**
 * What a chat started outside the composer (a Notes task, a trail, the Storage
 * and Notes side chats) asks for once a harness is known. `defaults` is what
 * the composer would start on in the same scope.
 *
 * On the composer's harness it asks for nothing, so the chat inherits the
 * saved chat defaults, model and effort, exactly like a composer chat. On any
 * other harness it asks for that harness's own defaults (Settings, Coding
 * agents), then the provider's default model. It never names the provider's
 * default when the person has saved a choice: that is what made these chats
 * start on Sonnet while the composer said Opus.
 */
export function harnessLaunchOverrides(
  harness: string,
  defaults: Pick<ResolvedChatSettings, "model"> | null | undefined,
  adapters: readonly LaunchAdapter[],
  configs: readonly LaunchHarnessConfig[],
): Pick<ChatDefaults, "model" | "effort"> {
  if (defaults?.model.harness === harness) return {};
  const adapter = adapters.find(item => item.id === harness);
  const saved = configs.find(item => item.id === harness)?.defaultModel ?? null;
  // A saved model only counts while the provider still offers it. An empty
  // catalog means discovery has not landed: keep the saved id.
  const offered = saved && (!adapter?.models.length || adapter.models.some(option =>
    option.id === saved && option.available !== false && option.compatible !== false));
  const model = (offered ? saved : null) ?? adapter?.defaultModel ?? adapter?.models[0]?.id ?? null;
  const effort = configs.find(item => item.id === harness)?.effort ?? null;
  const supported = adapter?.models.find(option => option.id === model)?.supportedEffortLevels ?? [];
  return { model: { harness, model }, effort: { value: effort && supported.includes(effort) ? effort : null } };
}
