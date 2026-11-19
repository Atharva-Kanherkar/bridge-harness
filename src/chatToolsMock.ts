// Preview-only implementation. The shared core remains authoritative with a backend.
// The texts, groups and sources copy the host, so a preview shows what a person sees.
import type { BridgeState } from "./types";
import type { ChatToolItem, ChatToolSelection, ChatToolSelectionStatus, ChatToolSource, ChatToolSourceKind, GetChatToolsParams, GetChatToolsResult, UpdateChatToolsParams, UpdateChatToolsResult } from "./protocol/generated/protocol";
import { normalizeAccount } from "./providerAccounts";

type Session = BridgeState["sessions"][number];
type Choice = Required<ChatToolSelection>;
interface Row { revision: number; choice: Choice }
interface Listed { items: ChatToolItem[]; sources: ChatToolSource[] }

export const MOCK_COLD_NOTE = "Available after the first turn";
const BRIDGE_REASON = "Bridge owns this server. It stays as attached today.";
const CLAUDE_PLUGIN_REASON = "A plugin is not a unit. Choose its servers and skills instead.";
const CODEX_APP_REASON = "Codex has no verified switch for one app. The Apps switch turns all apps on or off.";
const CODEX_SKILLS_UNKNOWN_REASON = "Bridge can check the Codex config for a skills key only after a turn of this chat.";
const OPENCODE_SKILLS_REASON = "OpenCode has no verified switch for one skill.";
const NO_UNITS_REASON = "This harness has no tool choice";
const DONOR_NOTE = "The newest chat of this account that started. A new chat can list other items";
const STALE = "The tool choice changed in another window. Reload it and try again";
const TURN_BUSY = "A turn of this chat runs now. Try again when it ends.";
const BUSY: Record<string, string> = {
  working: TURN_BUSY,
  checkpointing: "This chat makes a checkpoint now. Try again when it is done.",
  waiting: "This chat waits for your answer. Answer it, then try again.",
  starting: "This chat starts now. Try again when it is ready.",
};
const STATUS_BUSY = "This chat is busy. Try again when it is idle.";
const SAVE_STATUSES = ["ready", "idle", "stopped", "failed"];
const CLAUDE_FIGURES = "claude.getContextUsage";
const CODEX_SERVER_FIGURES = "codex.mcpServerStatus bytes_div4_v1";
const CODEX_SKILL_FIGURES = "codex.skills/list bytes_div4_v1";

// The sample world of the preview: what each warm harness lists, with a token figure where the harness gives one.
const CLAUDE_CATALOG: Record<string, string> = { "claude.ai Gmail": "Gmail", "claude.ai Google Calendar": "Google Calendar" };
const CLAUDE_SERVERS: [string, number][] = [["bridge", 1210], ["claude.ai Gmail", 2140], ["claude.ai Google Calendar", 1630], ["chrome-devtools", 5320], ["plugin:vercel-plugin:vercel", 880]];
const CLAUDE_SKILLS: [string, number][] = [["review", 410], ["simplify", 220], ["superpowers:brainstorming", 640], ["superpowers:writing-plans", 530]];
const CLAUDE_PLUGINS = ["superpowers"];
const CODEX_SERVERS: [string, number][] = [["bridge", 980], ["github", 4120], ["linear", 2310]];
const CODEX_APPS: [number, string[]] = [3560, ["Gmail", "Notion"]];
const CODEX_SKILLS: [string, number][] = [["imagegen", 140], ["pdf", 180], ["playwright", 260]];
const OPENCODE_SERVERS = ["bridge", "context7", "sentry"];
// A Claude chat outside the mock state. A draft on the own login borrows its init when no mock Claude chat started.
const SAMPLE_DONOR = "Plan the release checklist";

const rows = new Map<string, Row>();
const minutesAgo = (minutes: number) => new Date(Date.now() - minutes * 60_000).toISOString();
const harnessName = (harness: string) => ({ claude: "Claude Code", codex: "Codex", opencode: "OpenCode" } as Record<string, string>)[harness] ?? harness;
const describe = (harness: string, account: string | null) => `${harnessName(harness)} (${account ? `account ${account}` : "own login"})`;
const isBridge = (id: string) => id.trim().toLowerCase() === "bridge";

const unit = (kind: ChatToolItem["kind"], id: string, group: string, label = id): ChatToolItem => ({ kind, id, label, group, state: "on", fixedReason: null, seen: true, tokens: null });
const fixed = (item: ChatToolItem, reason: string): ChatToolItem => ({ ...item, state: "fixed", fixedReason: reason });
const switched = (item: ChatToolItem, on: boolean): ChatToolItem => ({ ...item, state: on ? "on" : "off" });
const unseen = (item: ChatToolItem): ChatToolItem => ({ ...item, seen: false });
const serverState = (item: ChatToolItem, off: Set<string>) => isBridge(item.id) ? fixed(item, BRIDGE_REASON) : switched(item, !off.has(item.id));
const figure = (value: number, source: string, at: string) => ({ value, source, at });
const source = (kind: ChatToolSourceKind, label: string, at: string, note: string | null = null): ChatToolSource => ({ kind, label, at, available: true, note });
const missing = (kind: ChatToolSourceKind, label: string, note = MOCK_COLD_NOTE): ChatToolSource => ({ kind, label, at: null, available: false, note });

function savedServers(off: Set<string>, listed: Set<string>, group: (name: string) => string) {
  return [...off].filter(name => !listed.has(name)).sort().map(name => unseen(switched(unit("server", name, group(name)), false)));
}

function claudeServerGroup(name: string) {
  if (isBridge(name)) return "Bridge";
  if (name.startsWith("claude.ai ")) return "claude.ai";
  const plugin = name.startsWith("plugin:") ? name.slice(7).split(":") : [];
  return plugin.length > 1 ? plugin[0] : "MCP servers";
}
const claudeSkillGroup = (name: string) => name.includes(":") ? name.slice(0, name.indexOf(":")) : "Skills";

interface ClaudeInit { label: string; at: string; note: string | null; figuresAt: string }

function claudeList(init: ClaudeInit | null, row: Choice | null): Listed {
  const off = new Set(row?.serversOff ?? []);
  const only = row?.claudeSkills ? new Set(row.claudeSkills.only) : null;
  const tokens = (value: number) => init ? figure(value, CLAUDE_FIGURES, init.figuresAt) : null;
  const servers: [string, number][] = init ? CLAUDE_SERVERS : Object.keys(CLAUDE_CATALOG).map(name => [name, 0]);
  const items = servers.map(([name, value]) => serverState({ ...unit("server", name, claudeServerGroup(name), CLAUDE_CATALOG[name] ?? name), tokens: tokens(value) }, off));
  items.push(...savedServers(off, new Set(servers.map(([name]) => name)), claudeServerGroup));
  const skills: [string, number][] = init ? CLAUDE_SKILLS : [];
  items.push(...skills.map(([name, value]) => switched({ ...unit("skill", name, claudeSkillGroup(name)), tokens: tokens(value) }, !only || only.has(name))));
  items.push(...[...only ?? []].filter(name => !skills.some(([listed]) => listed === name)).map(name => unseen(unit("skill", name, claudeSkillGroup(name)))));
  items.push(...(init ? CLAUDE_PLUGINS : []).map(name => fixed(unit("plugin", name, "Plugins"), CLAUDE_PLUGIN_REASON)));
  const sources = [
    source("catalog", "Connector catalog", minutesAgo(12)),
    init ? source("claudeInit", init.label, init.at, init.note) : missing("claudeInit", "Claude init"),
    init ? source("contextReading", CLAUDE_FIGURES, init.figuresAt) : missing("contextReading", CLAUDE_FIGURES),
  ];
  return { items, sources };
}

function codexList(warm: boolean, row: Choice | null): Listed {
  const at = new Date().toISOString();
  const off = new Set(row?.serversOff ?? []);
  const servers = warm ? CODEX_SERVERS : [];
  const items = servers.map(([name, value]) => serverState({ ...unit("server", name, "MCP servers"), tokens: figure(value, CODEX_SERVER_FIGURES, at) }, off));
  items.push(...savedServers(off, new Set(servers.map(([name]) => name)), () => "MCP servers"));
  items.push(switched({ ...unit("apps", "codex_apps", "Apps", "Apps"), tokens: warm ? figure(CODEX_APPS[0], CODEX_SERVER_FIGURES, at) : null }, !row?.codexAppsOff));
  items.push(...(warm ? CODEX_APPS[1] : []).map(app => fixed(unit("app", app, "Apps"), CODEX_APP_REASON)));
  const skillsOff = new Set(row?.codexSkillsOff ?? []);
  const skills = warm ? CODEX_SKILLS : [];
  items.push(...skills.map(([name, value]) => switched({ ...unit("skill", name, "Skills"), tokens: figure(value, CODEX_SKILL_FIGURES, at) }, !skillsOff.has(name))));
  // A cold chat has no config read, so the host cannot tell whether a skill switch works.
  items.push(...[...skillsOff].filter(name => !skills.some(([listed]) => listed === name)).map(name => {
    const item = unseen(switched(unit("skill", name, "Skills"), false));
    return warm ? item : fixed(item, CODEX_SKILLS_UNKNOWN_REASON);
  }));
  const sources = warm
    ? ["Codex mcpServerStatus/list", "Codex skills/list", "Codex config/read"].map(label => source("runtime", label, at))
    : [missing("runtime", "Codex app-server")];
  return { items, sources };
}

function opencodeList(warm: boolean, row: Choice | null): Listed {
  const off = new Set(row?.serversOff ?? []);
  const servers = warm ? OPENCODE_SERVERS : [];
  const items = servers.map(name => serverState(unit("server", name, "MCP servers"), off));
  items.push(...savedServers(off, new Set(servers), () => "MCP servers"));
  items.push(fixed(unit("allSkills", "skills", "Skills", "Skills"), OPENCODE_SKILLS_REASON));
  return { items, sources: [warm ? source("runtime", "OpenCode MCP list", new Date().toISOString()) : missing("runtime", "OpenCode server")] };
}

const hasInit = (session: Session) => Boolean(session.providerSessionId) && !session.endedAt;
const topLevel = (session: Session) => !session.parentSessionId && session.kind !== "worker";

/** The newest top-level Claude chat of the account that started, the same workspace first. */
function donor(state: BridgeState, account: string | null, workspaceId: string | null, skip: string | null): ClaudeInit | null {
  const candidates = state.sessions
    .filter(session => session.harness === "claude" && topLevel(session) && session.id !== skip && hasInit(session) && normalizeAccount("claude", session.account) === account)
    .reverse()
    .sort((a, b) => Number(b.workspaceId === workspaceId) - Number(a.workspaceId === workspaceId));
  const chosen = candidates[0];
  const title = chosen ? chosen.title || chosen.label : account ? null : SAMPLE_DONOR;
  return title ? { label: `Claude init of ${title}`, at: chosen?.startedAt ?? minutesAgo(95), note: DONOR_NOTE, figuresAt: minutesAgo(80) } : null;
}

function list(harness: string, row: Choice | null, warm: boolean, claude: () => ClaudeInit | null): Listed {
  if (harness === "claude") return claudeList(claude(), row);
  if (harness === "codex") return codexList(warm, row);
  if (harness === "opencode") return opencodeList(warm, row);
  return { items: [], sources: [] };
}

function status(harness: string, account: string | null, row: Row | undefined): ChatToolSelectionStatus {
  const own: ChatToolSelectionStatus = { revision: row?.revision ?? 0, harness: row?.choice.harness ?? harness, account: row ? row.choice.account : account, applies: true, notAppliedReason: null };
  if (row && (row.choice.harness !== harness || row.choice.account !== account)) return { ...own, applies: false, notAppliedReason: `Chosen for ${describe(row.choice.harness, row.choice.account)}, not applied` };
  if (!["claude", "codex", "opencode"].includes(harness)) return { ...own, applies: false, notAppliedReason: NO_UNITS_REASON };
  return own;
}

function chat(state: BridgeState, sessionId: string): Session {
  const session = state.sessions.find(candidate => candidate.id === sessionId);
  if (!session) throw new Error("This chat does not exist");
  if (!topLevel(session)) throw new Error("Only a top-level chat can have a tool choice");
  return session;
}

export function mockChatTools(state: BridgeState, params: GetChatToolsParams): GetChatToolsResult {
  if (Boolean(params.sessionId) === Boolean(params.draft)) throw new Error("Give exactly one of sessionId and draft");
  if (params.draft) {
    const { harness, workspaceId = null } = params.draft;
    const account = normalizeAccount(harness, params.draft.account);
    const listed = list(harness, null, false, () => donor(state, account, workspaceId, null));
    return { harness, account, ...listed, choice: null, selection: status(harness, account, undefined) };
  }
  const session = chat(state, params.sessionId!);
  const account = normalizeAccount(session.harness, session.account);
  const row = rows.get(session.id);
  const own = (): ClaudeInit | null => hasInit(session)
    ? { label: "Claude init of this chat", at: session.startedAt ?? minutesAgo(30), note: null, figuresAt: minutesAgo(3) }
    : donor(state, account, session.workspaceId ?? null, session.id);
  const listed = list(session.harness, row?.choice.harness === session.harness ? row.choice : null, hasInit(session), own);
  return structuredClone({ harness: session.harness, account, ...listed, choice: row?.choice ?? null, selection: status(session.harness, account, row) });
}

/** The host check of a choice, with the account in its stored form. */
function check(choice: ChatToolSelection): Choice {
  const { harness } = choice;
  if (!["claude", "codex", "opencode"].includes(harness)) throw new Error(`A tool choice cannot target the harness "${harness}"`);
  const checked: Choice = { harness, account: normalizeAccount(harness, choice.account), serversOff: [...choice.serversOff ?? []], codexAppsOff: choice.codexAppsOff ?? false, claudeSkills: choice.claudeSkills ? { only: [...choice.claudeSkills.only] } : null, codexSkillsOff: [...choice.codexSkillsOff ?? []] };
  if (checked.serversOff.some(isBridge)) throw new Error("The Bridge server stays as attached today. A tool choice cannot leave it out");
  if (harness === "claude" && (checked.codexAppsOff || checked.codexSkillsOff.length)) throw new Error("A Claude tool choice cannot hold Codex items");
  if (harness === "codex" && checked.claudeSkills) throw new Error("A Codex tool choice cannot hold Claude skills");
  if (harness === "codex" && checked.serversOff.includes("codex_apps")) throw new Error("Use the Apps switch to leave out codex_apps");
  if (harness === "opencode" && (checked.claudeSkills || checked.codexAppsOff || checked.codexSkillsOff.length)) throw new Error("An OpenCode tool choice can leave out servers only");
  return checked;
}

export function mockUpdateChatTools(state: BridgeState, params: UpdateChatToolsParams): UpdateChatToolsResult {
  const session = chat(state, params.sessionId);
  const choice = check(params.selection);
  if (!SAVE_STATUSES.includes(session.status)) return { revision: null, refusal: BUSY[session.status] ?? STATUS_BUSY };
  if (session.activeTurnId) return { revision: null, refusal: TURN_BUSY };
  const revision = rows.get(session.id)?.revision ?? 0;
  if (revision !== params.expectedRevision) throw new Error(STALE);
  rows.set(session.id, { revision: revision + 1, choice });
  return { revision: revision + 1, refusal: null };
}

/** The first choice of a new chat. A choice for another harness or account throws before the chat exists. */
export function mockSaveFirstToolChoice(sessionId: string, model: { harness: string; account?: string | null }, selection: ChatToolSelection) {
  const choice = check(selection);
  const account = normalizeAccount(model.harness, model.account);
  if (choice.harness !== model.harness || choice.account !== account) {
    throw new Error(`The tool choice was made for ${describe(choice.harness, choice.account)}, but the chat starts on ${describe(model.harness, account)}. Review its tools and try again`);
  }
  rows.set(sessionId, { revision: 1, choice });
}
