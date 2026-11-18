// The built-in apps: every screen that used to own a sidebar row.
//
// One table feeds the launcher, the sidebar pins and the Store, so a screen
// cannot be reachable from one and missing from another. Later slices load
// this from core as manifests; until then it is the frontend's own list.

import { AppWindow, Bot, ChartNoAxesColumn, FolderGit2, GitPullRequest, LayoutGrid, Library, NotebookPen, Route, Send, Store, TerminalSquare, Workflow, type LucideIcon } from "lucide-react";
import type { AppView } from "../navigationHistory";
import type { PersonalAppSummary } from "../protocol/generated/protocol";

export type AppId =
  | "notes"
  | "trails"
  | "pulls"
  | "workflows"
  | "memory"
  | "mission-control"
  | "terminals"
  | "usage"
  | "projects"
  | "agents"
  | "store";

/** Where an app shows up besides the launcher. Only what exists today. */
export type AppSlot = "screen" | "dock";

export type BuiltinApp = {
  id: AppId;
  name: string;
  description: string;
  icon: LucideIcon;
  view: AppView;
  slots: readonly AppSlot[];
  /** Extra words the launcher matches, beyond the name. */
  keywords: readonly string[];
};

export const BUILTIN_APPS: readonly BuiltinApp[] = [
  { id: "notes", name: "Notes", description: "Pages, tasks and quick captures", icon: NotebookPen, view: "notes", slots: ["screen"], keywords: ["pages", "tasks", "capture", "write"] },
  { id: "trails", name: "Trails", description: "One page per piece of work: its chats, notes, PRs and drafts", icon: Route, view: "trails", slots: ["screen"], keywords: ["work", "issue", "plan", "continuity"] },
  { id: "pulls", name: "Pulls", description: "Pull requests across your repositories", icon: GitPullRequest, view: "pulls", slots: ["screen", "dock"], keywords: ["github", "pr", "review", "source control"] },
  { id: "workflows", name: "Workflows", description: "Runs that repeat on a schedule", icon: Workflow, view: "workflows", slots: ["screen"], keywords: ["automation", "schedule", "runs"] },
  { id: "memory", name: "Memory", description: "What Bridge remembers about you", icon: Library, view: "memory", slots: ["screen"], keywords: ["remember", "proposals", "facts"] },
  { id: "mission-control", name: "Mission Control", description: "Every agent on one board", icon: LayoutGrid, view: "mission-control", slots: ["screen"], keywords: ["board", "grid", "overview"] },
  { id: "terminals", name: "Terminals", description: "A shell for each stream of work", icon: TerminalSquare, view: "agent-fleet", slots: ["screen", "dock"], keywords: ["shell", "fleet", "console"] },
  { id: "usage", name: "Usage", description: "Spend and limits per provider", icon: ChartNoAxesColumn, view: "usage", slots: ["screen"], keywords: ["cost", "tokens", "limits", "quota"] },
  { id: "projects", name: "Projects", description: "Your repositories and folders", icon: FolderGit2, view: "projects", slots: ["screen"], keywords: ["repositories", "folders", "workspaces"] },
  { id: "agents", name: "Agents", description: "Your own agents and Bridge's built-in ones", icon: Bot, view: "saved-setups", slots: ["screen"], keywords: ["setups", "orchestrator", "presets"] },
  { id: "store", name: "Store", description: "Apps, plugins and skills", icon: Store, view: "marketplace", slots: ["screen"], keywords: ["marketplace", "plugins", "skills", "install"] },
];

/** Apps core lists that have no screen of their own: they show up only as
 *  cards in a chat, so the launcher has nothing to open. The Store still
 *  lists them, to turn them on or off. */
export const CARD_APPS: readonly { id: string; name: string; description: string; icon: LucideIcon }[] = [
  { id: "outgoing", name: "Outgoing", description: "Messages a chat drafts for you to send", icon: Send },
];

/** Screens that are not apps: the chat itself, its board, settings pages
 *  reached from inside other screens, and the pane each of the person's own
 *  apps opens in (reached from its own launcher entry). Anything else must be an app, or the
 *  launcher would leave it unreachable. */
export const NON_APP_VIEWS: readonly AppView[] = ["workspace", "work", "settings", "archives", "briefing-settings", "personal-app"];

export const SLOT_LABELS: Record<AppSlot, string> = { screen: "Full screen", dock: "Dock" };

const BY_ID = new Map(BUILTIN_APPS.map(app => [app.id, app]));

export function isAppId(value: unknown): value is AppId {
  return typeof value === "string" && BY_ID.has(value as AppId);
}

export function appById(id: AppId): BuiltinApp {
  return BY_ID.get(id)!;
}

/** The app that owns what is on screen. The workspace in its grid paradigm is
 *  Mission Control drawn in place, so it counts as that app. */
export function appForView(view: AppView, paradigm?: "single" | "grid"): AppId | null {
  if (view === "workspace") return paradigm === "grid" ? "mission-control" : null;
  return BUILTIN_APPS.find(app => app.view === view)?.id ?? null;
}

/** A count of things waiting on the person. `capped` means the source list
 *  stopped at its limit, so the true number may be higher. */
export type AppBadgeCount = { count: number; capped?: boolean };
export type AppBadges = Partial<Record<AppId, AppBadgeCount>>;
export type BadgeText = { count: string; label: string };

export function badgeText(badge: AppBadgeCount | undefined): BadgeText | undefined {
  if (!badge || badge.count <= 0) return undefined;
  const count = `${badge.count}${badge.capped ? "+" : ""}`;
  return { count, label: `${count} waiting for you` };
}

/** The Apps row's one badge: everything every app is waiting on, summed. */
export function appsBadge(badges: AppBadges): BadgeText | undefined {
  let count = 0;
  let capped = false;
  for (const badge of Object.values(badges)) {
    if (!badge || badge.count <= 0) continue;
    count += badge.count;
    capped ||= !!badge.capped;
  }
  return badgeText({ count, capped });
}

// Personal apps --------------------------------------------------------------
//
// The person's own locked apps ("My apps") share the launcher, the sidebar
// pins and navigation with the built-ins. Core keys both by id, and the two
// never collide; the client prefixes personal ids so one key names either.

export type PersonalKey = `personal:${string}`;
/** A built-in app or one of the person's own. */
export type AppKey = AppId | PersonalKey;

export function personalKey(appId: string): PersonalKey {
  return `personal:${appId}`;
}

/** The personal app id a key names, or null for a built-in. */
export function personalIdOf(key: AppKey): string | null {
  return key.startsWith("personal:") ? key.slice("personal:".length) : null;
}

/** The id core knows the app by. */
export function coreIdOf(key: AppKey): string {
  return personalIdOf(key) ?? key;
}

/** The key for an id core reported, or null when this client knows no such
 *  app. `personal` is the set of the person's app ids. */
export function keyOfCoreId(id: string, personal: ReadonlySet<string>): AppKey | null {
  if (isAppId(id)) return id;
  return personal.has(id) ? personalKey(id) : null;
}

/** What the launcher and the sidebar draw for any app. */
export type LaunchEntry = {
  key: AppKey;
  name: string;
  description: string;
  icon: LucideIcon;
  keywords: readonly string[];
};

export function builtinEntry(app: BuiltinApp): LaunchEntry {
  return { key: app.id, name: app.name, description: app.description, icon: app.icon, keywords: app.keywords };
}

export function personalEntry(app: Pick<PersonalAppSummary, "id" | "name" | "intent" | "server">): LaunchEntry {
  return { key: personalKey(app.id), name: app.name, description: app.intent, icon: AppWindow, keywords: [app.server.toLowerCase(), "my apps"] };
}

/** Case-insensitive match on the name and the keywords. An empty query
 *  matches everything. */
export function appMatches(app: Pick<BuiltinApp, "name" | "keywords">, query: string): boolean {
  const needle = query.trim().toLowerCase();
  if (!needle) return true;
  return app.name.toLowerCase().includes(needle) || app.keywords.some(word => word.includes(needle));
}
