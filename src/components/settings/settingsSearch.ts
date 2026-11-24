// The rail search: one query over every page and every row.
//
// The index is built from the page registry (`sections.ts`) and the row list
// below. Each row's `id` is the `anchor` a page renders on that row through
// the kit, and a test mounts every page to prove each id is really there, so
// the index cannot drift from what the pages draw. Rows that only exist
// because of the user's data (a harness, an agent) are contributed at render
// time and open that item's own detail page.

import { ALL_PAGES, PAGES, type PageId } from "./sections";

export type SearchTarget = { kind: "harness" | "agent"; id: string };

export type SearchableRow = {
  page: PageId;
  label: string;
  description?: string;
  /** The `anchor` the page renders on this row. Absent: the hit opens the page. */
  id?: string;
  /** Opens a harness or agent detail page instead of a row. */
  target?: SearchTarget;
};

export type SearchGroup = {
  page: PageId;
  pageLabel: string;
  rows: SearchableRow[];
};

/** A row the page always renders, whatever the user has configured. */
export type RegistryRow = SearchableRow & { id: string };

export const SETTINGS_ROWS: RegistryRow[] = [
  { page: "general", id: "mode", label: "Mode", description: "Match macOS, Paper, or Graphite" },
  { page: "general", id: "window-background", label: "Window background", description: "Solid or tinted by your desktop" },
  { page: "general", id: "zoom", label: "Window zoom", description: "Scale the whole window" },
  { page: "general", id: "show-thinking", label: "Show thinking", description: "Draw or hide the model's reasoning text in the transcript" },
  { page: "general", id: "edit-activity", label: "Open edit activity automatically", description: "Open short Activity sections with file diffs in the transcript" },
  { page: "general", id: "thinking-control", label: "Thinking control", description: "Slider, Sentence, or List effort picker" },
  { page: "general", id: "mission-control", label: "Show worker chats in Mission Control", description: "Also show chats for background tasks" },

  { page: "notifications", id: "notices", label: "Notices", description: "Toast, banner and sound for needs you, a waiting result, failed and completed turns" },
  { page: "notifications", id: "quiet-hours", label: "Quiet hours", description: "No banners or sounds between two times" },

  { page: "voice", id: "local-dictation", label: "Local dictation", description: "Download or remove the private on-device English speech model" },

  { page: "composer", id: "inline-suggestions", label: "Inline suggestions", description: "Ghost-text continuations of your draft, accepted with Tab" },
  { page: "composer", id: "suggestion-model", label: "Suggestion model", description: "Which model writes the inline continuation" },
  { page: "composer", id: "search-deeper", label: "Search deeper", description: "Let chat search ask a small model when the index is unsure" },
  { page: "composer", id: "search-model", label: "Search model", description: "Which Claude model chat search uses" },

  { page: "menuBar", id: "menu-display", label: "Show in menu bar", description: "Show the icon, quota used or remaining, and today's spend" },
  { page: "menuBar", id: "menu-favorites", label: "Favorite providers", description: "Which providers appear first in the menu" },
  { page: "menuBar", id: "menu-providers", label: "Providers & accounts", description: "Read usage for each provider account" },
  { page: "menuBar", id: "menu-usage", label: "Usage & spend", description: "Account, overview, daily history, tokens and spend" },
  { page: "menuBar", id: "menu-refresh", label: "Refresh account usage", description: "How often the menu bar refreshes usage" },

  { page: "chatDefaults", id: "default-target", label: "Agent or chat mode", description: "Which agent a new chat starts with" },
  { page: "chatDefaults", id: "default-model", label: "Default model", description: "The model a new chat starts on" },
  { page: "chatDefaults", id: "default-effort", label: "Default thinking effort", description: "Thinking effort for new chats" },
  { page: "chatDefaults", id: "default-access", label: "Access", description: "User approval or full access for new chats" },
  { page: "chatDefaults", id: "default-location", label: "Work location", description: "Project checkout or a new isolated worktree" },
  { page: "chatDefaults", id: "input-while-running", label: "Input while running", description: "Steer or queue messages sent while an agent is working" },
  { page: "chatDefaults", id: "limit-recovery", label: "When a usage limit stops a chat", description: "Resume automatically when the limit resets, or switch to a backup model" },

  { page: "codingAgents", id: "installed-agents", label: "Installed coding agents", description: "Coding agents Bridge can start right now, and ones it can install" },
  { page: "codingAgents", id: "early-compaction", label: "Early compaction", description: "Smaller context for an idle Claude or Codex chat, off by default" },

  { page: "models", id: "model-catalog", label: "Available models", description: "Provider model catalogs and their freshness, for chats, background tasks and verification" },

  { page: "agents", id: "your-agents", label: "New agent", description: "Create an agent with reusable instructions" },

  { page: "permissions", id: "auto-approve", label: "Let agents act without asking", description: "Accept provider permission requests automatically" },
  { page: "permissions", id: "instruction-proposals", label: "Suggested instruction changes", description: "Let background agents suggest improvements to their instructions" },
  { page: "permissions", id: "personal-apps", label: "Allow personal apps through OpenCode", description: "Off by default, less safe" },
  { page: "permissions", id: "always-asks", label: "Sending or publishing from a browser", description: "Always asks, whatever the switch says" },
  { page: "permissions", id: "auto-approvals", label: "Recent auto-approvals", description: "The audit log of automatic decisions" },

  { page: "keys", id: "saved-keys", label: "Saved keys", description: "API keys agents can use on the sites you allow, without seeing them" },
  { page: "keys", id: "pasted-keys", label: "Pasted in chats", description: "Keys you pasted into a chat, and where each may be sent" },

  { page: "data", id: "worktree-storage", label: "Worktree storage", description: "Project copies, safe cleanup, repositories and retention" },

  { page: "workers", id: "preferred-agent", label: "Preferred coding agent", description: "Which coding agent background tasks use first" },
  { page: "workers", id: "task-limits", label: "Background task limits", description: "Tasks running at once, per message, and wait time" },
  { page: "workers", id: "orchestrator-notices", label: "Orchestrator notices", description: "Which worker notices cost an orchestrator a model turn" },
  { page: "workers", id: "reviewer", label: "Pull request reviewer", description: "Model, effort and instructions per harness for GitHub PR reviews" },
  { page: "workers", id: "review-jobs", label: "Review jobs", description: "Draft first or post automatically, auto-review on push, skip draft PRs, incremental re-review, second opinion, fallback reviewers, reviews at once" },

  { page: "work", id: "briefing", label: "Integration briefing", description: "Slack, GitHub, and other integration activity from the past 24 hours, and what it reads" },
  { page: "work", id: "briefing-schedule", label: "Refresh frequency", description: "How often the briefing runs, and refresh on focus" },

  { page: "updates", id: "beta-builds", label: "Beta nightly builds", description: "Opt in to signed nightly updates" },
  { page: "updates", id: "check-updates", label: "Check for updates", description: "Check the selected channel now" },
];

/** Each page itself is a hit, found by its name or what it is for. */
export const PAGE_ROWS: SearchableRow[] = ALL_PAGES.map(page => ({ page, label: PAGES[page].label, description: PAGES[page].description }));

function matches(row: SearchableRow, needle: string): boolean {
  return row.label.toLowerCase().includes(needle)
    || (row.description?.toLowerCase().includes(needle) ?? false);
}

/**
 * Rows that match `query`, grouped by the page that owns them, in rail order.
 * An empty query returns every row.
 */
export function filterSettingsRows(query: string, rows: SearchableRow[]): SearchGroup[] {
  const needle = query.trim().toLowerCase();
  const hits = needle ? rows.filter(row => matches(row, needle)) : rows;
  const byPage = new Map<PageId, SearchableRow[]>();
  for (const row of hits) {
    const existing = byPage.get(row.page);
    if (existing) existing.push(row); else byPage.set(row.page, [row]);
  }
  return ALL_PAGES.filter(page => byPage.has(page)).map(page => ({
    page,
    pageLabel: PAGES[page].label,
    rows: byPage.get(page) ?? [],
  }));
}
