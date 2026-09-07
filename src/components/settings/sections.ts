// The nine settings pages and the four rail groups they sit in.
//
// Ids are wire-stable, not cosmetic: `App.tsx` opens `permissions` from the
// bypass badge and `prompts` from the usage panel, so renaming either would
// break a caller that has nothing to do with Settings. `agents` still addresses
// what the rail now calls Presets, and `work` what it calls Work briefing, for
// the same reason.

export type Section =
  | "appearance"
  | "permissions"
  | "composer"
  | "agents"
  | "models"
  | "prompts"
  | "harnesses"
  | "work"
  | "import";

export type RailGroup = "General" | "Agents" | "Runtimes" | "Data";

export const SECTION_LABELS: Record<Section, string> = {
  appearance: "Appearance",
  permissions: "Permissions",
  composer: "Composer",
  agents: "Presets",
  models: "Models",
  prompts: "Prompts",
  harnesses: "Harnesses",
  work: "Work briefing",
  import: "Import",
};

/** Rail order. The list is the contract: General, Agents, Runtimes, Data. */
export const SECTION_ORDER: { group: RailGroup; sections: Section[] }[] = [
  { group: "General", sections: ["appearance", "permissions", "composer"] },
  { group: "Agents", sections: ["agents", "models", "prompts"] },
  { group: "Runtimes", sections: ["harnesses"] },
  { group: "Data", sections: ["work", "import"] },
];

export const ALL_SECTIONS: Section[] = SECTION_ORDER.flatMap(group => group.sections);
