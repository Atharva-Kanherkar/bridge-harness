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
// ```storage-plan cards the person approves here (`StoragePlanCard`).

import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { FolderOpen, MessageCircle, Trash2 } from "lucide-react";
import { cn } from "@/lib/utils";
import { bridgeApi as api } from "../../api";
import type { DiskEntry, DiskListing, DiskOverview, DiskSuggestion } from "../../types";
import { cleanupTotal, diskBytes, displayPath, storageSnapshot, withSnapshot, type StoragePlanItem } from "../../diskSpace";
import { StorageAgentContext, type StorageAgentHost } from "../StoragePlanCard";
import { GhostButton, PrimaryButton, StatusPill, TextButton } from "./kit";
import type { StorageCopilotHost } from "./StorageCopilot";
import { WorktreeStorage } from "./WorktreeStorage";

const POLL_MS = 1500;

/** The page's one categorical palette: the validated six-hue ramp the context
 *  lens already wears, in rank order, so the biggest thing is always the same
 *  blue. Chrome stays achromatic; only data marks are coloured. */
const SERIES = ["bg-ctx-1", "bg-ctx-2", "bg-ctx-3", "bg-ctx-4", "bg-ctx-5", "bg-ctx-6"];
const SERIES_WASH = ["bg-ctx-1/12", "bg-ctx-2/12", "bg-ctx-3/12", "bg-ctx-4/12", "bg-ctx-5/12", "bg-ctx-6/12"];
const OTHER = "bg-foreground/15";
/** Suggestion groups wear a fixed series so a card's colour means its kind. */
const GROUP_SERIES: Record<string, number> = { developer: 0, caches: 1, files: 3 };
const groupSeries = (group: string) => GROUP_SERIES[group] ?? 2;
const GROUP_LABEL: Record<string, string> = { developer: "Developer", caches: "Cache", files: "Files" };

const TILE = "rounded-xl border border-border bg-card";

const ROOTS = [
  { label: "Home", path: null },
  { label: "Applications", path: "/Applications" },
  { label: "Whole disk", path: "/" },
] as const;


type Confirming = { entries: DiskEntry[]; permanent: boolean } | "empty-trash" | null;

function message(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function SectionHeading({ title, detail, action }: { title: string; detail?: ReactNode; action?: ReactNode }) {
  return <header className="mb-3 flex flex-wrap items-baseline gap-x-3 gap-y-1">
    <h3 className="text-ui font-medium text-foreground">{title}</h3>
    {detail && <span className="text-caption text-muted-foreground">{detail}</span>}
    {action && <span className="ml-auto flex items-center gap-1">{action}</span>}
  </header>;
}

function Size({ entry }: { entry: Pick<DiskEntry, "sizeBytes" | "measuring"> & { partial?: boolean } }) {
  if (entry.sizeBytes === null || entry.sizeBytes === undefined) {
    return <span className="text-muted-foreground motion-safe:animate-pulse">{entry.measuring ? "Measuring" : "—"}</span>;
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
    className={cn("grid size-7 place-items-center rounded-md outline-none transition-colors focus-visible:ring-2 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-30", tone === "destructive" ? "text-muted-foreground hover:bg-destructive/10 hover:text-destructive" : "text-muted-foreground hover:bg-accent hover:text-foreground")}
  >{children}</button>;

/** Free space, and what fills the rest: two tiles side by side. The home
 *  folder's biggest children each wear a series colour; everything else the
 *  volume reports as used is one quiet segment. */
function DiskSummary({ overview, home, onOpen, onEmptyTrash, busy }: {
  overview: DiskOverview | null;
  home: DiskListing | null;
  onOpen: (path: string) => void;
  onEmptyTrash: () => void;
  busy: boolean;
}) {
  const volume = overview?.volume;
  if (!volume) return null;
  const top = (home?.entries ?? []).filter(entry => (entry.sizeBytes ?? 0) > 0).slice(0, SERIES.length);
  const named = top.reduce((total, entry) => total + (entry.sizeBytes ?? 0), 0);
  const other = Math.max(volume.usedBytes - named, 0);
  const share = (value: number) => `${Math.max((value / volume.totalBytes) * 100, value > 0 ? 0.4 : 0)}%`;
  const percent = (value: number) => `${Math.round((value / volume.totalBytes) * 100)}%`;
  const usedShare = volume.usedBytes / volume.totalBytes;
  const safe = cleanupTotal(overview.suggestions, "safe");
  return <section aria-label="Disk" className="grid gap-3 @2xl/storage:grid-cols-[minmax(0,1fr)_minmax(0,2fr)]">
    <div className={cn(TILE, "flex flex-col p-4")}>
      <p className="text-caption text-muted-foreground">Free</p>
      <p className="mt-1 flex flex-wrap items-baseline gap-x-2">
        <span className="font-display text-4xl font-semibold tabular-nums tracking-tight text-foreground">{diskBytes(volume.freeBytes)}</span>
        <span className="text-ui text-muted-foreground">free of {diskBytes(volume.totalBytes)}</span>
      </p>
      <div className="mt-3 flex flex-wrap items-center gap-2">
        {usedShare > 0.9 ? <StatusPill tone="warning">Nearly full</StatusPill> : <StatusPill tone="success">{percent(volume.freeBytes)} free</StatusPill>}
        {safe > 0 && <StatusPill tone="info">{diskBytes(safe)} rebuildable</StatusPill>}
      </div>
      <div className="mt-auto pt-4"><GhostButton onClick={onEmptyTrash} disabled={busy}>Empty Trash</GhostButton></div>
    </div>
    <div className={cn(TILE, "p-4")}>
      <p className="text-caption text-muted-foreground">What fills it</p>
      <div className="mt-3 flex h-3 w-full gap-0.5 overflow-hidden rounded-full bg-muted" role="img" aria-label={`${diskBytes(volume.usedBytes)} used of ${diskBytes(volume.totalBytes)}`}>
        {top.map((entry, index) => <span key={entry.path} className={cn("h-full origin-left first:rounded-l-full motion-safe:animate-[meter-fill_600ms_ease-out]", SERIES[index])} style={{ width: share(entry.sizeBytes ?? 0) }} />)}
        <span className={cn("h-full", OTHER)} style={{ width: share(other) }} />
      </div>
      <ul className="mt-3 grid grid-cols-1 gap-1 @md/storage:grid-cols-2">
        {top.map((entry, index) => <li key={entry.path}>
          <button type="button" onClick={() => onOpen(entry.path)} className="flex w-full items-center gap-2 rounded-md px-1.5 py-1 text-left text-caption outline-none transition-colors hover:bg-accent focus-visible:ring-2 focus-visible:ring-ring">
            <span className={cn("size-2.5 shrink-0 rounded-[3px]", SERIES[index])} />
            <span className="min-w-0 flex-1 truncate text-foreground">{entry.name}</span>
            <span className="shrink-0 tabular-nums text-muted-foreground">{diskBytes(entry.sizeBytes)}</span>
          </button>
        </li>)}
        <li className="flex items-center gap-2 px-1.5 py-1 text-caption">
          <span className={cn("size-2.5 shrink-0 rounded-[3px]", OTHER)} />
          <span className="min-w-0 flex-1 truncate text-foreground">Apps, system, and other</span>
          <span className="shrink-0 tabular-nums text-muted-foreground">{diskBytes(other)}</span>
        </li>
      </ul>
    </div>
  </section>;
}

function suggestionEntry(item: DiskSuggestion): DiskEntry {
  return { name: item.label, path: item.path, kind: "directory", sizeBytes: item.sizeBytes, itemCount: null, measuring: item.measuring, partial: false, modifiedAt: null, protectedReason: null };
}

/** Known cleanup candidates as a grid of cards: colour says what kind of
 *  thing it is, the pill says whether it comes back on its own. */
function Suggestions({ overview, onOpen, onDelete, onAsk, busy }: {
  overview: DiskOverview;
  onOpen: (path: string) => void;
  onDelete: (entry: DiskEntry) => void;
  onAsk?: (item: DiskSuggestion) => void;
  busy: boolean;
}) {
  const rows = [...overview.suggestions].sort((a, b) => (b.sizeBytes ?? -1) - (a.sizeBytes ?? -1));
  if (rows.length === 0) return null;
  const safe = cleanupTotal(overview.suggestions, "safe");
  const review = cleanupTotal(overview.suggestions, "review");
  return <section aria-label="Cleanup suggestions">
    <SectionHeading title="Worth a look" detail={safe > 0 ? <><span className="text-success">{diskBytes(safe)}</span> rebuilds itself on demand{review > 0 ? <>, {diskBytes(review)} to review</> : null}</> : undefined} />
    <ul className="grid grid-cols-1 gap-3 @md/storage:grid-cols-2 @4xl/storage:grid-cols-3">
      {rows.map(item => {
        const series = groupSeries(item.group);
        return <li key={item.id} className={cn(TILE, "group relative flex flex-col overflow-hidden p-3.5 transition-colors hover:border-muted-foreground/40")}>
          <span aria-hidden="true" className={cn("absolute inset-y-0 left-0 w-1", SERIES[series])} />
          <div className="flex items-start gap-2">
            <span className={cn("rounded-md px-1.5 py-0.5 text-[11px] leading-none text-foreground", SERIES_WASH[series])}>{GROUP_LABEL[item.group] ?? "Other"}</span>
            <span className="ml-auto whitespace-nowrap">{item.safety === "safe" ? <StatusPill tone="success">Rebuilds itself</StatusPill> : <StatusPill tone="warning">Review first</StatusPill>}</span>
          </div>
          <button type="button" onClick={() => onOpen(item.path)} className="mt-2 rounded text-left outline-none focus-visible:ring-2 focus-visible:ring-ring">
            <span className="block text-ui font-medium text-foreground">{item.label}</span>
            <span className="mt-0.5 line-clamp-2 text-caption text-muted-foreground">{item.description}</span>
          </button>
          <div className="mt-auto flex items-end gap-1 pt-3">
            <span className="mr-auto whitespace-nowrap font-display text-xl font-semibold tabular-nums tracking-tight text-foreground"><Size entry={item} /></span>
            {onAsk && <IconAction label={`Ask about ${item.label}`} onClick={() => onAsk(item)}><MessageCircle size={14} aria-hidden="true" /></IconAction>}
            <IconAction label={`Show ${item.label}`} onClick={() => onOpen(item.path)}><FolderOpen size={14} aria-hidden="true" /></IconAction>
            <IconAction label={`Move ${item.label} to the Trash`} tone="destructive" disabled={busy || !item.sizeBytes} onClick={() => onDelete(suggestionEntry(item))}><Trash2 size={14} aria-hidden="true" /></IconAction>
          </div>
        </li>;
      })}
    </ul>
  </section>;
}

function Breadcrumb({ path, home, onOpen }: { path: string; home: string | undefined; onOpen: (path: string) => void }) {
  const inHome = home && (path === home || path.startsWith(`${home}/`));
  const base = inHome ? home : "";
  const rest = path.slice(base.length).split("/").filter(Boolean);
  const crumbs = [{ label: inHome ? "Home" : "Macintosh HD", path: base || "/" }, ...rest.map((part, index) => ({ label: part, path: `${base}/${rest.slice(0, index + 1).join("/")}` }))];
  return <nav aria-label="Folder" className="flex min-w-0 flex-wrap items-center gap-1 text-caption">
    {crumbs.map((crumb, index) => <span key={crumb.path} className="flex items-center gap-1">
      {index > 0 && <span aria-hidden="true" className="text-muted-foreground/60">/</span>}
      {index === crumbs.length - 1
        ? <span className="text-foreground">{crumb.label}</span>
        : <button type="button" onClick={() => onOpen(crumb.path)} className="rounded text-muted-foreground outline-none transition-colors hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring">{crumb.label}</button>}
    </span>)}
  </nav>;
}

const EXPLORER_ROW = "grid grid-cols-[auto_minmax(0,1fr)_5.5rem_5.5rem] items-center gap-3 px-3 sm:grid-cols-[auto_minmax(0,1fr)_6rem_5.5rem_5.5rem]";

function Explorer({ listing, overview, root, selected, busy, onRoot, onOpen, onToggle, onDelete, onAsk, onRefresh }: {
  listing: DiskListing | null;
  overview: DiskOverview | null;
  root: string | null;
  selected: ReadonlyMap<string, DiskEntry>;
  busy: boolean;
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
  return <section aria-label="Everything on this Mac">
    <SectionHeading
      title="Everything on this Mac"
      detail={listing ? <>{diskBytes(listing.sizeBytes)}{listing.measuring ? " so far, still measuring" : ""}</> : undefined}
      action={<>
        <span role="group" aria-label="Start from" className="flex rounded-lg bg-muted p-0.5">
          {ROOTS.map(item => <button
            key={item.label}
            type="button"
            aria-pressed={(root ?? null) === item.path}
            onClick={() => onRoot(item.path)}
            className={cn("rounded-md px-2.5 py-1 text-caption outline-none transition-colors focus-visible:ring-2 focus-visible:ring-ring", (root ?? null) === item.path ? "bg-background text-foreground shadow-sm" : "text-muted-foreground hover:text-foreground")}
          >{item.label}</button>)}
        </span>
        <TextButton onClick={onRefresh} disabled={busy} ariaLabel="Measure this folder again">Remeasure</TextButton>
      </>}
    />
    <div className={cn(TILE, "overflow-hidden")}>
      <div className="border-b border-border px-3 py-2">{listing ? <Breadcrumb path={listing.path} home={overview?.home} onOpen={onOpen} /> : <span className="text-caption text-muted-foreground">Measuring…</span>}</div>
      {listing?.unreadable && <p className="px-3 pt-3 text-caption text-muted-foreground">{listing.unreadable}</p>}
      <ul className="divide-y divide-border/50">
        {listing?.entries.map((entry, index) => {
          const folder = entry.kind === "directory" || entry.kind === "package";
          const checked = selected.has(entry.path);
          return <li key={entry.path} className={cn("group py-2", EXPLORER_ROW, checked && "bg-accent/50")}>
            <button
              type="button"
              role="checkbox"
              aria-label={`Select ${entry.name}`}
              aria-checked={checked}
              disabled={Boolean(entry.protectedReason)}
              onClick={() => onToggle(entry)}
              className={cn("grid size-4 shrink-0 place-items-center rounded-[5px] outline-none ring-1 ring-inset transition-colors focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-30", checked ? "bg-foreground ring-foreground" : "ring-border hover:ring-muted-foreground")}
            >
              {checked && <span className="size-1.5 rounded-[2px] bg-background" />}
            </button>
            <div className="min-w-0">
              {folder
                ? <button type="button" onClick={() => onOpen(entry.path)} className="block max-w-full truncate rounded text-left text-ui text-foreground outline-none hover:underline focus-visible:ring-2 focus-visible:ring-ring">{entry.name}</button>
                : <span className="block truncate text-ui text-foreground">{entry.name}</span>}
              <span className="mt-1 block h-1 rounded-full bg-muted">
                <span className={cn("block h-full rounded-full", SERIES[index % SERIES.length])} style={{ width: `${((entry.sizeBytes ?? 0) / largest) * 100}%` }} />
              </span>
            </div>
            <span className="hidden text-right text-caption tabular-nums text-muted-foreground sm:block">
              {folder && entry.itemCount ? `${entry.itemCount.toLocaleString()} item${entry.itemCount === 1 ? "" : "s"}` : ""}
            </span>
            <span className="text-right text-ui tabular-nums text-foreground"><Size entry={entry} /></span>
            <span className="flex justify-end opacity-0 transition-opacity group-focus-within:opacity-100 group-hover:opacity-100">
              {entry.protectedReason
                ? <span className="truncate text-caption text-muted-foreground" title={entry.protectedReason}>Protected</span>
                : <>
                  {onAsk && <IconAction label={`Ask about ${entry.name}`} onClick={() => onAsk(entry)}><MessageCircle size={14} aria-hidden="true" /></IconAction>}
                  <IconAction label={`Move ${entry.name} to the Trash`} tone="destructive" disabled={busy} onClick={() => onDelete(entry)}><Trash2 size={14} aria-hidden="true" /></IconAction>
                </>}
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
  const [note, setNote] = useState<string | null>(null);
  const [error, setError] = useState<string>();
  const pathRef = useRef(path);
  pathRef.current = path;

  const fail = useCallback((caught: unknown) => {
    setError(message(caught));
    onError?.(message(caught));
  }, [onError]);

  const load = useCallback(async (target: string | null, refresh = false) => {
    try {
      const [nextOverview, nextListing] = await Promise.all([api.storageOverview(), api.scanDirectory(target, refresh)]);
      // A slow answer for a folder the person already left must not land.
      if (pathRef.current !== target) return;
      setOverview(nextOverview);
      setListing(nextListing);
      if (nextListing.path === nextOverview.home) setHome(nextListing);
      else if (!home || home.measuring) setHome(await api.scanDirectory(null));
    } catch (caught) { fail(caught); }
  }, [fail, home]);

  useEffect(() => { void load(path); }, [path]); // eslint-disable-line react-hooks/exhaustive-deps -- reload on navigation only

  const measuring = Boolean(overview?.measuring || listing?.measuring || home?.measuring);
  useEffect(() => {
    if (!measuring) return;
    const timer = window.setTimeout(() => void load(pathRef.current), POLL_MS);
    return () => window.clearTimeout(timer);
  }, [measuring, overview, listing, home, load]);

  const open = (next: string | null) => {
    setPath(next);
    setNote(null);
    setConfirming(null);
  };
  const toggle = (entry: DiskEntry) => setSelected(current => {
    const next = new Map(current);
    if (next.has(entry.path)) next.delete(entry.path); else next.set(entry.path, entry);
    return next;
  });
  const selection = useMemo(() => [...selected.values()], [selected]);
  const selectedBytes = selection.reduce((total, entry) => total + (entry.sizeBytes ?? 0), 0);

  const remove = async (entries: DiskEntry[], permanent: boolean) => {
    setConfirming(null); setBusy(true); setError(undefined); setNote(null);
    try {
      const result = await api.deletePaths(entries.map(entry => entry.path), permanent);
      const count = result.deleted.length;
      const done = count === 0 ? "" : `${result.trashed ? "Moved" : "Deleted"} ${count} item${count === 1 ? "" : "s"}${result.bytesFreed ? `, ${diskBytes(result.bytesFreed)}` : ""}${result.trashed ? " to the Trash. Empty the Trash to get the space back." : "."}`;
      const refused = result.failed.map(failure => `${failure.path.split("/").pop()}: ${failure.reason}`).join(" ");
      setNote([done, refused].filter(Boolean).join(" ") || "Nothing was removed.");
      setSelected(current => {
        const next = new Map(current);
        for (const deleted of result.deleted) next.delete(deleted);
        return next;
      });
      await load(pathRef.current);
    } catch (caught) { fail(caught); } finally { setBusy(false); }
  };

  const emptyTrash = async () => {
    setConfirming(null); setBusy(true); setError(undefined); setNote(null);
    try {
      const result = await api.emptyTrash();
      setNote(result.emptied ? "The Trash is empty." : result.detail ?? "The Trash was not emptied.");
      await load(pathRef.current);
    } catch (caught) { fail(caught); } finally { setBusy(false); }
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
    reveal: target => open(target.slice(0, target.lastIndexOf("/")) || "/"),
    reply: text => { ask(text); },
  };

  return <div data-settings-column className="@container/settings mx-auto w-full max-w-page-wide px-5 pb-16 pt-6 sm:px-8">
    <header className="mb-8">
      <h2 className="font-display text-title font-semibold leading-tight tracking-tight text-foreground">{title}</h2>
      <p className="mt-1 text-ui leading-relaxed text-muted-foreground">What is using space on this Mac, and what you can let go of. Deleting moves to the Trash first.</p>
    </header>
    <div className={cn("grid gap-8", copilot && "lg:grid-cols-[minmax(0,1fr)_24rem]")}>
      <div className="@container/storage min-w-0 space-y-10">
        {error && <p role="alert" className="text-caption text-destructive">{error}</p>}
        {!overview && !error && <p role="status" className="text-caption text-muted-foreground">Looking at your disk…</p>}
        <DiskSummary overview={overview} home={home} onOpen={open} onEmptyTrash={() => setConfirming("empty-trash")} busy={busy} />

        {note && <p role="status" className="-mt-6 text-caption text-muted-foreground">{note}</p>}
        {confirming && <div role="group" aria-label="Confirm deletion" className="-mt-6 rounded-lg bg-muted/50 px-4 py-3">
          {confirming === "empty-trash"
            ? <>
              <p className="text-ui font-medium text-foreground">Empty the Trash?</p>
              <p className="mt-1 text-caption text-muted-foreground">Everything in it is deleted for good. macOS may ask once to let Bridge use Finder.</p>
              <div className="mt-3 flex gap-2">
                <GhostButton onClick={() => setConfirming(null)}>Cancel</GhostButton>
                <TextButton tone="destructive" disabled={busy} onClick={() => void emptyTrash()}>Empty Trash</TextButton>
              </div>
            </>
            : <>
              <p className="text-ui font-medium text-foreground">
                {confirming.permanent ? "Delete" : "Move"} {confirming.entries.length === 1 ? confirming.entries[0].name : `${confirming.entries.length} items`}{confirming.permanent ? " permanently?" : " to the Trash?"}
              </p>
              <ul className="mt-1 space-y-0.5">
                {confirming.entries.slice(0, 5).map(entry => <li key={entry.path} className="flex gap-3 text-caption text-muted-foreground">
                  <span className="min-w-0 flex-1 truncate font-mono">{displayPath(entry.path, overview?.home)}</span>
                  <span className="shrink-0 tabular-nums">{diskBytes(entry.sizeBytes)}</span>
                </li>)}
                {confirming.entries.length > 5 && <li className="text-caption text-muted-foreground">and {confirming.entries.length - 5} more</li>}
              </ul>
              <p className="mt-2 text-caption text-muted-foreground">
                {confirming.permanent ? "This cannot be undone." : "You can put them back from the Trash until you empty it."}
              </p>
              <div className="mt-3 flex flex-wrap gap-2">
                <GhostButton onClick={() => setConfirming(null)}>Cancel</GhostButton>
                {confirming.permanent
                  ? <TextButton tone="destructive" disabled={busy} onClick={() => void remove(confirming.entries, true)}>Delete permanently</TextButton>
                  : <>
                    <PrimaryButton disabled={busy} onClick={() => void remove(confirming.entries, false)}>Move to Trash</PrimaryButton>
                    <TextButton tone="destructive" disabled={busy} onClick={() => setConfirming({ entries: confirming.entries, permanent: true })}>Delete permanently instead</TextButton>
                  </>}
              </div>
            </>}
        </div>}

        {overview && <Suggestions overview={overview} onOpen={open} onDelete={entry => setConfirming({ entries: [entry], permanent: false })} onAsk={copilot ? item => askAbout(item.label, item.path, item.sizeBytes) : undefined} busy={busy} />}

        <Explorer
          listing={listing}
          overview={overview}
          root={root}
          selected={selected}
          busy={busy}
          onRoot={next => { setRoot(next); open(next); }}
          onOpen={open}
          onToggle={toggle}
          onDelete={entry => setConfirming({ entries: [entry], permanent: false })}
          onAsk={copilot ? entry => askAbout(entry.name, entry.path, entry.sizeBytes) : undefined}
          onRefresh={() => void load(path, true)}
        />

        <WorktreeStorage onError={onError} />
        {extra}
      </div>
      {copilot && <StorageAgentContext.Provider value={agentHost}>{copilot.render(brief, selection.length)}</StorageAgentContext.Provider>}
    </div>

    {selection.length > 0 && <div role="region" aria-label="Selection" className="sticky bottom-4 z-10 mx-auto mt-6 flex w-fit items-center gap-3 rounded-full bg-popover px-4 py-2 text-caption shadow-lg">
      <span className="tabular-nums text-foreground">{selection.length} selected · {diskBytes(selectedBytes)}</span>
      <TextButton onClick={() => setSelected(new Map())}>Clear</TextButton>
      {copilot && <TextButton onClick={() => ask(`Tell me what the ${selection.length} item${selection.length === 1 ? "" : "s"} I selected are, and whether I can delete them.`)}>Ask the agent</TextButton>}
      <TextButton tone="destructive" disabled={busy} onClick={() => setConfirming({ entries: selection, permanent: false })}>Move to Trash</TextButton>
    </div>}
  </div>;
}
