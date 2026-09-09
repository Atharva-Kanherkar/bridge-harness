// The settings rail: title, search, four groups, and the reset footer.
//
// It replaces both the old 64px header (whose subtitle described a Settings
// screen that no longer existed) and the shield note that sat under the nav
// saying something true of prompts on a surface that is not only prompts.
// Reset all moved here, behind a confirmation that names what it deletes,
// because a destructive global action does not belong in a page header.

import { useState } from "react";
import {
  RotateCcw as ArrowCounterClockwise, Code, Download as DownloadSimple, type LucideIcon as Icon, Keyboard, Search as MagnifyingGlass, Bot as Robot,
  ScrollText as Scroll, ShieldCheck, SlidersHorizontal, Sparkles as Sparkle, Sun, HardDrive,
} from "lucide-react";
import { SECTION_LABELS, SECTION_ORDER, type Section } from "./sections";
import { filterSettingsRows, type SearchableRow } from "./settingsSearch";
import { GhostButton, Select, TextButton } from "./kit";
import { cn } from "@/lib/utils";

const SECTION_ICONS: Record<Section, Icon> = {
  appearance: Sun,
  permissions: ShieldCheck,
  composer: Keyboard,
  agents: Robot,
  models: SlidersHorizontal,
  prompts: Scroll,
  storage: HardDrive,
  harnesses: Code,
  work: Sparkle,
  import: DownloadSimple,
};

export function SettingsRail({ section, query, rows, onQueryChange, onSelect, onResetAll, resetting }: {
  section: Section;
  query: string;
  rows: SearchableRow[];
  onQueryChange: (next: string) => void;
  onSelect: (next: Section) => void;
  onResetAll: () => void;
  resetting: boolean;
}) {
  const [confirming, setConfirming] = useState(false);
  const results = query.trim() ? filterSettingsRows(query, rows) : null;

  return <nav aria-label="Settings" className="flex w-full shrink-0 flex-col border-b border-border bg-muted/30 md:w-44 md:border-r md:border-b-0 lg:w-52">
    <div className="shrink-0 px-3 pt-4">
      <h1 className="hidden px-1.5 text-[15px] font-semibold text-foreground md:block">Settings</h1>
      <div className="md:hidden"><Select label="Settings section" value={section} options={SECTION_ORDER.flatMap(group => group.sections.map(id => ({ value: id, label: SECTION_LABELS[id] })))} onChange={value => onSelect(value as Section)} width="w-full" /></div>
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
        ? <SearchResults results={results} onSelect={onSelect} />
        : SECTION_ORDER.map(group => <div key={group.group} className="mb-3 last:mb-0">
            <p className="px-1.5 pb-1 text-[11px] font-medium text-muted-foreground">{group.group}</p>
            {group.sections.map(id => {
              const Icon = SECTION_ICONS[id];
              const active = section === id;
              return <button
                key={id}
                type="button"
                aria-current={active ? "page" : undefined}
                onClick={() => onSelect(id)}
                className={cn(
                  "flex h-8 w-full items-center gap-2 rounded-lg px-1.5 text-left text-[13px] transition-colors",
                  active ? "bg-selection font-medium text-selection-foreground" : "text-muted-foreground hover:bg-accent hover:text-foreground",
                )}
              >
                <Icon size={14} strokeWidth={1.7} className="shrink-0" />
                <span className="min-w-0 flex-1 truncate">{SECTION_LABELS[id]}</span>
              </button>;
            })}
          </div>)}
    </div>

    <div className="shrink-0 border-t border-border/60 p-3">
      {confirming
        ? <div className="rounded-lg border border-border-card bg-card p-2.5">
            <p className="text-[11.5px] leading-relaxed text-muted-foreground">
              This deletes every Bridge, Codex, Claude, and agent override, resets all role model profiles, and removes the agent presets you created. Built-in presets come back at their defaults.
            </p>
            <div className="mt-2.5 flex items-center gap-2">
              <GhostButton onClick={() => setConfirming(false)}>Keep them</GhostButton>
              <TextButton tone="destructive" disabled={resetting} onClick={() => { setConfirming(false); onResetAll(); }}>Reset everything</TextButton>
            </div>
          </div>
        : <TextButton disabled={resetting} onClick={() => setConfirming(true)}>
            <ArrowCounterClockwise size={12} strokeWidth={1.7} aria-hidden="true" />Reset all settings
          </TextButton>}
    </div>
  </nav>;
}

function SearchResults({ results, onSelect }: {
  results: ReturnType<typeof filterSettingsRows>;
  onSelect: (next: Section) => void;
}) {
  if (results.length === 0) {
    return <p className="px-1.5 py-2 text-[11.5px] text-muted-foreground">No setting matches that.</p>;
  }
  return <div role="list" aria-label="Search results">
    {results.map(group => <div key={group.section} className="mb-3 last:mb-0">
      <p className="px-1.5 pb-1 text-[11px] font-medium text-muted-foreground">{group.pageLabel}</p>
      {group.rows.map(row => <button
        key={`${group.section}:${row.label}`}
        type="button"
        role="listitem"
        onClick={() => onSelect(group.section)}
        className="w-full rounded-lg px-1.5 py-1 text-left text-muted-foreground transition-colors hover:bg-foreground/[0.045] hover:text-foreground"
      >
        <span className="block truncate text-[13px]">{row.label}</span>
        {row.description && <span className="block truncate text-[11px] text-muted-foreground/70">{row.description}</span>}
      </button>)}
    </div>)}
  </div>;
}
