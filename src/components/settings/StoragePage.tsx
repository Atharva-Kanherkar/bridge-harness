// Storage: everything on this Mac, largest first, and the place a person can
// delete what they no longer want.
//
// The backend never waits on a folder walk. A listing comes back at once with
// the sizes it already knows, so the page asks again while anything is still
// being measured and the rows fill in. Deleting moves to the Trash unless the
// person asks for more; the backend refuses the few places that would break
// macOS or Bridge, and says why per row.
//
// The copilot rail is a standing Bridge chat docked beside the listing. The
// first question of each visit carries what this page measured, so the agent
// can look deeper than a size listing can.

import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { cn } from "@/lib/utils";
import { bridgeApi as api } from "../../api";
import type { DiskEntry, DiskListing, DiskOverview } from "../../types";
import { cleanupTotal, diskBytes, displayPath, storageBriefing } from "../../diskSpace";
import { GhostButton, PrimaryButton, TextButton } from "./kit";
import type { StorageCopilotHost } from "./StorageCopilot";
import { WorktreeStorage } from "./WorktreeStorage";

const POLL_MS = 1500;

/** Achromatic by design: a frame hosting other brands stays neutral, so the
 *  breakdown is a lightness ramp rather than a palette. */
const RAMP = ["bg-foreground/80", "bg-foreground/60", "bg-foreground/45", "bg-foreground/32", "bg-foreground/22"];
const OTHER = "bg-foreground/12";

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

/** Free space, and what fills the rest: the home folder's biggest children in
 *  a lightness ramp, then everything else the volume reports as used. */
function DiskSummary({ overview, home, onOpen, onEmptyTrash, busy }: {
  overview: DiskOverview | null;
  home: DiskListing | null;
  onOpen: (path: string) => void;
  onEmptyTrash: () => void;
  busy: boolean;
}) {
  const volume = overview?.volume;
  if (!volume) return null;
  const top = (home?.entries ?? []).filter(entry => (entry.sizeBytes ?? 0) > 0).slice(0, RAMP.length);
  const named = top.reduce((total, entry) => total + (entry.sizeBytes ?? 0), 0);
  const other = Math.max(volume.usedBytes - named, 0);
  const share = (value: number) => `${Math.max((value / volume.totalBytes) * 100, value > 0 ? 0.4 : 0)}%`;
  const usedShare = volume.usedBytes / volume.totalBytes;
  return <section aria-label="Disk">
    <div className="flex flex-wrap items-baseline gap-x-3 gap-y-1">
      <span className="font-display text-4xl font-semibold tabular-nums tracking-tight text-foreground">{diskBytes(volume.freeBytes)}</span>
      <span className="text-ui text-muted-foreground">free of {diskBytes(volume.totalBytes)}</span>
      {usedShare > 0.9 && <span className="text-caption text-warning">Nearly full</span>}
      <span className="ml-auto"><TextButton onClick={onEmptyTrash} disabled={busy}>Empty Trash</TextButton></span>
    </div>
    <div className="mt-4 flex h-2 w-full overflow-hidden rounded-full bg-muted" role="img" aria-label={`${diskBytes(volume.usedBytes)} used of ${diskBytes(volume.totalBytes)}`}>
      {top.map((entry, index) => <span key={entry.path} className={cn("h-full origin-left motion-safe:animate-[meter-fill_600ms_ease-out]", RAMP[index])} style={{ width: share(entry.sizeBytes ?? 0) }} />)}
      <span className={cn("h-full", OTHER)} style={{ width: share(other) }} />
    </div>
    <ul className="mt-3 flex flex-wrap gap-x-5 gap-y-1.5">
      {top.map((entry, index) => <li key={entry.path}>
        <button type="button" onClick={() => onOpen(entry.path)} className="flex items-center gap-1.5 rounded text-caption text-muted-foreground outline-none transition-colors hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring">
          <span className={cn("size-2 rounded-full", RAMP[index])} />
          <span className="text-foreground">{entry.name}</span>
          <span className="tabular-nums">{diskBytes(entry.sizeBytes)}</span>
        </button>
      </li>)}
      <li className="flex items-center gap-1.5 text-caption text-muted-foreground">
        <span className={cn("size-2 rounded-full", OTHER)} />
        <span className="text-foreground">Apps, system, and other</span>
        <span className="tabular-nums">{diskBytes(other)}</span>
      </li>
    </ul>
  </section>;
}

function Suggestions({ overview, onOpen, onDelete, busy }: {
  overview: DiskOverview;
  onOpen: (path: string) => void;
  onDelete: (entry: DiskEntry) => void;
  busy: boolean;
}) {
  const rows = [...overview.suggestions].sort((a, b) => (b.sizeBytes ?? -1) - (a.sizeBytes ?? -1));
  if (rows.length === 0) return null;
  const safe = cleanupTotal(overview.suggestions, "safe");
  return <section aria-label="Cleanup suggestions">
    <SectionHeading title="Worth a look" detail={safe > 0 ? `${diskBytes(safe)} rebuilds itself on demand` : undefined} />
    <ul className="divide-y divide-border/50">
      {rows.map(item => <li key={item.id} className="group flex items-center gap-4 py-2.5">
        <button type="button" onClick={() => onOpen(item.path)} className="min-w-0 flex-1 rounded text-left outline-none focus-visible:ring-2 focus-visible:ring-ring">
          <p className="flex items-baseline gap-2">
            <span className="text-ui text-foreground">{item.label}</span>
            <span className={`text-caption ${item.safety === "safe" ? "text-success" : "text-muted-foreground"}`}>{item.safety === "safe" ? "Rebuilt automatically" : "Review first"}</span>
          </p>
          <p className="truncate text-caption text-muted-foreground">{item.description}</p>
        </button>
        <span className="w-20 shrink-0 text-right text-ui tabular-nums text-foreground"><Size entry={item} /></span>
        <span className="flex w-28 shrink-0 justify-end opacity-0 transition-opacity group-focus-within:opacity-100 group-hover:opacity-100">
          <TextButton
            tone="destructive"
            disabled={busy || !item.sizeBytes}
            ariaLabel={`Move ${item.label} to the Trash`}
            onClick={() => onDelete({ name: item.label, path: item.path, kind: "directory", sizeBytes: item.sizeBytes, itemCount: null, measuring: item.measuring, partial: false, modifiedAt: null, protectedReason: null })}
          >Move to Trash</TextButton>
        </span>
      </li>)}
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

function Explorer({ listing, overview, root, selected, busy, onRoot, onOpen, onToggle, onDelete, onRefresh }: {
  listing: DiskListing | null;
  overview: DiskOverview | null;
  root: string | null;
  selected: ReadonlyMap<string, DiskEntry>;
  busy: boolean;
  onRoot: (path: string | null) => void;
  onOpen: (path: string) => void;
  onToggle: (entry: DiskEntry) => void;
  onDelete: (entry: DiskEntry) => void;
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
        {ROOTS.map(item => <TextButton key={item.label} onClick={() => onRoot(item.path)}>
          <span className={cn((root ?? null) === item.path && "text-foreground")}>{item.label}</span>
        </TextButton>)}
        <TextButton onClick={onRefresh} disabled={busy} ariaLabel="Measure this folder again">Remeasure</TextButton>
      </>}
    />
    {listing && <Breadcrumb path={listing.path} home={overview?.home} onOpen={onOpen} />}
    {listing?.unreadable && <p className="mt-3 text-caption text-muted-foreground">{listing.unreadable}</p>}
    <ul className="mt-2 divide-y divide-border/50">
      {listing?.entries.map(entry => {
        const folder = entry.kind === "directory" || entry.kind === "package";
        const checked = selected.has(entry.path);
        return <li key={entry.path} className={cn("group flex items-center gap-3 py-2", checked && "bg-accent/40")}>
          <button
            type="button"
            role="checkbox"
            aria-label={`Select ${entry.name}`}
            aria-checked={checked}
            disabled={Boolean(entry.protectedReason)}
            onClick={() => onToggle(entry)}
            className={cn("grid size-3.5 shrink-0 place-items-center rounded-[4px] outline-none ring-1 ring-inset transition-colors focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-30", checked ? "bg-foreground ring-foreground" : "ring-border hover:ring-muted-foreground")}
          >
            {checked && <span className="size-1.5 rounded-[2px] bg-background" />}
          </button>
          <div className="min-w-0 flex-1">
            {folder
              ? <button type="button" onClick={() => onOpen(entry.path)} className="block max-w-full truncate rounded text-left text-ui text-foreground outline-none hover:underline focus-visible:ring-2 focus-visible:ring-ring">{entry.name}</button>
              : <span className="block truncate text-ui text-foreground">{entry.name}</span>}
            <span className="mt-1 block h-0.5 rounded-full bg-muted">
                      <span className="block h-full rounded-full bg-foreground/30" style={{ width: `${((entry.sizeBytes ?? 0) / largest) * 100}%` }} />
            </span>
          </div>
          <span className="hidden w-24 shrink-0 text-right text-caption tabular-nums text-muted-foreground sm:block">
            {folder && entry.itemCount ? `${entry.itemCount.toLocaleString()} item${entry.itemCount === 1 ? "" : "s"}` : ""}
          </span>
          <span className="w-20 shrink-0 text-right text-ui tabular-nums text-foreground"><Size entry={entry} /></span>
          <span className="flex w-28 shrink-0 justify-end opacity-0 transition-opacity group-focus-within:opacity-100 group-hover:opacity-100">
            {entry.protectedReason
              ? <span className="truncate text-caption text-muted-foreground" title={entry.protectedReason}>Protected</span>
              : <TextButton tone="destructive" disabled={busy} ariaLabel={`Move ${entry.name} to the Trash`} onClick={() => onDelete(entry)}>Move to Trash</TextButton>}
          </span>
        </li>;
      })}
      {listing && listing.omittedCount > 0 && <li className="flex items-center gap-3 py-2 pl-6.5 text-caption text-muted-foreground">
        <span className="flex-1">{listing.omittedCount.toLocaleString()} smaller items</span>
        <span className="w-20 text-right tabular-nums">{diskBytes(listing.omittedBytes)}</span>
        <span className="w-28" />
      </li>}
      {unseen > 0 && <li className="flex items-center gap-3 py-2 pl-6.5 text-caption text-muted-foreground">
        <span className="flex-1">System volume, snapshots, and space macOS can purge</span>
        <span className="w-20 text-right tabular-nums">{diskBytes(unseen)}</span>
        <span className="w-28" />
      </li>}
    </ul>
    {listing && !listing.unreadable && listing.entries.length === 0 && <p className="py-6 text-center text-caption text-muted-foreground">This folder is empty.</p>}
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

  // Only the first question of a visit carries the measurement; after that the
  // chat already has it, and repeating it would bury the conversation.
  const briefed = useRef(false);
  const brief = (question: string) => {
    if (briefed.current) return question;
    briefed.current = true;
    return storageBriefing({ question, overview, listing, selected: selection });
  };
  const ask = (question: string) => copilot?.ask(brief(question));

  return <div data-settings-column className="@container/settings mx-auto w-full max-w-page-wide px-5 pb-16 pt-6 sm:px-8">
    <header className="mb-8">
      <h2 className="font-display text-title font-semibold leading-tight tracking-tight text-foreground">{title}</h2>
      <p className="mt-1 text-ui leading-relaxed text-muted-foreground">What is using space on this Mac, and what you can let go of. Deleting moves to the Trash first.</p>
    </header>
    <div className={cn("grid gap-12", copilot && "lg:grid-cols-[minmax(0,1fr)_22rem]")}>
      <div className="min-w-0 space-y-12">
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

        {overview && <Suggestions overview={overview} onOpen={open} onDelete={entry => setConfirming({ entries: [entry], permanent: false })} busy={busy} />}

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
          onRefresh={() => void load(path, true)}
        />

        <WorktreeStorage onError={onError} />
        {extra}
      </div>
      {copilot?.render(brief, selection.length)}
    </div>

    {selection.length > 0 && <div role="region" aria-label="Selection" className="sticky bottom-4 z-10 mx-auto mt-6 flex w-fit items-center gap-3 rounded-full bg-popover px-4 py-2 text-caption shadow-lg">
      <span className="tabular-nums text-foreground">{selection.length} selected · {diskBytes(selectedBytes)}</span>
      <TextButton onClick={() => setSelected(new Map())}>Clear</TextButton>
      {copilot && <TextButton onClick={() => ask(`Tell me what the ${selection.length} item${selection.length === 1 ? "" : "s"} I selected are, and whether I can delete them.`)}>Ask Bridge</TextButton>}
      <TextButton tone="destructive" disabled={busy} onClick={() => setConfirming({ entries: selection, permanent: false })}>Move to Trash</TextButton>
    </div>}
  </div>;
}
