// Legacy ids remain valid for contextual links; only five destinations live in the rail.
export type Section =
  | "general" | "codingAgents" | "data"
  | "appearance" | "menuBar" | "notifications" | "updates" | "permissions" | "composer" | "voice"
  | "chatDefaults" | "agents" | "models" | "prompts" | "harnesses"
  | "work" | "import" | "archives" | "workers" | "storage" | "keys";

export type PrimarySection = "general" | "codingAgents" | "permissions" | "data" | "import";
export type RailGroup = "Settings";
export const SECTION_LABELS: Record<Section, string> = {
  chatDefaults: "New chat defaults", voice: "Voice", general: "General", codingAgents: "Coding agents", data: "Data & storage",
  appearance: "Appearance", menuBar: "Menu bar", notifications: "Notifications", updates: "Updates",
  permissions: "Permissions", composer: "Typing & search", agents: "Agents",
  models: "Model preferences", prompts: "Bridge instructions", harnesses: "Coding agents",
  work: "Daily briefing", import: "Import",
  storage: "Disk usage", archives: "Archived chats", workers: "Background tasks", keys: "API keys",
};
export const PRIMARY_SECTIONS: PrimarySection[] = ["general", "codingAgents", "permissions", "data", "import"];
export const SECTION_ORDER: { group: RailGroup; sections: PrimarySection[] }[] = [
  { group: "Settings", sections: PRIMARY_SECTIONS },
];
export const ALL_SECTIONS = Object.keys(SECTION_LABELS) as Section[];
export function primarySection(section: Section): PrimarySection {
  if (["general", "appearance", "menuBar", "notifications", "updates", "composer", "voice"].includes(section)) return "general";
  if (["permissions", "keys"].includes(section)) return "permissions";
  if (["data", "storage", "archives"].includes(section)) return "data";
  // Moving to Bridge from another tool is its own destination, not a data detail.
  if (section === "import") return "import";
  return "codingAgents";
}
