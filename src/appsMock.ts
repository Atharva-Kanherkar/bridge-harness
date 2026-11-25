// Mock-mode apps: the same thirteen built-ins core lists, with their on/off
// state and pins kept in localStorage so `bun run dev` behaves like a daemon
// across reloads. The refusals use core's exact words.

import type { AppBadge, AppManifest, AppSlot, AppSummary, ListAppsResult, ListPersonalAppsResult, PersonalAppSummary } from "./protocol/generated/protocol";

const STATE_KEY = "bridge.apps.mock.v1";
const MAX_PINS = 3;

type Spec = { id: string; name: string; description: string; slots: AppSlot[]; tools?: [string, boolean][]; events?: string[]; memory?: string[]; canDisable?: boolean };

// Mirrors `bridge-core/src/apps/builtin.rs`. `appsMock.test.ts` keeps the ids
// in step with the frontend registry.
const SPECS: Spec[] = [
  { id: "notes", name: "Notes", description: "Pages, tasks and quick captures", slots: ["screen"], tools: [["notes_search", false], ["notes_read", false], ["notes_create", true], ["notes_edit", true], ["notes_append", true], ["notes_set", true], ["notes_draw", true]] },
  { id: "trails", name: "Trails", description: "One page per piece of work: its chats, notes, PRs and drafts", slots: ["screen"] },
  { id: "pulls", name: "Pulls", description: "Pull requests across your repositories", slots: ["screen", "dock"] },
  { id: "reviews", name: "Reviews", description: "Every PR review, what needs you, and how each reviewer does", slots: ["screen"] },
  { id: "workflows", name: "Workflows", description: "Runs that repeat on a schedule", slots: ["screen"] },
  { id: "memory", name: "Memory", description: "What Bridge remembers about you", slots: ["screen"], tools: [["memory_search", false], ["memory_propose", false]] },
  { id: "mission-control", name: "Mission Control", description: "Every agent on one board", slots: ["screen"] },
  { id: "terminals", name: "Terminals", description: "A shell for each stream of work", slots: ["screen", "dock"] },
  { id: "usage", name: "Usage", description: "Spend and limits per provider", slots: ["screen"] },
  { id: "projects", name: "Projects", description: "Your repositories and folders", slots: ["screen"] },
  { id: "agents", name: "Agents", description: "Your own agents and Bridge's built-in ones", slots: ["screen"] },
  { id: "store", name: "Store", description: "Apps, plugins and skills", slots: ["screen"], canDisable: false },
  { id: "outgoing", name: "Outgoing", description: "Messages a chat drafts for you to send", slots: ["card"], tools: [["outgoing_draft", false]], events: ["outgoing.sent"], memory: ["the people, channels and repositories the person writes to"] },
];

export const MOCK_APP_IDS: readonly string[] = SPECS.map(spec => spec.id);

/** The demo's one locked personal app ("My apps"), until it is deleted. */
export const MOCK_PERSONAL_APP: PersonalAppSummary = {
  id: "slack-dm", version: 1, name: "Slack DM", intent: "Find and message a teammate without leaving the chat",
  harness: "claude", server: "claude.ai Slack", reads: ["slack_search_users"], writes: ["slack_send_message"],
  contentHash: `sha256:${"a".repeat(64)}`, viewBytes: 0, lockedAt: "2026-11-18T09:00:00Z",
};

function personalApps(): PersonalAppSummary[] {
  return state().deleted ? [] : [MOCK_PERSONAL_APP];
}

function manifest(spec: Spec): AppManifest {
  return {
    id: spec.id,
    name: spec.name,
    description: spec.description,
    version: "0.0.0-mock",
    source: "builtin",
    views: spec.slots.map(slot => ({ slot })),
    tools: (spec.tools ?? []).map(([name, asksFirst]) => ({ name, asksFirst })),
    events: spec.events ?? [],
    memory: spec.memory ?? [],
  };
}

type State = { off: string[]; pins: string[]; deleted?: boolean };

function read(): State {
  try {
    const parsed = JSON.parse(localStorage.getItem(STATE_KEY) ?? "null") as Partial<State> | null;
    if (parsed && Array.isArray(parsed.off) && Array.isArray(parsed.pins)) return { off: parsed.off, pins: parsed.pins, deleted: parsed.deleted === true };
  } catch {
    // Storage blocked or corrupt: start fresh.
  }
  return { off: [], pins: [] };
}

let memory: State | null = null;
function state(): State {
  if (typeof localStorage === "undefined") return (memory ??= { off: [], pins: [] });
  return read();
}
function write(next: State) {
  if (typeof localStorage === "undefined") { memory = next; return; }
  try { localStorage.setItem(STATE_KEY, JSON.stringify(next)); } catch { memory = next; }
}

const listeners = new Set<() => void>();
function emit() {
  for (const listener of [...listeners]) listener();
}

/** Badge counts the caller computes from the other mocks, by app id. */
export type MockBadgeCounts = Partial<Record<"workflows" | "memory" | "trails", number>>;

export function mockListApps(counts: MockBadgeCounts = {}): ListAppsResult {
  const { off, pins } = state();
  return {
    apps: SPECS.map(spec => {
      const enabled = !off.includes(spec.id);
      const count = counts[spec.id as keyof MockBadgeCounts];
      const badge: AppBadge | null = enabled && count !== undefined ? { count } : null;
      const position = pins.indexOf(spec.id);
      return { manifest: manifest(spec), enabled, canDisable: spec.canDisable ?? true, pinned: position >= 0 ? position : null, badge } satisfies AppSummary;
    }),
    personal: personalApps().map(app => {
      const position = pins.indexOf(app.id);
      return { app, enabled: !off.includes(app.id), pinned: position >= 0 ? position : null };
    }),
  };
}

export function mockDeletePersonalApp(id: string): ListPersonalAppsResult {
  if (id === MOCK_PERSONAL_APP.id) {
    const current = state();
    write({ off: current.off.filter(item => item !== id), pins: current.pins.filter(pin => pin !== id), deleted: true });
    emit();
  }
  return { apps: personalApps() };
}

function spec(id: string): Spec {
  const personal = personalApps().find(app => app.id === id);
  if (personal) return { id, name: personal.name, description: personal.intent, slots: ["screen"] };
  const found = SPECS.find(item => item.id === id);
  if (!found) throw new Error(`Bridge has no app called ${id}.`);
  return found;
}

export function mockSetAppPinned(id: string, pinned: boolean, counts?: MockBadgeCounts): ListAppsResult {
  const app = spec(id);
  const current = state();
  let pins = current.pins.filter(pin => SPECS.some(item => item.id === pin) || personalApps().some(app => app.id === pin));
  if (pinned && !pins.includes(id)) {
    if (!app.slots.includes("screen") && !app.slots.includes("dock")) throw new Error(`${app.name} has no screen to pin to the sidebar.`);
    if (current.off.includes(id)) throw new Error(`Turn ${app.name} on before pinning it.`);
    if (pins.length >= MAX_PINS) throw new Error(`You can pin ${MAX_PINS} apps. Unpin one first.`);
    pins = [...pins, id];
  } else if (!pinned) {
    pins = pins.filter(pin => pin !== id);
  }
  if (pins.join() !== current.pins.join()) {
    write({ ...current, pins });
    emit();
  }
  return mockListApps(counts);
}

export function mockSetAppEnabled(id: string, enabled: boolean, counts?: MockBadgeCounts): ListAppsResult {
  const app = spec(id);
  if (!enabled && app.canDisable === false) throw new Error(`The ${app.name} cannot be turned off.`);
  const current = state();
  const off = enabled ? current.off.filter(item => item !== id) : current.off.includes(id) ? current.off : [...current.off, id];
  const pins = enabled ? current.pins : current.pins.filter(pin => pin !== id);
  if (off.join() !== current.off.join() || pins.join() !== current.pins.join()) {
    write({ ...current, off, pins });
    emit();
  }
  return mockListApps(counts);
}

export function onMockAppsChanged(handler: () => void): () => void {
  listeners.add(handler);
  return () => { listeners.delete(handler); };
}

/** Tests only: fire the hint as core would after a badge source moved. */
export function emitMockAppsChanged() {
  emit();
}

/** Tests only. */
export function resetMockApps() {
  memory = null;
  if (typeof localStorage !== "undefined") localStorage.removeItem(STATE_KEY);
}
