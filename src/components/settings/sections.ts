// Legacy ids remain valid for contextual links; only four destinations live in the rail.
export type Section =
  | "general" | "codingAgents" | "data"
  | "appearance" | "menuBar" | "updates" | "permissions" | "composer"
  | "agents" | "models" | "prompts" | "harnesses" | "clones"
  | "work" | "import" | "archives" | "workers" | "storage";

export type PrimarySection = "general" | "codingAgents" | "permissions" | "data";
export type RailGroup = "Settings";
export const SECTION_LABELS: Record<Section, string> = {
  general: "General", codingAgents: "Coding agents", data: "Data & storage",
  appearance: "Appearance", menuBar: "Menu bar", updates: "Updates",
  permissions: "Permissions", composer: "Typing & search", agents: "Saved setups",
  models: "Model preferences", prompts: "Bridge instructions", harnesses: "Coding agents",
  clones: "Browser copies", work: "Daily briefing", import: "Import history",
  storage: "Disk usage", archives: "Archived chats", workers: "Background tasks",
};
export const PRIMARY_SECTIONS: PrimarySection[] = ["general", "codingAgents", "permissions", "data"];
export const SECTION_ORDER: { group: RailGroup; sections: PrimarySection[] }[] = [
  { group: "Settings", sections: PRIMARY_SECTIONS },
];
export const ALL_SECTIONS = Object.keys(SECTION_LABELS) as Section[];
export function primarySection(section: Section): PrimarySection {
  if (["general", "appearance", "menuBar", "updates", "composer"].includes(section)) return "general";
  if (["permissions", "clones"].includes(section)) return "permissions";
  if (["data", "import", "storage", "archives"].includes(section)) return "data";
  return "codingAgents";
}
