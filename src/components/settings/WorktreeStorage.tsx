// Bridge's own project copies (worktrees): what they cost, and the only place
// a person can act on one. Drawn as one section of the Storage page.
//
// Reclaim follows the safety assessment. Explicit Delete lets a person
// discard uncertain contents after confirmation; live use and pending worker
// output remain protected by the backend.
//
// Sizes are the last measurement, not a live figure. Measuring means walking a
// directory that can hold a few hundred thousand files, which is the sweep's
// job on its own schedule, so the page says when it last looked rather than
// pretending to be current.

import { useCallback, useEffect, useState } from "react";
import { cn } from "@/lib/utils";
import { bridgeApi as api } from "../../api";
import type { WorktreeInventoryEntry, WorktreeUsage } from "../../types";
import { GhostButton, Select, StatusPill, TextButton, type PillTone } from "./kit";

// Repositories have no identity colour of their own, so the breakdown wears
// the same achromatic lightness ramp as the disk summary above it.
const RAMP = ["bg-foreground/80", "bg-foreground/55", "bg-foreground/35", "bg-foreground/20"];
const swatch = (index: number) => RAMP[index % RAMP.length];

/** A segmented meter bar across repositories, each wearing a palette hue,
 *  with a hover tooltip — the same reveal-once-on-mount motion as the meter's
 *  own bars, applied per segment instead of per window. */
function RepoBreakdown({ repositories, totalBytes, onSelect }: {
  repositories: WorktreeUsage["repositories"];
  totalBytes: number;
  onSelect: (repoRoot: string) => void;
}) {
  const [hover, setHover] = useState<number | null>(null);
  const sorted = [...repositories].sort((a, b) => b.sizeBytes - a.sizeBytes);
  if (sorted.length === 0 || totalBytes <= 0) return null;
  // Raising tiny slivers to a visible minimum can push the total past 100%;
  // rescale everything back down so the bar's widths still sum to 100% and
  // large segments keep their true proportion instead of getting squeezed by
  // flex-shrink.
  const raw = sorted.map(repo => Math.max((repo.sizeBytes / totalBytes) * 100, repo.sizeBytes > 0 ? 0.5 : 0));
  const rawTotal = raw.reduce((sum, value) => sum + value, 0);
  const scale = rawTotal > 100 ? 100 / rawTotal : 1;
  return <div className="py-3">
    <div className="relative flex h-1.5 w-full overflow-hidden rounded-full bg-muted">
      {sorted.map((repo, index) => {
        const width = raw[index] * scale;
        return <button
          key={repo.repoRoot}
          type="button"
          aria-label={`${repoName(repo.repoRoot)}: ${bytes(repo.sizeBytes)}`}
          onMouseEnter={() => setHover(index)}
          onMouseLeave={() => setHover(current => (current === index ? null : current))}
          onFocus={() => setHover(index)}
          onBlur={() => setHover(current => (current === index ? null : current))}
          onClick={() => onSelect(repo.repoRoot)}
          className={cn("h-full origin-left cursor-pointer outline-none motion-safe:animate-[meter-fill_600ms_ease-out] first:rounded-l-full last:rounded-r-full", swatch(index), hover === index && "brightness-110")}
          style={{ width: `${width}%` }}
        />;
      })}
    </div>
    {hover !== null && sorted[hover] && <div role="tooltip" className="u-glass-popover mt-2 inline-flex items-center gap-2 rounded-lg px-2.5 py-1.5 text-caption">
      <span className={cn("size-2 shrink-0 rounded-full", swatch(hover))} />
      <span className="font-medium text-foreground">{repoName(sorted[hover].repoRoot)}</span>
      <span className="tabular-nums text-muted-foreground">{sorted[hover].count} · {bytes(sorted[hover].sizeBytes)}{sorted[hover].overBudget ? " · Over limit" : ""}</span>
    </div>}
    <ul className="mt-3 flex flex-wrap gap-x-4 gap-y-1.5">
      {sorted.map((repo, index) => <li key={repo.repoRoot}>
        <button
          type="button"
          onClick={() => onSelect(repo.repoRoot)}
          onMouseEnter={() => setHover(index)}
          onMouseLeave={() => setHover(current => (current === index ? null : current))}
          className={cn("flex items-center gap-1.5 rounded-md px-1 py-0.5 text-xs outline-none transition-colors hover:bg-accent focus-visible:ring-2 focus-visible:ring-ring", hover === index && "bg-accent")}
        >
          <span className={cn("size-2 shrink-0 rounded-full", swatch(index))} />
          <span className="truncate text-foreground" title={repo.repoRoot}>{repoName(repo.repoRoot)}</span>
          <span className="shrink-0 tabular-nums text-muted-foreground">{repo.count} · {bytes(repo.sizeBytes)}{repo.overBudget ? " · Over limit" : ""}</span>
        </button>
      </li>)}
    </ul>
  </div>;
}

function bytes(value: number | null | undefined): string {
  if (value === null || value === undefined) return "—";
  const units = ["B", "KiB", "MiB", "GiB", "TiB"];
  let size = value;
  let unit = 0;
  while (size >= 1024 && unit < units.length - 1) {
    size /= 1024;
    unit += 1;
  }
  return unit === 0 ? `${value} B` : `${size.toFixed(1)} ${units[unit]}`;
}

function age(seconds: number): string {
  if (seconds < 60) return "just now";
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m idle`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h idle`;
  return `${Math.floor(hours / 24)}d idle`;
}

/** Only two dispositions can be acted on; the rest exist to be explained. */
const RECLAIMABLE = new Set(["reclaimable", "pushed_unmerged"]);

/** Dispositions a person may override for a checkout they can see and chose
 *  themselves — never `retained`, which already means something else has a
 *  stake in it. Mirrors `is_removable`'s `force` branch in worktree_registry.rs. */
const FORCIBLE = new Set(["at_risk", "unverifiable"]);

const DISPOSITION_TONE: Record<string, PillTone> = {
  reclaimable: "success",
  pushed_unmerged: "info",
  at_risk: "warning",
  retained: "neutral",
  unverifiable: "destructive",
};

const DISPOSITION_LABEL: Record<string, string> = {
  reclaimable: "Reclaimable",
  pushed_unmerged: "On a remote",
  at_risk: "Holds work",
  retained: "In use",
  unverifiable: "Unreadable",
};

function repoName(path: string): string {
  const parts = path.split("/").filter(Boolean);
  return parts[parts.length - 1] ?? path;
}

export function WorktreeStorage({ onError }: { onError?: (message: string) => void }) {
  const [usage, setUsage] = useState<WorktreeUsage | null>(null);
  const [entries, setEntries] = useState<WorktreeInventoryEntry[]>([]);
  const [busy, setBusy] = useState<string | null>(null);
  const [note, setNote] = useState<string | null>(null);
  const [error, setError] = useState<string>();
  const [loading, setLoading] = useState(true);
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState("all");
  const [repository, setRepository] = useState("");
  const [sort, setSort] = useState("size");
  const [confirming, setConfirming] = useState<{ entry: WorktreeInventoryEntry; force: boolean } | "sweep" | null>(null);

  const load = useCallback(async () => {
    setLoading(true); setError(undefined);
    try {
      const [nextUsage, nextEntries] = await Promise.all([api.worktreeUsage(), api.listWorktrees()]);
      setUsage(nextUsage);
      setEntries(nextEntries);
    } catch (error) {
      setError(error instanceof Error ? error.message : String(error));
      onError?.(error instanceof Error ? error.message : String(error));
    } finally { setLoading(false); }
  }, [onError]);

  useEffect(() => { void load(); }, [load]);

  const reclaim = async (entry: WorktreeInventoryEntry, force: boolean) => {
    setConfirming(null); setError(undefined);
    setBusy(entry.id);
    setNote(null);
    try {
      const result = await api.reclaimWorktree(entry.id, force);
      setNote(result.reclaimed
        ? `${force ? "Deleted" : "Reclaimed"} ${bytes(result.bytesFreed)} from ${repoName(entry.path)}.`
        : result.detail ?? "Nothing was reclaimed.");
      await load();
    } catch (error) {
      setError(error instanceof Error ? error.message : String(error));
      onError?.(error instanceof Error ? error.message : String(error));
    } finally {
      setBusy(null);
    }
  };

  const sweep = async () => {
    setConfirming(null); setError(undefined);
    setBusy("sweep");
    setNote(null);
    try {
      const result = await api.sweepWorktrees();
      setNote((result.removed === 0
        ? "Nothing could be reclaimed safely under the current retention policy."
        : `Reclaimed ${result.removed} checkout${result.removed === 1 ? "" : "s"}, freeing ${bytes(result.removedBytes)}.`)
        + (result.measurementsTruncated ? ` ${result.measurementsTruncated} measurements were incomplete.` : "")
        + (result.overBudgetBytes ? ` ${bytes(result.overBudgetBytes)} remains over budget and is protected.` : ""));
      await load();
    } catch (error) {
      setError(error instanceof Error ? error.message : String(error));
      onError?.(error instanceof Error ? error.message : String(error));
    } finally {
      setBusy(null);
    }
  };

  // The caps are per repository; `totalBytes` is the sum across all of them.
  // Comparing the two labelled two 6 GiB repositories as over a 10 GiB limit
  // when neither was, and said nothing about a repository over the *count* cap.
  // The backend already decides this per repository.
  const externalCount = entries.filter(entry => entry.state === "external").length;
  const unmeasuredCount = entries.filter(entry => entry.sizeBytes === null).length;
  const overBudget = usage?.repositories.some(repo => repo.overBudget) ?? false;
  const visible = entries.filter(entry => {
    if (repository && entry.repoRoot !== repository) return false;
    if (filter === "external" && entry.state !== "external") return false;
    if (filter === "reclaimable" && (entry.state === "external" || !RECLAIMABLE.has(entry.disposition ?? ""))) return false;
    if (filter === "protected" && (entry.state === "external" || RECLAIMABLE.has(entry.disposition ?? ""))) return false;
    return [entry.path, entry.branch, entry.repoRoot, entry.retainedReason, entry.ownerSessionId].some(value => value?.toLowerCase().includes(query.trim().toLowerCase()));
  }).sort((a, b) => sort === "idle" ? b.idleSeconds - a.idleSeconds : sort === "name" ? (a.branch ?? a.path).localeCompare(b.branch ?? b.path) : (b.sizeBytes ?? -1) - (a.sizeBytes ?? -1));

  return <section aria-label="Project copies" className="space-y-3">
    <header className="flex flex-wrap items-baseline gap-x-3 gap-y-1">
      <h3 className="text-ui font-medium text-foreground">Project copies</h3>
      {usage && <span className="text-caption tabular-nums text-muted-foreground">
        {bytes(usage.totalBytes)} across {usage.totalCount} checkout{usage.totalCount === 1 ? "" : "s"}
        {externalCount > 0 && `, including ${externalCount} external`}
      </span>}
      {usage && usage.reclaimableBytes > 0 && <span className="text-caption text-success">{bytes(usage.reclaimableBytes)} reclaimable</span>}
      {usage && usage.retainedCount > 0 && <span className="text-caption text-muted-foreground">{usage.retainedCount} kept for a reason</span>}
      {overBudget && <StatusPill tone="warning">Over the limit</StatusPill>}
      <span className="ml-auto flex items-center gap-1">
        <TextButton disabled={loading || busy !== null} onClick={() => void load()} ariaLabel="Refresh inventory">Refresh</TextButton>
        <TextButton onClick={() => setConfirming("sweep")} disabled={busy !== null || loading} ariaLabel="Review safe cleanup">
          {busy === "sweep" ? "Checking…" : "Check for cleanup"}
        </TextButton>
      </span>
    </header>
    <p className="text-caption leading-relaxed text-muted-foreground">
      The copies of your projects Bridge's agents work in.{usage && ` Automatic cleanup targets at most ${usage.maxPerRepo} Bridge-owned worktrees and ${bytes(usage.maxTotalBytes)} per repository, and only removes copies it can prove are expendable.`} External checkouts are never removed. Confirmed Delete can discard dirty or unreadable checkouts. Live sessions and unadopted worker output remain protected.
      {unmeasuredCount > 0 && ` ${unmeasuredCount} checkout${unmeasuredCount === 1 ? "" : "s"} not yet measured; total includes measured sizes only.`}
    </p>
    {error && <p role="alert" className="text-caption text-destructive">{error}</p>}
    {loading && <p role="status" className="text-caption text-muted-foreground">Loading storage inventory...</p>}
    {usage && <RepoBreakdown repositories={usage.repositories} totalBytes={usage.totalBytes} onSelect={repo => setRepository(current => (current === repo ? "" : repo))} />}
    {note && <p role="status" className="text-caption text-muted-foreground">{note}</p>}
    {confirming && <div role="group" aria-label="Confirm cleanup" className="rounded-lg bg-muted/50 px-4 py-3">
      <p className="text-ui font-medium text-foreground">{confirming === "sweep" ? "Run safe cleanup?" : confirming.force ? `Delete ${confirming.entry.branch ?? repoName(confirming.entry.path)}?` : `Reclaim ${confirming.entry.branch ?? repoName(confirming.entry.path)}?`}</p>
      <p className="mt-1 break-words text-caption leading-relaxed text-muted-foreground">
        {confirming === "sweep"
          ? "Reassess checkouts and remove only those allowed by retention limits. Protected work stays."
          : confirming.force
            ? `${confirming.entry.path}. Bridge could not prove this checkout is safe to remove (${confirming.entry.retainedReason ?? "uncommitted or unproven work"}). Deleting it anyway discards anything not saved elsewhere.`
            : `${confirming.entry.path}. This removes the checkout and its ignored build files, not chat history. Safety is checked again before removal.`}
      </p>
      <div className="mt-3 flex gap-2">
        <GhostButton onClick={() => setConfirming(null)}>Cancel</GhostButton>
        {confirming !== "sweep" && confirming.force
          ? <TextButton tone="destructive" disabled={busy !== null} onClick={() => void reclaim(confirming.entry, true)}>Delete anyway</TextButton>
          : <GhostButton disabled={busy !== null} onClick={() => confirming === "sweep" ? void sweep() : void reclaim(confirming.entry, false)}>Confirm cleanup</GhostButton>}
      </div>
    </div>}
    {entries.length > 0 && <div className="flex flex-wrap items-center gap-2">
      <input type="search" aria-label="Search worktrees" placeholder="Search branch, path, or reason" value={query} onChange={event => setQuery(event.target.value)} className="h-8 min-w-48 flex-1 rounded-md bg-muted/50 px-3 text-caption text-foreground outline-none placeholder:text-muted-foreground focus-visible:ring-2 focus-visible:ring-ring" />
      <Select label="Repository" value={repository} onChange={setRepository} width="w-44" options={[{ value: "", label: "All repositories" }, ...[...new Set(entries.map(entry => entry.repoRoot))].map(repo => ({ value: repo, label: repoName(repo), description: repo }))]} />
      <Select label="Worktree status" value={filter} onChange={setFilter} width="w-36" options={[{ value: "all", label: "All checkouts" }, { value: "reclaimable", label: "Reclaimable" }, { value: "protected", label: "Protected" }, { value: "external", label: "External" }]} />
      <Select label="Sort worktrees" value={sort} onChange={setSort} width="w-36" options={[{ value: "size", label: "Largest first" }, { value: "idle", label: "Longest idle" }, { value: "name", label: "Branch name" }]} />
    </div>}
    {entries.length === 0 && !loading && <p className="text-caption text-muted-foreground">Bridge has not created any worktrees yet.</p>}
    <ul className="divide-y divide-border/50">
      {visible.map(entry => {
        const external = entry.state === "external";
        const disposition = entry.disposition ?? (external ? "retained" : "");
        const actionable = !external && RECLAIMABLE.has(disposition);
        // Bridge cannot prove these safe, but a person looking at the row
        // can decide for themselves — never offered for a live session, an
        // unadopted worker output, or a checkout Bridge did not create,
        // since those stay "retained" and are never forcible.
        const forcible = !external && !actionable && FORCIBLE.has(disposition);
        return <li key={entry.id} className="group flex items-center gap-4 py-2.5">
          <div className="min-w-0 flex-1">
            <p className="flex min-w-0 items-baseline gap-2">
              <span className="truncate text-ui text-foreground" title={entry.path}>{entry.branch ?? repoName(entry.path)}</span>
              <span className="shrink-0 text-caption text-muted-foreground">{repoName(entry.repoRoot)} · {entry.kind} · {age(entry.idleSeconds)}</span>
            </p>
            <p className="truncate text-caption text-muted-foreground" title={entry.path}>
              {external ? "Not created by Bridge — shown for context, never reclaimed." : entry.retainedReason ?? entry.path}
            </p>
          </div>
          {disposition && <StatusPill tone={DISPOSITION_TONE[disposition] ?? "neutral"}>{DISPOSITION_LABEL[disposition] ?? disposition}</StatusPill>}
          <span className="w-20 shrink-0 text-right text-ui tabular-nums text-foreground">{bytes(entry.sizeBytes)}</span>
          <span className="flex w-24 shrink-0 justify-end">
            {actionable && <TextButton onClick={() => setConfirming({ entry, force: false })} disabled={busy !== null} ariaLabel={`Reclaim ${entry.branch ?? entry.path}`}>
              {busy === entry.id ? "Reclaiming…" : "Reclaim"}
            </TextButton>}
            {forcible && <TextButton tone="destructive" onClick={() => setConfirming({ entry, force: true })} disabled={busy !== null} ariaLabel={`Delete ${entry.branch ?? entry.path}`}>
              {busy === entry.id ? "Deleting…" : "Delete"}
            </TextButton>}
          </span>
        </li>;
      })}
    </ul>
    {!loading && entries.length > 0 && visible.length === 0 && <p className="py-4 text-center text-caption text-muted-foreground">No checkouts match these filters.</p>}
  </section>;
}
