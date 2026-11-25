// The settings rail: back to the app, search, every page as a destination,
// then Updates and the reset action in the footer.
//
// Settings takes the window's sidebar slot rather than sitting beside it, the
// way System Settings and most desktop apps do it: two stacked navigation
// columns made every page narrow and asked the eye to pick which sidebar was
// in charge. The rail owns the window's leading edge, so its first row keeps
// clear of the traffic lights and drags the window.
//
// Every page is one click away at every width. On a wide window the rail is a
// full-height column with its own scroll; below md it collapses to a section
// picker that lists the same pages under the same group labels. Advanced is
// collapsed by default and remembers being opened. Reset lives here, behind a
// confirmation that names what it deletes, because a destructive global
// action does not belong in a page header.

import { useState } from "react";
import {
  RotateCcw as ArrowCounterClockwise, ChevronLeft as CaretLeft, ChevronRight as CaretRight, Code, Download as DownloadSimple, RefreshCw, type LucideIcon as Icon, Keyboard, Search as MagnifyingGlass, Bot as Robot,
  ScrollText as Scroll, ShieldCheck, SlidersHorizontal, Sparkles as Sparkle, Sun, HardDrive, Archive, Mic, Bell, KeyRound, MessagesSquare, Newspaper, Cpu,
} from "lucide-react";
import { ALL_PAGES, PAGES, RAIL_GROUPS, type PageId } from "./sections";
import { filterSettingsRows, type SearchableRow } from "./settingsSearch";
import { GhostButton, Select, TextButton } from "./kit";
import type { UpdateInfo } from "../../updater";
import { cn } from "@/lib/utils";
import { InputGroup, InputGroupAddon, InputGroupInput } from "@/components/ui/input-group";
import { Button } from "@/components/ui/button";
import { RowButton } from "@/components/ui/row";

const PAGE_ICONS: Record<PageId, Icon> = {
  general: Sun,
  notifications: Bell,
  voice: Mic,
  composer: Keyboard,
  menuBar: SlidersHorizontal,
  chatDefaults: MessagesSquare,
  codingAgents: Code,
  models: Cpu,
  agents: Robot,
  permissions: ShieldCheck,
  keys: KeyRound,
  import: DownloadSimple,
  archives: Archive,
  data: HardDrive,
  workers: Sparkle,
  prompts: Scroll,
  work: Newspaper,
  updates: RefreshCw,
};

export const ADVANCED_OPEN_KEY = "bridge.settings.railAdvancedOpen";

function readAdvancedOpen(): boolean {
  try { return localStorage.getItem(ADVANCED_OPEN_KEY) === "true"; } catch { return false; }
}

function writeAdvancedOpen(open: boolean) {
  try { localStorage.setItem(ADVANCED_OPEN_KEY, open ? "true" : "false"); } catch { /* the fold still works for this visit */ }
}

/** The narrow-window picker: every page, under the rail's group labels. */
const PICKER_OPTIONS = ALL_PAGES.map(id => ({
  value: id,
  label: PAGES[id].label,
  group: RAIL_GROUPS.find(group => group.pages.includes(id))?.label ?? "Bridge",
}));

export function SettingsRail({ section, query, rows, onQueryChange, onSelect, onSearchSelect, onResetAll, onExit, resetting, availableUpdate }: {
  section: PageId;
  query: string;
  rows: SearchableRow[];
  onQueryChange: (next: string) => void;
  onSelect: (next: PageId) => void;
  onSearchSelect: (row: SearchableRow) => void;
  onResetAll: () => void;
  /** Leave Settings for wherever the user was before it. */
  onExit?: () => void;
  resetting: boolean;
  availableUpdate?: UpdateInfo;
}) {
  const [confirming, setConfirming] = useState(false);
  const [advancedOpen, setAdvancedOpen] = useState(readAdvancedOpen);
  const results = query.trim() ? filterSettingsRows(query, rows) : null;

  const item = (id: PageId) => {
    const PageIcon = PAGE_ICONS[id];
    const active = section === id;
    return <RowButton
      key={id}
      tone="nav"
      shape="card"
      active={active}
      aria-current={active ? "page" : undefined}
      onClick={() => onSelect(id)}
    >
      <PageIcon size={14} strokeWidth={1.7} aria-hidden="true" className="shrink-0" />
      <span className="min-w-0 flex-1 truncate">{PAGES[id].label}</span>
      {id === "updates" && availableUpdate && <span role="img" aria-label="Update available" className="size-1.5 shrink-0 rounded-full bg-foreground" />}
    </RowButton>;
  };

  return <nav aria-label="Settings" className="u-vibrancy-sidebar flex w-full shrink-0 flex-col border-b border-sidebar-border bg-sidebar md:h-full md:w-60 md:border-b-0 md:border-r">
    <div data-tauri-drag-region="deep" className="u-traffic-inset flex h-11 shrink-0 items-center pl-24 pr-3">
      {onExit && <Button variant="quiet" size="xs" onClick={onExit}>
        <CaretLeft size={14} strokeWidth={1.7} aria-hidden="true" />Back to app
      </Button>}
    </div>
    <div className="flex shrink-0 items-center gap-2 px-3 pb-2 md:block">
      <div className="min-w-0 flex-1 md:hidden">
        <Select label="Settings section" value={section} options={PICKER_OPTIONS} onChange={value => onSelect(value as PageId)} width="w-full" />
      </div>
      <InputGroup className="min-w-0 flex-1">
        <InputGroupAddon><MagnifyingGlass size={12} strokeWidth={1.7} aria-hidden="true" /></InputGroupAddon>
        <InputGroupInput
          size="sm"
          type="search"
          value={query}
          aria-label="Search settings"
          placeholder="Search settings"
          onChange={event => onQueryChange(event.target.value)}
        />
      </InputGroup>
    </div>

    <div className={cn("min-h-0 flex-1 overflow-y-auto px-3 pb-3 pt-1", results ? "max-h-72 md:max-h-none" : "hidden md:block")}>
      {results
        ? <SearchResults results={results} onSelect={onSearchSelect} />
        : RAIL_GROUPS.map(group => {
            const collapsible = group.id === "advanced";
            // The page on screen is always visible, even inside a closed fold.
            const open = !collapsible || advancedOpen || group.pages.includes(section);
            return <div key={group.id} role="group" aria-label={group.label} className="mt-4 first:mt-1">
              {collapsible
                ? <RowButton
                    size="sm"
                    tone="plain"
                    aria-expanded={open}
                    onClick={() => { const next = !open; setAdvancedOpen(next); writeAdvancedOpen(next); }}
                  >
                    <span className="flex-1">{group.label}</span>
                    <CaretRight size={12} strokeWidth={1.7} aria-hidden="true" className={cn("transition-transform", open && "rotate-90")} />
                  </RowButton>
                : <p className="flex h-6 items-center px-2 text-caption font-medium text-muted-foreground">{group.label}</p>}
              {open && <div className="mt-0.5 space-y-px">{group.pages.map(item)}</div>}
            </div>;
          })}
    </div>

    <div className="shrink-0 space-y-1 border-t border-border/60 p-3">
      <div className="hidden md:block">{item("updates")}</div>
      {confirming
        ? <div className="rounded-lg border border-border-card bg-card p-2.5">
            <p className="text-caption leading-relaxed text-muted-foreground">
              This restores default agent settings, model choices, and instructions, and removes your agents. Chats and sign-in details are kept.
            </p>
            <div className="mt-2.5 flex flex-wrap items-center gap-2">
              <GhostButton onClick={() => setConfirming(false)}>Keep them</GhostButton>
              <TextButton tone="destructive" disabled={resetting} onClick={() => { setConfirming(false); onResetAll(); }}>Reset everything</TextButton>
            </div>
          </div>
        : <RowButton
            tone="muted"
            shape="card"
            className="items-start"
            disabled={resetting}
            onClick={() => setConfirming(true)}
          >
            <ArrowCounterClockwise size={14} strokeWidth={1.7} aria-hidden="true" className="mt-0.5 shrink-0" />
            <span className="min-w-0 flex-1">Reset agents, models and instructions</span>
          </RowButton>}
    </div>
  </nav>;
}

function SearchResults({ results, onSelect }: {
  results: ReturnType<typeof filterSettingsRows>;
  onSelect: (row: SearchableRow) => void;
}) {
  if (results.length === 0) {
    return <p className="px-2 py-2 text-caption text-muted-foreground">No setting matches that.</p>;
  }
  return <div role="list" aria-label="Search results">
    {results.map(group => <div key={group.page} className="mb-3 last:mb-0">
      <p className="px-2 pb-1 text-caption font-medium text-muted-foreground">{group.pageLabel}</p>
      {group.rows.map(row => <RowButton
        key={`${group.page}:${row.id ?? row.target?.id ?? "page"}:${row.label}`}
        tone="muted"
        shape="card"
        role="listitem"
        onClick={() => onSelect(row)}
      ><span className="block min-w-0 flex-1">
        <span className="block truncate text-ui">{row.label}</span>
        {row.description && <span className="block truncate text-caption text-muted-foreground/70">{row.description}</span>}
      </span></RowButton>)}
    </div>)}
  </div>;
}
