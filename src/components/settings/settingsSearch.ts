// The rail search: one query over every row on every page.
//
// Settings grew to nine pages, and "which page is the inline suggestion model
// on" stopped having an obvious answer. Rather than guess at an information
// architecture that makes every row findable by browsing, the rail searches the
// rows themselves and names the page each hit belongs to.
//
// The static index below covers the rows a page always has. Rows that only
// exist because of the user's data (a harness, a preset, a prompt section) are
// contributed at render time through `extraRows`, so a search for "opencode"
// finds the runtime as well as the word in a description.

import { SECTION_LABELS, type Section } from "./sections";

export type SearchableRow = {
  section: Section;
  label: string;
  description?: string;
};

export type SearchGroup = {
  section: Section;
  pageLabel: string;
  rows: SearchableRow[];
};

/** Every row a page shows regardless of what the user has configured. */
export const STATIC_SETTINGS_ROWS: SearchableRow[] = [
  { section: "workers", label: "Worker limits and routing", description: "Default harness, concurrency, retries, failover, stall timeout and warm retention" },
  { section: "archives", label: "Archived chats", description: "Search, read and unarchive conversations without restoring worktrees" },
  { section: "storage", label: "Worktree storage", description: "Disk usage, safe cleanup, repositories and retention" },
  { section: "appearance", label: "Mode", description: "Match macOS, Paper, or Graphite" },
  { section: "appearance", label: "Shell", description: "Solid or Cursor translucency" },
  { section: "appearance", label: "Thinking control", description: "Slider, Sentence, or List effort picker" },

  { section: "permissions", label: "Auto-approve provider permissions", description: "Accept provider permission requests automatically" },
  { section: "permissions", label: "Worker write scope", description: "Always asks, whatever the switch says" },
  { section: "permissions", label: "Browser outward effects", description: "Always asks, whatever the switch says" },
  { section: "permissions", label: "Recent auto-approvals", description: "The audit log of automatic decisions" },

  { section: "composer", label: "Inline suggestions", description: "Ghost-text continuations of your draft, accepted with Tab" },
  { section: "composer", label: "Suggestion model", description: "Which model writes the inline continuation" },

  { section: "agents", label: "New preset", description: "Create an agent preset" },

  { section: "models", label: "Orchestration", description: "The model you talk to" },
  { section: "models", label: "Workers", description: "Agents the orchestrator delegates to" },
  { section: "models", label: "Verification", description: "Verification roles with a specialized rubric" },
  { section: "models", label: "Catalog", description: "Provider model catalogs and their freshness" },

  { section: "prompts", label: "Prompt target", description: "Orchestrator, worker roles, or direct session" },
  { section: "prompts", label: "Export overrides", description: "Write every prompt override to a JSON file" },
  { section: "prompts", label: "Import overrides", description: "Apply prompt overrides from a JSON file" },

  { section: "harnesses", label: "Installed", description: "Runtimes Bridge can start right now" },
  { section: "harnesses", label: "Available", description: "Runtimes Bridge can install for you" },

  { section: "work", label: "Suggested work", description: "A background briefing turn on the harness you pick" },
  { section: "work", label: "What it reads", description: "Which connected tools the briefing may read" },
  { section: "work", label: "Cadence", description: "How often the briefing runs" },
  { section: "work", label: "Refresh on focus", description: "Also refresh when Bridge regains focus" },

  { section: "import", label: "Import from another harness", description: "Bring Claude Code history, memory, and setup into Bridge" },
];

function matches(row: SearchableRow, needle: string): boolean {
  return row.label.toLowerCase().includes(needle)
    || (row.description?.toLowerCase().includes(needle) ?? false);
}

/**
 * Rows that match `query`, grouped by the page that owns them and kept in the
 * rail's page order. An empty query returns every row, so a caller can render
 * the same list for "show me everything" without a second code path.
 */
export function filterSettingsRows(query: string, rows: SearchableRow[]): SearchGroup[] {
  const needle = query.trim().toLowerCase();
  const hits = needle ? rows.filter(row => matches(row, needle)) : rows;
  const bySection = new Map<Section, SearchableRow[]>();
  for (const row of hits) {
    const existing = bySection.get(row.section);
    if (existing) existing.push(row); else bySection.set(row.section, [row]);
  }
  // Preserve the order rows arrived in, which is the rail's page order for the
  // static index and the user's own order for contributed rows.
  const seen: Section[] = [];
  for (const row of hits) if (!seen.includes(row.section)) seen.push(row.section);
  return seen.map(section => ({
    section,
    pageLabel: SECTION_LABELS[section],
    rows: bySection.get(section) ?? [],
  }));
}
