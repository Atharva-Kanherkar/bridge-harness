// The one registry of settings pages. The rail, the narrow-window section
// picker, page titles and the search index all read it, so a page cannot be
// reachable from one and missing from another.

/** A page the rail can land on. */
export type PageId =
  | "account" | "general" | "notifications" | "voice" | "composer" | "menuBar"
  | "chatDefaults" | "codingAgents" | "models" | "agents" | "permissions" | "keys"
  | "import" | "archives" | "data"
  | "workers" | "prompts" | "work"
  | "updates";

/** Retired ids, kept so existing deep links still open the right page. */
export type LegacySection = "appearance" | "storage" | "harnesses" | "history";

export type Section = PageId | LegacySection;

export type RailGroupId = "basics" | "agents" | "data" | "advanced";

export type PageInfo = { label: string; description: string; group: RailGroupId | "footer" };

export const PAGES: Record<PageId, PageInfo> = {
  account: { label: "Account", description: "Sign in to Bridge. Optional, and separate from your coding agents' own accounts.", group: "basics" },
  general: { label: "General", description: "How Bridge looks, its zoom level, and what a conversation shows.", group: "basics" },
  notifications: { label: "Notifications", description: "Toasts, banners and sounds for each kind of notice, and quiet hours.", group: "basics" },
  voice: { label: "Voice", description: "Dictate, review, then send. Private English speech recognition on your Mac.", group: "basics" },
  composer: { label: "Typing & search", description: "Text suggestions while you type, and how Bridge searches chat history.", group: "basics" },
  menuBar: { label: "Menu bar", description: "Account limits, tokens, and spend at a glance.", group: "basics" },
  chatDefaults: { label: "New chats", description: "How new chats start. Each chat can change its own settings.", group: "agents" },
  codingAgents: { label: "Coding agents", description: "Install, sign in to, and set defaults for each coding agent.", group: "agents" },
  models: { label: "Models", description: "Models for your chats and the tasks Bridge runs for you.", group: "agents" },
  agents: { label: "Your agents", description: "Reusable models and instructions to start a chat with.", group: "agents" },
  permissions: { label: "Permissions", description: "How much Bridge asks before an agent acts.", group: "agents" },
  keys: { label: "API keys", description: "Keys agents can use on the sites you allow, without ever seeing them.", group: "agents" },
  import: { label: "Import", description: "Bring your chats and setup over from the tools you already use.", group: "data" },
  archives: { label: "Archived chats", description: "Search, read and unarchive conversations.", group: "data" },
  data: { label: "Storage", description: "What is using space on this Mac, and what you can let go of.", group: "data" },
  workers: { label: "Background tasks", description: "Tasks running at once, retries, and how long Bridge waits for an agent.", group: "advanced" },
  prompts: { label: "Bridge instructions", description: "Instructions for Bridge's built-in roles. Permissions still apply.", group: "advanced" },
  work: { label: "Daily briefing", description: "A summary of activity from your connected tools.", group: "advanced" },
  updates: { label: "Updates", description: "Choose which Bridge builds you receive and check for a new one.", group: "footer" },
};

export const RAIL_GROUPS: { id: RailGroupId; label: string; pages: PageId[] }[] = [
  { id: "basics", label: "Basics", pages: ["account", "general", "notifications", "voice", "composer", "menuBar"] },
  { id: "agents", label: "Agents & models", pages: ["chatDefaults", "codingAgents", "models", "agents", "permissions", "keys"] },
  { id: "data", label: "Data", pages: ["import", "archives", "data"] },
  { id: "advanced", label: "Advanced", pages: ["workers", "prompts", "work"] },
];

/** Every page, in rail order, footer last. */
export const ALL_PAGES: PageId[] = [...RAIL_GROUPS.flatMap(group => group.pages), "updates"];

const ALIASES: Record<LegacySection, PageId> = {
  appearance: "general",
  storage: "data",
  harnesses: "codingAgents",
  history: "import",
};

/** The page a section id opens, resolving retired ids. */
export function canonicalSection(section: Section): PageId {
  return (ALIASES as Record<string, PageId>)[section] ?? (section as PageId);
}

/** The label a section is known by, the same in the rail and as the page title. */
export function sectionLabel(section: Section): string {
  return PAGES[canonicalSection(section)].label;
}
