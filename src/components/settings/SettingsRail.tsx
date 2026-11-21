// The settings rail: title, search, the destinations, and the reset footer.
//
// It replaces both the old 64px header (whose subtitle described a Settings
// screen that no longer existed) and the shield note that sat under the nav
// saying something true of prompts on a surface that is not only prompts.
// Reset all moved here, behind a confirmation that names what it deletes,
// because a destructive global action does not belong in a page header.

import { useState } from "react";
import {
  RotateCcw as ArrowCounterClockwise, Code, Download as DownloadSimple, RefreshCw, type LucideIcon as Icon, Keyboard, Search as MagnifyingGlass, Bot as Robot,
  ScrollText as Scroll, ShieldCheck, SlidersHorizontal, Sparkles as Sparkle, Sun, HardDrive, Archive, Mic, Bell, KeyRound, MessagesSquare,
} from "lucide-react";
import { ALL_PAGES, PAGES, RAIL_GROUPS, type PageId } from "./sections";
import type { UpdateInfo } from "../../updater";
import { filterSettingsRows, type SearchableRow } from "./settingsSearch";
import { GhostButton, Select, TextButton } from "./kit";
import { cn } from "@/lib/utils";

const PAGE_ICONS: Record<PageId, Icon> = {
  general: Sun,
  notifications: Bell,
  voice: Mic,
  composer: Keyboard,
  menuBar: SlidersHorizontal,
  chatDefaults: MessagesSquare,
  codingAgents: Code,
  models: Sparkle,
  agents: Robot,
  permissions: ShieldCheck,
  keys: KeyRound,
  import: DownloadSimple,
  archives: Archive,
  data: HardDrive,
  workers: Robot,
  prompts: Scroll,
  work: Sparkle,
  updates: RefreshCw,
};

export function SettingsRail({ section, query, rows, onQueryChange, onSelect, onSearchSelect, onResetAll, resetting, availableUpdate }: {
  section: PageId;
  query: string;
  rows: SearchableRow[];
  onQueryChange: (next: string) => void;
  onSelect: (next: PageId) => void;
  onSearchSelect: (row: SearchableRow) => void;
  onResetAll: () => void;
  resetting: boolean;
  availableUpdate?: UpdateInfo;
}) {
  const [confirming, setConfirming] = useState(false);
  const results = query.trim() ? filterSettingsRows(query, rows) : null;

  return <nav aria-label="Settings" className="flex w-full shrink-0 flex-col border-b border-border bg-muted/30 md:w-44 md:border-r md:border-b-0 lg:w-52">
    <div className="shrink-0 px-3 pt-4">
      <h1 className="hidden px-1.5 text-message font-semibold text-foreground md:block">Settings</h1>
      <div className="md:hidden"><Select label="Settings section" value={section} options={ALL_PAGES.map(id => ({ value: id, label: PAGES[id].label }))} onChange={value => onSelect(value as PageId)} width="w-full" /></div>
      <div className="relative mt-3">
        <MagnifyingGlass size={12} strokeWidth={1.7} aria-hidden="true" className="pointer-events-none absolute left-2 top-1/2 -translate-y-1/2 text-muted-foreground" />
        <input
          type="search"
          value={query}
          aria-label="Search settings"
          placeholder="Search"
          onChange={event => onQueryChange(event.target.value)}
          className="h-7 w-full rounded-lg border border-border-card bg-popover pl-7 pr-2 text-xs text-foreground outline-none transition-colors placeholder:text-muted-foreground focus:border-ring"
        />
      </div>
    </div>

    <div className={cn("min-h-0 flex-1 overflow-y-auto px-3 py-3 md:block", results ? "max-h-40 md:max-h-none" : "hidden")}>
      {results
        ? <SearchResults results={results} onSelect={onSearchSelect} />
        : [...RAIL_GROUPS, { id: "footer", label: "", pages: ["updates"] as PageId[] }].map(group => <div key={group.id} className="mb-3 last:mb-0">
            {group.label && <p className="px-1.5 pb-1 text-caption font-medium text-muted-foreground">{group.label}</p>}
            {group.pages.map(id => {
              const Icon = PAGE_ICONS[id];
              const active = section === id;
              return <button
                key={id}
                type="button"
                aria-current={active ? "page" : undefined}
                onClick={() => onSelect(id)}
                className={cn(
                  "flex h-8 w-full items-center gap-2 rounded-lg px-1.5 text-left text-ui transition-colors",
                  active ? "bg-selection font-medium text-selection-foreground" : "text-muted-foreground hover:bg-accent hover:text-foreground",
                )}
              >
                <Icon size={14} strokeWidth={1.7} className="shrink-0" />
                <span className="min-w-0 flex-1 truncate">{PAGES[id].label}</span>
                {id === "updates" && availableUpdate && <span aria-label="Update available" className="size-1.5 shrink-0 rounded-full bg-foreground" />}
              </button>;
            })}
          </div>)}
    </div>

    <div className="shrink-0 border-t border-border/60 p-3">
      {confirming
        ? <div className="rounded-lg border border-border-card bg-card p-2.5">
            <p className="text-caption leading-relaxed text-muted-foreground">
              This restores default agent settings, model choices, and instructions, and removes your agents. Chats and sign-in details are kept.
            </p>
            <div className="mt-2.5 flex items-center gap-2">
              <GhostButton onClick={() => setConfirming(false)}>Keep them</GhostButton>
              <TextButton tone="destructive" disabled={resetting} onClick={() => { setConfirming(false); onResetAll(); }}>Reset everything</TextButton>
            </div>
          </div>
        : <TextButton disabled={resetting} onClick={() => setConfirming(true)}>
            <ArrowCounterClockwise size={12} strokeWidth={1.7} aria-hidden="true" />Reset agents, models and instructions
          </TextButton>}
    </div>
  </nav>;
}

function SearchResults({ results, onSelect }: {
  results: ReturnType<typeof filterSettingsRows>;
  onSelect: (row: SearchableRow) => void;
}) {
  if (results.length === 0) {
    return <p className="px-1.5 py-2 text-caption text-muted-foreground">No setting matches that.</p>;
  }
  return <div role="list" aria-label="Search results">
    {results.map(group => <div key={group.page} className="mb-3 last:mb-0">
      <p className="px-1.5 pb-1 text-caption font-medium text-muted-foreground">{group.pageLabel}</p>
      {group.rows.map(row => <button
        key={`${group.page}:${row.id ?? row.target?.id ?? "page"}:${row.label}`}
        type="button"
        role="listitem"
        onClick={() => onSelect(row)}
        className="w-full rounded-lg px-1.5 py-1 text-left text-muted-foreground transition-colors hover:bg-foreground/[0.045] hover:text-foreground"
      >
        <span className="block truncate text-ui">{row.label}</span>
        {row.description && <span className="block truncate text-caption text-muted-foreground/70">{row.description}</span>}
      </button>)}
    </div>)}
  </div>;
}
