import type { ChatDefaults, ChatDefaultsRevision, ResolvedChatSettings } from "./protocol/generated/protocol";

/** Only reusable choices. No cwd, branch, browser, prompt, or credential state. */
export function defaultsFromSettings(settings: ResolvedChatSettings): ChatDefaults {
  return { target: settings.target, model: settings.model, effort: { value: settings.effort ?? null }, access: settings.access, createWorktree: settings.createWorktree, activeTurnInput: settings.activeTurnInput };
}

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
