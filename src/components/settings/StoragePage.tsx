// Storage: everything on this Mac, largest first, and the place a person can
// delete what they no longer want.
//
// The backend never waits on a folder walk. A listing comes back at once with
// the sizes it already knows, so the page asks again while anything is still
// being measured and the rows fill in. Deleting moves to the Trash unless the
// person asks for more; the backend refuses the few places that would break
// macOS or Bridge, and says why per row.
//
// The copilot rail is a standing Bridge chat docked beside the listing, with
// its own system prompt (`storage_agent.rs`). Messages carry what this page
// measured whenever it changed, and the agent proposes changes as
// ```storage-plan cards the person approves here (`StoragePlanCard`). The
// person can drag the rail wider; its width is kept per Mac.

import { useCallback, useEffect, useId, useMemo, useRef, useState, type CSSProperties, type KeyboardEvent as ReactKeyboardEvent, type PointerEvent as ReactPointerEvent, type ReactNode, type RefObject } from "react";
import { Check, ChevronDown, ChevronRight, CircleAlert, CircleCheck, File, Folder, FolderOpen, LoaderCircle, Lock, MessageCircle, RefreshCw, Trash2, X } from "lucide-react";
import { cn } from "@/lib/utils";
import { bridgeApi as api } from "../../api";
import type { DiskDeleteResult, DiskEntry, DiskListing, DiskOverview, DiskSuggestion } from "../../types";
import { cleanupTotal, diskBytes, displayPath, storageSnapshot, withSnapshot, type StoragePlanItem } from "../../diskSpace";
import { StorageAgentContext, type StorageAgentHost } from "../StoragePlanCard";
import { Button } from "../ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogPanel, DialogTitle } from "../ui/dialog";
import { GhostButton, PrimaryButton, StatusPill, TextButton } from "./kit";
import type { StorageCopilotHost } from "./StorageCopilot";
import { WorktreeStorage } from "./WorktreeStorage";

const POLL_MS = 1500;

/** The page's one categorical palette: the validated six-hue ramp the context
 *  lens already wears, in rank order, so the biggest thing is always the same
 *  blue. Chrome stays achromatic; only data marks are coloured. */
const SERIES = ["bg-ctx-1", "bg-ctx-2", "bg-ctx-3", "bg-ctx-4", "bg-ctx-5", "bg-ctx-6"];
const OTHER = "bg-foreground/15";

const TILE = "rounded-xl border border-border-card bg-card";
const PLACEHOLDER = "inline-block h-[0.75em] w-[3.5em] rounded-full bg-foreground/10 align-middle motion-safe:animate-pulse";

const ROOTS = [
  { label: "Home", path: null },
  { label: "Applications", path: "/Applications" },
  { label: "Whole disk", path: "/" },
] as const;

/** The assistant rail's width in px. The default is the old fixed 24rem; the
 *  listing always keeps LISTING_MIN beside it (the grid template enforces the
 *  same bound in CSS, so a narrower window never squeezes the listing). */
const RAIL_DEFAULT = 384;
const RAIL_MIN = 320;
const RAIL_MAX = 720;
const RAIL_STEP = 16;
const RAIL_KEY = "bridge.storage.railWidth";
const LISTING_MIN = 400;
/** The gap column the drag handle sits in (2rem). */
const RAIL_GUTTER = 32;

const clampRail = (width: number, room = Infinity) =>
  Math.round(Math.max(RAIL_MIN, Math.min(RAIL_MAX, room - RAIL_GUTTER - LISTING_MIN, width)));

function storedRailWidth() {
  const value = Number(localStorage.getItem(RAIL_KEY) ?? RAIL_DEFAULT);
  return Number.isFinite(value) ? clampRail(value) : RAIL_DEFAULT;
}

/** The drag edge between the listing and the assistant rail. It fills the gap
 *  between them; its line rides along with the sticky rail. */
function RailDivider({ width, railId, gridRef, railRef, onResize }: {
  width: number;
  railId: string;
  gridRef: RefObject<HTMLDivElement | null>;
  railRef: RefObject<HTMLDivElement | null>;
  onResize: (next: (width: number) => number) => void;
}) {
  const [dragging, setDragging] = useState(false);
  const room = () => gridRef.current?.clientWidth || Infinity;
  const resize = (next: (width: number) => number) => onResize(current => clampRail(next(current), room()));

  const start = (event: ReactPointerEvent<HTMLDivElement>) => {
    if (event.button !== 0) return;
    event.preventDefault();
    const handle = event.currentTarget;
    const originX = event.clientX;
    // Start from what is drawn: a narrow window may show less than was stored.
    const origin = railRef.current?.getBoundingClientRect().width || width;
    handle.setPointerCapture?.(event.pointerId);
    setDragging(true);
    // The rail sits on the right, so dragging left widens it.
    const move = (pointer: PointerEvent) => resize(() => origin + originX - pointer.clientX);
    const end = () => {
      setDragging(false);
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", end);
      window.removeEventListener("pointercancel", end);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", end);
    window.addEventListener("pointercancel", end);
  };

  const keys = (event: ReactKeyboardEvent<HTMLDivElement>) => {
    const next = event.key === "ArrowLeft" ? (current: number) => current + RAIL_STEP
      : event.key === "ArrowRight" ? (current: number) => current - RAIL_STEP
      : event.key === "Home" ? () => RAIL_MIN
      : event.key === "End" ? () => RAIL_MAX
      : null;
    if (next === null) return;
    event.preventDefault();
    resize(next);
  };

  return <div
    role="separator"
    aria-orientation="vertical"
    aria-label="Resize storage assistant"
    aria-controls={railId}
    aria-valuenow={width}
    aria-valuemin={RAIL_MIN}
    aria-valuemax={RAIL_MAX}
    tabIndex={0}
    title="Drag to resize · double-click to reset"
    onPointerDown={start}
    onKeyDown={keys}
    onDoubleClick={() => resize(() => RAIL_DEFAULT)}
    className="group hidden cursor-col-resize touch-none outline-none lg:block"
  >
    <span className="sticky top-6 mx-auto flex h-[calc(100dvh-6rem)] w-3 items-center justify-center">
      <span className={cn("h-full w-px rounded-full transition-colors group-hover:bg-ring/60 group-focus-visible:bg-ring", dragging ? "bg-ring/60" : "bg-transparent")} />
      <span aria-hidden="true" className={cn("absolute h-8 w-1 rounded-full bg-border transition-opacity group-hover:opacity-0 group-focus-visible:opacity-0", dragging && "opacity-0")} />
    </span>
  </div>;
}

type Confirming = { entries: DiskEntry[]; permanent: boolean } | "empty-trash" | null;

/** What the last delete or Empty Trash did, shown at the bottom of the page
 *  where it cannot scroll out of sight. */
interface Outcome {
  summary: string;
  failures: { name: string; reason: string }[];
  /** Something went to the Trash, so its space is still taken until emptied. */
  offerEmptyTrash: boolean;
}

function outcomeOf(result: DiskDeleteResult, entries: readonly DiskEntry[]): Outcome {
  // Name a failure the way the person picked it ("Docker disk image", not "vms").
  const named = new Map(entries.map(entry => [entry.path, entry.name]));
  const count = result.deleted.length;
  const what = `${count} item${count === 1 ? "" : "s"}${result.bytesFreed ? ` (${diskBytes(result.bytesFreed)})` : ""}`;
  const summary = count === 0
    ? `Nothing was ${result.trashed ? "moved" : "deleted"}.`
    : result.trashed ? `Moved ${what} to the Trash. The space comes back when you empty it.` : `Deleted ${what}.`;
  return {
    summary,
    failures: result.failed.map(failure => ({ name: named.get(failure.path) ?? (failure.path.split("/").pop() || failure.path), reason: failure.reason })),
    offerEmptyTrash: result.trashed && count > 0,
  };
}

function message(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

/** Descriptions mark commands with backticks; show them as code. */
function Prose({ text }: { text: string }) {
  return <>{text.split("`").map((part, index) => index % 2 === 1
    ? <code key={index} className="rounded bg-muted px-1 font-mono text-[0.92em] text-foreground">{part}</code>
    : part)}</>;
}

function SectionHeading({ title, detail, action }: { title: string; detail?: ReactNode; action?: ReactNode }) {
  return <header className="mb-2.5 flex flex-wrap items-center gap-x-3 gap-y-2 px-0.5">
    <h3 className="text-ui font-medium text-foreground">{title}</h3>
    {detail && <span className="text-caption text-muted-foreground">{detail}</span>}
    {action && <span className="ml-auto flex items-center gap-1.5">{action}</span>}
  </header>;
}

function Size({ entry }: { entry: Pick<DiskEntry, "sizeBytes" | "measuring"> & { partial?: boolean } }) {
  if (entry.sizeBytes === null || entry.sizeBytes === undefined) {
    // Still being measured: a quiet bar the size of a number, not a word
    // repeated on every row. `em` keeps it in step with the text it replaces.
    return entry.measuring
      ? <span role="status" aria-label="Measuring" className={PLACEHOLDER} />
      : <span className="text-muted-foreground">—</span>;
  }
  return <span title={entry.partial ? "Some items inside could not be read, so this is a lower bound." : undefined}>
    {entry.partial ? "≥ " : ""}{diskBytes(entry.sizeBytes)}
  </span>;
}

const IconAction = ({ label, onClick, disabled, tone = "muted", children }: { label: string; onClick: () => void; disabled?: boolean; tone?: "muted" | "destructive"; children: ReactNode }) =>
  <button
    type="button"
    aria-label={label}
    title={label}
    disabled={disabled}
    onClick={onClick}
    className={cn("grid size-7 shrink-0 place-items-center rounded-md outline-none transition-colors focus-visible:ring-2 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-30", tone === "destructive" ? "text-muted-foreground hover:bg-destructive/10 hover:text-destructive" : "text-muted-foreground hover:bg-accent hover:text-foreground")}
  >{children}</button>;

/** How full the disk is, in one card: what macOS says is available, then a
 *  bar of what fills the rest. The home folder's biggest children each wear a
 *  series colour; everything else the volume reports as used is one quiet
 *  segment. */
function DiskSummary({ overview, home, onOpen, onEmptyTrash, busy }: {
  overview: DiskOverview | null;
  home: DiskListing | null;
  onOpen: (path: string) => void;
  onEmptyTrash: () => void;
  busy: boolean;
}) {
  const volume = overview?.volume;
  if (!volume) return null;
  // Slivers under half a percent of the disk fold into "other", so while the
  // folders are still measuring a stray file never earns a colour.
  const top = (home?.entries ?? []).filter(entry => (entry.sizeBytes ?? 0) >= volume.totalBytes * 0.005).slice(0, SERIES.length);
  const pending = home?.measuring ? Math.max(Math.min(4, SERIES.length - top.length), 0) : 0;
  const named = top.reduce((total, entry) => total + (entry.sizeBytes ?? 0), 0);
  const other = Math.max(volume.usedBytes - named, 0);
  const share = (value: number) => `${Math.max((value / volume.totalBytes) * 100, value > 0 ? 0.4 : 0)}%`;
  const usedShare = volume.usedBytes / volume.totalBytes;
  return <section aria-label="Disk" className={cn(TILE, "p-5")}>
    <div className="flex flex-wrap items-end gap-x-6 gap-y-3">
      <div className="min-w-0">
        <p className="text-caption text-muted-foreground">Available</p>
        <p className="mt-0.5 flex flex-wrap items-baseline gap-x-2">
          <span className="font-display text-4xl font-semibold tabular-nums tracking-tight text-foreground">{diskBytes(volume.freeBytes)}</span>
          <span className="text-ui text-muted-foreground">of {diskBytes(volume.totalBytes)}</span>
          {usedShare > 0.9 && <StatusPill tone="warning">Nearly full</StatusPill>}
        </p>
      </div>
      <div className="ml-auto"><GhostButton onClick={onEmptyTrash} disabled={busy}><Trash2 size={13} aria-hidden="true" />Empty Trash</GhostButton></div>
    </div>
    <div className="mt-4 flex h-2.5 w-full gap-0.5 overflow-hidden rounded-full bg-muted" role="img" aria-label={`${diskBytes(volume.usedBytes)} used of ${diskBytes(volume.totalBytes)}`}>
      {top.map((entry, index) => <span key={entry.path} className={cn("h-full origin-left first:rounded-l-full motion-safe:animate-[meter-fill_600ms_ease-out]", SERIES[index])} style={{ width: share(entry.sizeBytes ?? 0) }} />)}
      <span className={cn("h-full", OTHER)} style={{ width: share(other) }} />
    </div>
    <ul className="mt-3 grid grid-cols-1 gap-x-4 gap-y-0.5 @md/storage:grid-cols-2 @3xl/storage:grid-cols-3">
      {top.map((entry, index) => <li key={entry.path}>
        <button type="button" onClick={() => onOpen(entry.path)} className="-mx-1.5 flex w-[calc(100%+0.75rem)] items-center gap-2 rounded-md px-1.5 py-1 text-left text-caption outline-none transition-colors hover:bg-accent focus-visible:ring-2 focus-visible:ring-ring">
          <span className={cn("size-2.5 shrink-0 rounded-[3px]", SERIES[index])} />
          <span className="min-w-0 flex-1 truncate text-foreground">{entry.name}</span>
          <span className="shrink-0 tabular-nums text-muted-foreground">{diskBytes(entry.sizeBytes)}</span>
        </button>
      </li>)}
      {Array.from({ length: pending }, (_, index) => <li key={`pending-${index}`} aria-hidden="true" className="flex items-center gap-2 py-1 text-caption">
        <span className="size-2.5 shrink-0 rounded-[3px] bg-foreground/10" />
        <span className={cn(PLACEHOLDER, "w-[7em]")} />
      </li>)}
      <li className="flex items-center gap-2 py-1 text-caption">
        <span className={cn("size-2.5 shrink-0 rounded-[3px]", OTHER)} />
        <span className="min-w-0 flex-1 truncate text-foreground">Apps, system, and other</span>
        <span className="shrink-0 tabular-nums text-muted-foreground">{diskBytes(other)}</span>
      </li>
    </ul>
  </section>;
}

function suggestionEntry(item: DiskSuggestion): DiskEntry {
  return { name: item.label, path: item.path, kind: "directory", sizeBytes: item.sizeBytes, itemCount: null, measuring: item.measuring, partial: false, modifiedAt: null, protectedReason: item.protectedReason ?? null };
}

const SUGGESTIONS_SHOWN = 6;

/** Known cleanup candidates as one list, largest first: what it is, whether
 *  it comes back on its own, and what it costs. A folder Bridge only empties
 *  one item at a time (Downloads, app caches) opens instead of trashing. */
function Suggestions({ overview, onOpen, onDelete, onAsk, busy }: {
  overview: DiskOverview;
  onOpen: (path: string) => void;
  onDelete: (entry: DiskEntry) => void;
  onAsk?: (item: DiskSuggestion) => void;
  busy: boolean;
}) {
  const [all, setAll] = useState(false);
  const rows = [...overview.suggestions].sort((a, b) => (b.sizeBytes ?? -1) - (a.sizeBytes ?? -1));
  if (rows.length === 0) return null;
  const shown = all ? rows : rows.slice(0, SUGGESTIONS_SHOWN);
  const safe = cleanupTotal(overview.suggestions, "safe");
  const review = cleanupTotal(overview.suggestions, "review");
  return <section aria-label="Cleanup suggestions">
    <SectionHeading title="Worth a look" detail={safe > 0 ? <><span className="text-success">{diskBytes(safe)}</span> rebuilds itself on demand{review > 0 ? <>, {diskBytes(review)} to review</> : null}</> : undefined} />
    <div className={cn(TILE, "overflow-hidden")}>
      <ul className="divide-y divide-border/60">
        {shown.map(item => <li key={item.id} className="flex items-center gap-3 py-2.5 pl-4 pr-2.5">
          <button type="button" onClick={() => onOpen(item.path)} className="min-w-0 flex-1 rounded text-left outline-none focus-visible:ring-2 focus-visible:ring-ring">
            <span className="flex items-baseline gap-2">
              <span className="truncate text-ui font-medium text-foreground">{item.label}</span>
              <span className={cn("shrink-0 text-[11px]", item.safety === "safe" ? "text-success" : "text-muted-foreground")}>{item.safety === "safe" ? "Rebuilds itself" : "Review first"}</span>
            </span>
            <span className="mt-0.5 block truncate text-caption text-muted-foreground" title={item.description.replaceAll("`", "")}><Prose text={item.description} /></span>
          </button>
          <span className="w-[4.5rem] shrink-0 text-right text-ui font-medium tabular-nums text-foreground"><Size entry={item} /></span>
          <span className="flex shrink-0 items-center">
            {onAsk && <IconAction label={`Ask about ${item.label}`} onClick={() => onAsk(item)}><MessageCircle size={14} aria-hidden="true" /></IconAction>}
            {item.protectedReason
              ? <IconAction label={`Choose what to remove from ${item.label}`} onClick={() => onOpen(item.path)}><FolderOpen size={14} aria-hidden="true" /></IconAction>
              : <IconAction label={`Move ${item.label} to the Trash`} tone="destructive" disabled={busy || !item.sizeBytes} onClick={() => onDelete(suggestionEntry(item))}><Trash2 size={14} aria-hidden="true" /></IconAction>}
          </span>
        </li>)}
      </ul>
      {rows.length > SUGGESTIONS_SHOWN && <button type="button" onClick={() => setAll(value => !value)} className="flex w-full items-center justify-center gap-1 border-t border-border/60 py-2 text-caption text-muted-foreground outline-none transition-colors hover:bg-accent hover:text-foreground focus-visible:bg-accent">
        {all ? "Show fewer" : `Show all ${rows.length}`}
        <ChevronDown size={13} aria-hidden="true" className={cn("transition-transform", all && "rotate-180")} />
      </button>}
    </div>
  </section>;
}

function Breadcrumb({ path, home, onOpen }: { path: string; home: string | undefined; onOpen: (path: string) => void }) {
  const inHome = home && (path === home || path.startsWith(`${home}/`));
  const base = inHome ? home : "";
  const rest = path.slice(base.length).split("/").filter(Boolean);
  const crumbs = [{ label: inHome ? "Home" : "Macintosh HD", path: base || "/" }, ...rest.map((part, index) => ({ label: part, path: `${base}/${rest.slice(0, index + 1).join("/")}` }))];
  return <nav aria-label="Folder" className="flex min-w-0 flex-wrap items-center gap-1 text-caption">
    {crumbs.map((crumb, index) => <span key={crumb.path} className="flex items-center gap-1">
      {index > 0 && <ChevronRight size={12} aria-hidden="true" className="text-muted-foreground/60" />}
      {index === crumbs.length - 1
        ? <span className="font-medium text-foreground">{crumb.label}</span>
        : <button type="button" onClick={() => onOpen(crumb.path)} className="rounded text-muted-foreground outline-none transition-colors hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring">{crumb.label}</button>}
    </span>)}
  </nav>;
}

const EXPLORER_ROW = "grid grid-cols-[auto_minmax(0,1fr)_5.5rem_4rem] items-center gap-3 pl-4 pr-2.5 sm:grid-cols-[auto_minmax(0,1fr)_6rem_5.5rem_4rem]";

function Explorer({ listing, overview, root, selected, busy, sectionRef, onRoot, onOpen, onToggle, onDelete, onAsk, onRefresh }: {
  listing: DiskListing | null;
  overview: DiskOverview | null;
  root: string | null;
  selected: ReadonlyMap<string, DiskEntry>;
  busy: boolean;
  sectionRef: RefObject<HTMLElement>;
  onRoot: (path: string | null) => void;
  onOpen: (path: string) => void;
  onToggle: (entry: DiskEntry) => void;
  onDelete: (entry: DiskEntry) => void;
  onAsk?: (entry: DiskEntry) => void;
  onRefresh: () => void;
}) {
  const largest = Math.max(...(listing?.entries ?? []).map(entry => entry.sizeBytes ?? 0), 1);
  // At the top of the disk, what the walk cannot see (the sealed system
  // volume's share, snapshots, purgeable space) is still part of "used".
  const unseen = listing?.path === "/" && !listing.measuring && overview?.volume
    ? Math.max(overview.volume.usedBytes - listing.sizeBytes, 0)
    : 0;
  return <section ref={sectionRef} aria-label="Everything on this Mac" className="scroll-mt-6">
    <SectionHeading
      title="Everything on this Mac"
      detail={listing ? diskBytes(listing.sizeBytes) : undefined}
      action={<>
        <span role="group" aria-label="Start from" className="flex rounded-lg bg-muted p-0.5 text-caption">
          {ROOTS.map(item => <button
            key={item.label}
            type="button"
            aria-pressed={(root ?? null) === item.path}
            onClick={() => onRoot(item.path)}
            className={cn("rounded-md px-2.5 py-1 outline-none transition-colors focus-visible:ring-2 focus-visible:ring-ring", (root ?? null) === item.path ? "bg-background text-foreground shadow-sm" : "text-muted-foreground hover:text-foreground")}
          >{item.label}</button>)}
        </span>
        <IconAction label="Measure this folder again" onClick={onRefresh} disabled={busy}><RefreshCw size={13} aria-hidden="true" /></IconAction>
      </>}
    />
    <div className={cn(TILE, "overflow-hidden")}>
      <div className="flex items-center gap-2 border-b border-border/60 py-2.5 pl-4 pr-3">
        {listing ? <Breadcrumb path={listing.path} home={overview?.home} onOpen={onOpen} /> : <span role="status" aria-label="Measuring" className={cn(PLACEHOLDER, "w-[9em] text-caption")} />}
      </div>
      {listing?.unreadable && <p className="px-4 pt-3 text-caption text-muted-foreground">{listing.unreadable}</p>}
      <ul className="divide-y divide-border/50">
        {listing?.entries.map(entry => {
          const folder = entry.kind === "directory" || entry.kind === "package";
          const checked = selected.has(entry.path);
          return <li key={entry.path} className={cn("group py-2", EXPLORER_ROW, checked && "bg-accent/60")}>
            <button
              type="button"
              role="checkbox"
              aria-label={`Select ${entry.name}`}
              aria-checked={checked}
              disabled={Boolean(entry.protectedReason)}
              title={entry.protectedReason ?? undefined}
              onClick={() => onToggle(entry)}
              className={cn("grid size-4 shrink-0 place-items-center rounded-[5px] outline-none ring-1 ring-inset transition-colors focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-30", checked ? "bg-foreground ring-foreground" : "ring-border hover:ring-muted-foreground")}
            >
              {checked && <Check size={11} strokeWidth={3} aria-hidden="true" className="text-background" />}
            </button>
            <div className="min-w-0">
              <span className="flex min-w-0 items-center gap-1.5">
                {folder ? <Folder size={13} aria-hidden="true" className="shrink-0 text-muted-foreground" /> : <File size={13} aria-hidden="true" className="shrink-0 text-muted-foreground" />}
                {folder
                  ? <button type="button" onClick={() => onOpen(entry.path)} className="min-w-0 truncate rounded text-left text-ui text-foreground outline-none hover:underline focus-visible:ring-2 focus-visible:ring-ring">{entry.name}</button>
                  : <span className="min-w-0 truncate text-ui text-foreground">{entry.name}</span>}
              </span>
              <span className="mt-1 block h-1 rounded-full bg-muted">
                <span className="block h-full rounded-full bg-foreground/30" style={{ width: `${((entry.sizeBytes ?? 0) / largest) * 100}%` }} />
              </span>
            </div>
            <span className="hidden text-right text-caption tabular-nums text-muted-foreground sm:block">
              {folder && entry.itemCount ? `${entry.itemCount.toLocaleString()} item${entry.itemCount === 1 ? "" : "s"}` : ""}
            </span>
            <span className="text-right text-ui tabular-nums text-foreground"><Size entry={entry} /></span>
            <span className="flex justify-end">
              {entry.protectedReason
                ? <span className="flex items-center gap-1 text-caption text-muted-foreground" title={entry.protectedReason}><Lock size={12} aria-hidden="true" /><span className="sr-only">Protected</span></span>
                : <span className="flex opacity-0 transition-opacity group-focus-within:opacity-100 group-hover:opacity-100">
                  {onAsk && <IconAction label={`Ask about ${entry.name}`} onClick={() => onAsk(entry)}><MessageCircle size={14} aria-hidden="true" /></IconAction>}
                  <IconAction label={`Move ${entry.name} to the Trash`} tone="destructive" disabled={busy} onClick={() => onDelete(entry)}><Trash2 size={14} aria-hidden="true" /></IconAction>
                </span>}
            </span>
          </li>;
        })}
        {listing && listing.omittedCount > 0 && <li className={cn("py-2 text-caption text-muted-foreground", EXPLORER_ROW)}>
          <span className="size-4" />
          <span>{listing.omittedCount.toLocaleString()} smaller items</span>
          <span className="hidden sm:block" />
          <span className="text-right tabular-nums">{diskBytes(listing.omittedBytes)}</span>
          <span />
        </li>}
        {unseen > 0 && <li className={cn("py-2 text-caption text-muted-foreground", EXPLORER_ROW)}>
          <span className="size-4" />
          <span>System volume, snapshots, and space macOS can purge</span>
          <span className="hidden sm:block" />
          <span className="text-right tabular-nums">{diskBytes(unseen)}</span>
          <span />
        </li>}
      </ul>
      {listing && !listing.unreadable && listing.entries.length === 0 && <p className="py-6 text-center text-caption text-muted-foreground">This folder is empty.</p>}
    </div>
  </section>;
}

/** The one confirmation for every delete and for Empty Trash. A modal, so it
 *  appears where the person is looking whatever they clicked, and it stays
 *  open with a working state until the result is in. */
function ConfirmDialog({ confirming, busy, home, onCancel, onPermanent, onRemove, onEmptyTrash }: {
  confirming: Confirming;
  busy: boolean;
  home: string | undefined;
  onCancel: () => void;
  onPermanent: (permanent: boolean) => void;
  onRemove: (entries: DiskEntry[], permanent: boolean) => void;
  onEmptyTrash: () => void;
}) {
  // Keep the last content on screen while the dialog animates closed.
  const last = useRef(confirming);
  if (confirming) last.current = confirming;
  const shown = confirming ?? last.current;
  if (!shown) return null;
  const emptying = shown === "empty-trash";
  const entries = emptying ? [] : shown.entries;
  const permanent = !emptying && shown.permanent;
  const total = entries.reduce((sum, entry) => sum + (entry.sizeBytes ?? 0), 0);
  const subject = entries.length === 1 ? entries[0].name : `${entries.length} items`;
  const title = emptying ? "Empty the Trash?" : permanent ? `Delete ${subject} permanently?` : `Move ${subject} to the Trash?`;
  const description = emptying
    ? "Everything in it is deleted for good. macOS may ask once to let Bridge use Finder."
    : permanent
      ? "This cannot be undone. Nothing goes to the Trash."
      : "You can put it back from the Trash until you empty it. The space comes back when you do.";
  const spinner = <LoaderCircle size={14} aria-hidden="true" className="animate-spin" />;
  return <Dialog open={confirming !== null} onOpenChange={open => { if (!open && !busy) onCancel(); }}>
    <DialogContent aria-label="Confirm deletion" className="max-w-md" showCloseButton={false}>
      <DialogHeader>
        <span className={cn("grid size-9 place-items-center rounded-xl", emptying || permanent ? "bg-destructive/10 text-destructive" : "bg-muted text-foreground")}><Trash2 size={16} aria-hidden="true" /></span>
        <DialogTitle className="mt-1 text-lg leading-snug">{title}</DialogTitle>
        <DialogDescription>{description}</DialogDescription>
      </DialogHeader>
      {entries.length > 0 && <DialogPanel scrollFade={false}>
        <ul className="divide-y divide-border/60 overflow-hidden rounded-xl bg-muted/50 text-caption">
          {entries.slice(0, 6).map(entry => <li key={entry.path} className="flex items-center gap-3 px-3 py-2">
            <span className="min-w-0 flex-1">
              <span className="block truncate text-foreground">{entry.name}</span>
              <span className="block truncate font-mono text-[11px] text-muted-foreground">{displayPath(entry.path, home)}</span>
            </span>
            <span className="shrink-0 tabular-nums text-muted-foreground">{diskBytes(entry.sizeBytes)}</span>
          </li>)}
          {entries.length > 6 && <li className="px-3 py-2 text-muted-foreground">and {entries.length - 6} more</li>}
          {entries.length > 1 && total > 0 && <li className="flex justify-between px-3 py-2 font-medium text-foreground"><span>Total</span><span className="tabular-nums">{diskBytes(total)}</span></li>}
        </ul>
      </DialogPanel>}
      <DialogFooter variant="bare" className="items-center">
        {emptying
          ? <>
            <Button variant="ghost" disabled={busy} onClick={onCancel}>Cancel</Button>
            <Button variant="destructive" disabled={busy} onClick={onEmptyTrash}>{busy && spinner}{busy ? "Emptying…" : "Empty Trash"}</Button>
          </>
          : permanent
            ? <>
              <Button variant="ghost" className="mr-auto" disabled={busy} onClick={() => onPermanent(false)}>Back</Button>
              <Button variant="ghost" disabled={busy} onClick={onCancel}>Cancel</Button>
              <Button variant="destructive" disabled={busy} onClick={() => onRemove(entries, true)}>{busy && spinner}{busy ? "Deleting…" : "Delete permanently"}</Button>
            </>
            : <>
              <Button variant="ghost" className="mr-auto text-destructive hover:text-destructive" disabled={busy} onClick={() => onPermanent(true)}>Delete permanently…</Button>
              <Button variant="ghost" disabled={busy} onClick={onCancel}>Cancel</Button>
              <Button disabled={busy} onClick={() => onRemove(entries, false)}>{busy && spinner}{busy ? "Moving…" : "Move to Trash"}</Button>
            </>}
      </DialogFooter>
    </DialogContent>
  </Dialog>;
}

function OutcomeToast({ outcome, busy, onEmptyTrash, onDismiss }: { outcome: Outcome; busy: boolean; onEmptyTrash: () => void; onDismiss: () => void }) {
  const failed = outcome.failures.length > 0;
  return <div role={failed ? "alert" : "status"} aria-label="Result" className="pointer-events-auto w-full max-w-xl rounded-2xl bg-popover py-3 pl-4 pr-2 text-caption shadow-lg ring-1 ring-border/60">
    <div className="flex items-start gap-2.5">
      {failed
        ? <CircleAlert size={16} aria-hidden="true" className="mt-px shrink-0 text-destructive" />
        : <CircleCheck size={16} aria-hidden="true" className="mt-px shrink-0 text-success" />}
      <div className="min-w-0 flex-1">
        <p className="text-ui text-foreground">{outcome.summary}</p>
        {failed && <ul className="mt-1.5 space-y-1">
          {outcome.failures.map(failure => <li key={failure.name + failure.reason} className="leading-relaxed">
            <span className="font-medium text-foreground">{failure.name}</span> <span className="text-muted-foreground">{failure.reason}</span>
          </li>)}
        </ul>}
      </div>
      {outcome.offerEmptyTrash && <GhostButton onClick={onEmptyTrash} disabled={busy}>Empty Trash</GhostButton>}
      <IconAction label="Dismiss" onClick={onDismiss}><X size={14} aria-hidden="true" /></IconAction>
    </div>
  </div>;
}

/** A visit remeasures, but not on every hop between settings pages. */
const VISIT_REFRESH_MS = 60_000;
let lastVisitRefresh = 0;

export function StoragePage({ onError, title = "Storage", extra, copilot }: {
  onError?: (message: string) => void;
  title?: string;
  extra?: ReactNode;
  /** The docked storage chat. Without it there is no copilot. */
  copilot?: StorageCopilotHost;
}) {
  const [overview, setOverview] = useState<DiskOverview | null>(null);
  const [home, setHome] = useState<DiskListing | null>(null);
  const [listing, setListing] = useState<DiskListing | null>(null);
  const [path, setPath] = useState<string | null>(null);
  const [root, setRoot] = useState<string | null>(null);
  const [selected, setSelected] = useState<Map<string, DiskEntry>>(new Map());
  const [confirming, setConfirming] = useState<Confirming>(null);
  const [busy, setBusy] = useState(false);
  const [outcome, setOutcome] = useState<Outcome | null>(null);
  const [error, setError] = useState<string>();
  const explorerRef = useRef<HTMLElement>(null);
  const pathRef = useRef(path);
  pathRef.current = path;
  const [railWidth, setRailWidth] = useState(storedRailWidth);
  const gridRef = useRef<HTMLDivElement>(null);
  const railRef = useRef<HTMLDivElement>(null);
  const railId = useId();
  useEffect(() => { localStorage.setItem(RAIL_KEY, String(railWidth)); }, [railWidth]);

  const fail = useCallback((caught: unknown) => {
    setError(message(caught));
    onError?.(message(caught));
  }, [onError]);

  const load = useCallback(async (target: string | null, refresh = false) => {
    try {
      // The home folder holds every suggestion, so a refresh remeasures it
      // too. The last numbers stay on screen until the new ones land.
      if (refresh && target !== null) await api.scanDirectory(null, true);
      const [nextOverview, nextListing] = await Promise.all([api.storageOverview(), api.scanDirectory(target, refresh)]);
      // A slow answer for a folder the person already left must not land.
      if (pathRef.current !== target) return;
      setOverview(nextOverview);
      setListing(nextListing);
      if (nextListing.path === nextOverview.home) setHome(nextListing);
      else if (!home || home.measuring) setHome(await api.scanDirectory(null));
    } catch (caught) { fail(caught); }
  }, [fail, home]);

  // eslint-disable-next-line react-hooks/exhaustive-deps -- once per visit
  useEffect(() => { copilot?.onVisit?.(); }, []);
  const visitRefresh = useRef(Date.now() - lastVisitRefresh > VISIT_REFRESH_MS);
  useEffect(() => {
    const refresh = visitRefresh.current;
    visitRefresh.current = false;
    if (refresh) lastVisitRefresh = Date.now();
    void load(path, refresh);
  }, [path]); // eslint-disable-line react-hooks/exhaustive-deps -- reload on navigation only

  // The storage agent runs its own cleanup commands; when its turn ends,
  // what it removed has to show.
  const agentWorking = copilot?.working ?? false;
  const wasWorking = useRef(agentWorking);
  useEffect(() => {
    if (wasWorking.current && !agentWorking) void load(pathRef.current, true);
    wasWorking.current = agentWorking;
  }, [agentWorking, load]);

  const measuring = Boolean(overview?.measuring || listing?.measuring || home?.measuring);
  useEffect(() => {
    if (!measuring) return;
    const timer = window.setTimeout(() => void load(pathRef.current), POLL_MS);
    return () => window.clearTimeout(timer);
  }, [measuring, overview, listing, home, load]);

  const open = (next: string | null) => {
    setPath(next);
    setConfirming(null);
  };
  /** Open a folder from elsewhere on the page and bring the list into view. */
  const reveal = (next: string) => {
    open(next);
    explorerRef.current?.scrollIntoView?.({ behavior: "smooth", block: "start" });
  };
  const toggle = (entry: DiskEntry) => setSelected(current => {
    const next = new Map(current);
    if (next.has(entry.path)) next.delete(entry.path); else next.set(entry.path, entry);
    return next;
  });
  const selection = useMemo(() => [...selected.values()], [selected]);
  const selectedBytes = selection.reduce((total, entry) => total + (entry.sizeBytes ?? 0), 0);

  // The dialog stays open, working, until the answer is in; then the result
  // shows at the bottom of the page.
  const remove = async (entries: DiskEntry[], permanent: boolean) => {
    setBusy(true); setError(undefined); setOutcome(null);
    try {
      const result = await api.deletePaths(entries.map(entry => entry.path), permanent);
      setOutcome(outcomeOf(result, entries));
      setSelected(current => {
        const next = new Map(current);
        for (const deleted of result.deleted) next.delete(deleted);
        return next;
      });
      setConfirming(null);
      await load(pathRef.current);
    } catch (caught) { setConfirming(null); fail(caught); } finally { setBusy(false); }
  };

  const emptyTrash = async () => {
    setBusy(true); setError(undefined); setOutcome(null);
    try {
      const result = await api.emptyTrash();
      setOutcome(result.emptied
        ? { summary: "The Trash is empty.", failures: [], offerEmptyTrash: false }
        : { summary: "The Trash was not emptied.", failures: [{ name: "Trash", reason: result.detail ?? "Finder did not say why." }], offerEmptyTrash: false });
      setConfirming(null);
      await load(pathRef.current);
    } catch (caught) { setConfirming(null); fail(caught); } finally { setBusy(false); }
  };

  // Each message carries what the page measured, but only when that changed
  // since the last one: the agent always has current numbers and the chat
  // never repeats them. The agent's brief is its system prompt, not this.
  const lastSnapshot = useRef("");
  const [trashedByAgent, setTrashedByAgent] = useState<{ path: string; sizeBytes: number | null }[]>([]);
  const unreported = useRef<{ path: string; sizeBytes: number | null }[]>([]);
  const brief = (question: string) => {
    const snapshot = storageSnapshot({ overview, listing, home, selected: selection, trashed: unreported.current });
    if (snapshot === lastSnapshot.current) return question.trim();
    lastSnapshot.current = snapshot;
    unreported.current = [];
    return withSnapshot(question, snapshot);
  };
  const ask = (question: string) => copilot?.ask(brief(question));
  const askAbout = (name: string, path: string, size: number | null | undefined) =>
    ask(`What is ${name} (${displayPath(path, overview?.home)}, ${diskBytes(size)})? Can I get that space back, and how?`);

  // What a ```storage-plan card may do on this page.
  const trashedPaths = useMemo(() => new Set(trashedByAgent.map(item => item.path)), [trashedByAgent]);
  const agentHost: StorageAgentHost = {
    home: overview?.home,
    busy,
    trashed: trashedPaths,
    trash: async (items: StoragePlanItem[]) => {
      setBusy(true); setError(undefined);
      try {
        const result = await api.deletePaths(items.map(item => item.path), false);
        const moved = items.filter(item => result.deleted.includes(item.path)).map(item => ({ path: item.path, sizeBytes: item.sizeBytes }));
        setTrashedByAgent(current => [...current, ...moved]);
        unreported.current = [...unreported.current, ...moved];
        setSelected(current => {
          const next = new Map(current);
          for (const deleted of result.deleted) next.delete(deleted);
          return next;
        });
        await load(pathRef.current);
        const done = moved.length === 0 ? "" : `Moved ${moved.length} item${moved.length === 1 ? "" : "s"}${result.bytesFreed ? `, ${diskBytes(result.bytesFreed)},` : ""} to the Trash. Empty the Trash to get the space back.`;
        const refused = result.failed.map(failure => `${failure.path.split("/").pop()}: ${failure.reason}`).join(" ");
        return [done, refused].filter(Boolean).join(" ") || "Nothing was removed.";
      } catch (caught) { fail(caught); return message(caught); } finally { setBusy(false); }
    },
    select: items => setSelected(current => {
      const next = new Map(current);
      for (const item of items) next.set(item.path, { name: item.path.split("/").pop() ?? item.path, path: item.path, kind: "directory", sizeBytes: item.sizeBytes, itemCount: null, measuring: false, partial: false, modifiedAt: null, protectedReason: null });
      return next;
    }),
    reveal: target => reveal(target.slice(0, target.lastIndexOf("/")) || "/"),
    reply: text => { ask(text); },
  };

  return <div data-settings-column className="@container/settings mx-auto w-full max-w-page-wide px-5 pb-16 pt-6 sm:px-8">
    <div
      ref={gridRef}
      // The rail column takes the dragged width, but never less than RAIL_MIN
      // (20rem) and never so much that the listing drops under LISTING_MIN
      // plus the gutter (25rem + 2rem).
      className={cn("grid gap-8", copilot && "lg:grid-cols-[minmax(0,1fr)_2rem_clamp(20rem,var(--storage-rail),calc(100%_-_27rem))] lg:gap-x-0")}
      style={copilot ? { "--storage-rail": `${railWidth}px` } as CSSProperties : undefined}
    >
      <div className="@container/storage min-w-0 space-y-8">
        <header className="flex flex-wrap items-end gap-x-4 gap-y-1">
          <div className="min-w-0 flex-1">
            <h2 className="font-display text-title font-semibold leading-tight tracking-tight text-foreground">{title}</h2>
            <p className="mt-1 text-ui leading-relaxed text-muted-foreground">What is using space on this Mac, and what you can let go of. Deleting moves to the Trash first.</p>
          </div>
          {measuring && overview && <span role="status" className="flex items-center gap-1.5 pb-0.5 text-caption text-muted-foreground">
            <LoaderCircle size={12} aria-hidden="true" className="animate-spin" />Updating sizes
          </span>}
        </header>
        {error && <p role="alert" className="text-caption text-destructive">{error}</p>}
        {!overview && !error && <div role="status" aria-label="Looking at your disk" className={cn(TILE, "h-40 motion-safe:animate-pulse")} />}
        <DiskSummary overview={overview} home={home} onOpen={reveal} onEmptyTrash={() => setConfirming("empty-trash")} busy={busy} />

        {overview && <Suggestions overview={overview} onOpen={reveal} onDelete={entry => setConfirming({ entries: [entry], permanent: false })} onAsk={copilot ? item => askAbout(item.label, item.path, item.sizeBytes) : undefined} busy={busy} />}

        <Explorer
          listing={listing}
          overview={overview}
          root={root}
          selected={selected}
          busy={busy}
          sectionRef={explorerRef}
          onRoot={next => { setRoot(next); open(next); }}
          onOpen={open}
          onToggle={toggle}
          onDelete={entry => setConfirming({ entries: [entry], permanent: false })}
          onAsk={copilot ? entry => askAbout(entry.name, entry.path, entry.sizeBytes) : undefined}
          onRefresh={() => void load(path, true)}
        />

        <WorktreeStorage onError={onError} />
        {extra}
        {/* Results and the selection stay in reach at the bottom of the list column. */}
        {(outcome || selection.length > 0) && <div className="pointer-events-none sticky bottom-4 z-10 -mt-2 flex flex-col items-center gap-2">
          {outcome && <OutcomeToast outcome={outcome} busy={busy} onEmptyTrash={() => setConfirming("empty-trash")} onDismiss={() => setOutcome(null)} />}
          {selection.length > 0 && <div role="region" aria-label="Selection" className="pointer-events-auto flex w-fit items-center gap-3 rounded-full bg-popover py-1.5 pl-4 pr-1.5 text-caption shadow-lg ring-1 ring-border/60">
            <span className="tabular-nums text-foreground">{selection.length} selected · {diskBytes(selectedBytes)}</span>
            <TextButton onClick={() => setSelected(new Map())}>Clear</TextButton>
            {copilot && <TextButton onClick={() => ask(`Tell me what the ${selection.length} item${selection.length === 1 ? "" : "s"} I selected are, and whether I can delete them.`)}>Ask the agent</TextButton>}
            <PrimaryButton disabled={busy} onClick={() => setConfirming({ entries: selection, permanent: false })}>Move to Trash</PrimaryButton>
          </div>}
        </div>}
      </div>
      {copilot && <RailDivider width={railWidth} railId={railId} gridRef={gridRef} railRef={railRef} onResize={setRailWidth} />}
      {copilot && <div ref={railRef} id={railId} className="min-w-0">
        <StorageAgentContext.Provider value={agentHost}>{copilot.render(brief, selection.length)}</StorageAgentContext.Provider>
      </div>}
    </div>

    <ConfirmDialog
      confirming={confirming}
      busy={busy}
      home={overview?.home}
      onCancel={() => setConfirming(null)}
      onPermanent={permanent => setConfirming(current => current && current !== "empty-trash" ? { ...current, permanent } : current)}
      onRemove={(entries, permanent) => void remove(entries, permanent)}
      onEmptyTrash={() => void emptyTrash()}
    />
  </div>;
}
