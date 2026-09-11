// Storage: what Bridge's worktrees cost, and the only place a person can act on
// one.
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
import { GhostButton, Select, SettingsGroup, SettingsPage, StatusPill, TextButton, type PillTone } from "./kit";
import { harnessChartDot, harnessChartText } from "../harnessMarks";
import { Search, RefreshCw, HardDrive, ShieldCheck } from "lucide-react";

// Repositories have no harness of their own, so the breakdown chart borrows
// the same validated four-hue chart palette the usage board series wear,
// cycling by rank rather than by identity — the one categorical palette this
// codebase has signed off on, reused rather than a second one invented here.
const PALETTE = ["codex", "cursor", "claude", "opencode"] as const;
const swatch = (index: number) => PALETTE[index % PALETTE.length];

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
  return <div className="border-t border-border/60 px-4 py-3">
    <div className="relative flex h-2.5 w-full overflow-hidden rounded-full bg-muted">
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
          className={cn("h-full origin-left cursor-pointer outline-none motion-safe:animate-[meter-fill_600ms_ease-out] first:rounded-l-full last:rounded-r-full", harnessChartDot(swatch(index)), hover === index && "brightness-110")}
          style={{ width: `${width}%` }}
        />;
      })}
    </div>
    {hover !== null && sorted[hover] && <div role="tooltip" className="u-glass-popover mt-2 inline-flex items-center gap-2 rounded-lg px-2.5 py-1.5 text-caption">
      <span className={cn("size-2 shrink-0 rounded-[3px]", harnessChartDot(swatch(hover)))} />
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
          <span className={cn("size-2 shrink-0 rounded-[3px]", harnessChartDot(swatch(index)))} />
          <span className={cn("truncate font-mono", harnessChartText(swatch(index)))} title={repo.repoRoot}>{repoName(repo.repoRoot)}</span>
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

export function StoragePage({ onError }: { onError?: (message: string) => void }) {
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

  return <SettingsPage
    title="Storage"
    description="See where your disk goes, what can be reclaimed, and what needs your attention. Chat history and checkout storage are separate."
    action={<GhostButton onClick={() => setConfirming("sweep")} disabled={busy !== null || loading} ariaLabel="Review safe cleanup">
      {busy === "sweep" ? "Sweeping…" : "Sweep"}
    </GhostButton>}
  >
    {error && <p role="alert" className="rounded-lg border border-destructive/30 p-3 text-sm text-destructive">{error}</p>}
    {loading && <p role="status" className="text-xs text-muted-foreground">Loading storage inventory...</p>}
    {usage && <SettingsGroup
      label="Worktrees"
      note={`Automatic cleanup targets at most ${usage.maxPerRepo} Bridge-owned worktrees and ${bytes(usage.maxTotalBytes)} per repository. Over either limit it reclaims the least recently used checkouts it can prove are expendable, and reports the rest rather than forcing them.`}
    >
      <div className="flex flex-wrap items-baseline gap-x-6 gap-y-2 px-4 py-3">
        <HardDrive size={20} className="self-center text-muted-foreground" /><span className="text-3xl font-semibold tabular-nums text-foreground">{bytes(usage.totalBytes)}</span>
        <span className="text-xs text-muted-foreground">
          across {usage.totalCount} checkout{usage.totalCount === 1 ? "" : "s"}
          {externalCount > 0 && `, including ${externalCount} external`}
        </span>
        {unmeasuredCount > 0 && <span className="text-xs text-muted-foreground">
          {unmeasuredCount} checkout{unmeasuredCount === 1 ? "" : "s"} not yet measured; total includes measured sizes only
        </span>}
        {usage.reclaimableBytes > 0 && <span className="text-xs text-success">
          {bytes(usage.reclaimableBytes)} reclaimable
        </span>}
        {usage.retainedCount > 0 && <span className="text-xs text-muted-foreground">
          {usage.retainedCount} kept for a reason
        </span>}
        {overBudget && <StatusPill tone="warning">Over the limit</StatusPill>}
      </div>
      <RepoBreakdown repositories={usage.repositories} totalBytes={usage.totalBytes} onSelect={repo => setRepository(current => (current === repo ? "" : repo))} />
    </SettingsGroup>}

    <p className="flex gap-2 text-xs leading-relaxed text-muted-foreground"><ShieldCheck size={15} className="shrink-0" />External checkouts are never removed or counted against retention limits. Sweep protects dirty files and local-only commits. Confirmed Delete can discard dirty or unreadable checkouts. Live sessions and unadopted worker output remain protected.</p>
    {note && <p role="status" className="px-1 text-xs text-muted-foreground">{note}</p>}
    {confirming && <section aria-label="Confirm cleanup" className="rounded-xl border border-border bg-card p-4">
      <h3 className="text-sm font-semibold">{confirming === "sweep" ? "Run safe cleanup?" : confirming.force ? `Delete ${confirming.entry.branch ?? repoName(confirming.entry.path)}?` : `Reclaim ${confirming.entry.branch ?? repoName(confirming.entry.path)}?`}</h3>
      <p className="mt-2 break-words text-xs leading-relaxed text-muted-foreground">
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
    </section>}
    <div className="flex flex-wrap gap-2">
      <label className="flex min-w-48 flex-1 items-center gap-2 rounded-lg border border-border bg-card px-3"><Search size={14} className="text-muted-foreground" /><input type="search" aria-label="Search worktrees" placeholder="Branch, path, or reason" value={query} onChange={event => setQuery(event.target.value)} className="h-9 min-w-0 flex-1 bg-transparent text-xs outline-none" /></label>
      <Select label="Repository" value={repository} onChange={setRepository} width="w-44" options={[{ value: "", label: "All repositories" }, ...[...new Set(entries.map(entry => entry.repoRoot))].map(repo => ({ value: repo, label: repoName(repo), description: repo }))]} />
      <Select label="Worktree status" value={filter} onChange={setFilter} width="w-36" options={[{ value: "all", label: "All checkouts" }, { value: "reclaimable", label: "Reclaimable" }, { value: "protected", label: "Protected" }, { value: "external", label: "External" }]} />
      <Select label="Sort worktrees" value={sort} onChange={setSort} width="w-36" options={[{ value: "size", label: "Largest first" }, { value: "idle", label: "Longest idle" }, { value: "name", label: "Branch name" }]} />
      <GhostButton disabled={loading || busy !== null} onClick={() => void load()} ariaLabel="Refresh inventory"><RefreshCw size={14} /></GhostButton>
    </div>

    <SettingsGroup label={`Checkouts (${visible.length})`} note={entries.length === 0 && !loading ? "Bridge has not created any worktrees yet." : "Sizes are logical bytes from the last measurement, not unique physical allocation. Refresh reloads inventory; Sweep remeasures it."}>
      <ul className="divide-y divide-border/60">
        {visible.map(entry => {
          const external = entry.state === "external";
          const disposition = entry.disposition ?? (external ? "retained" : "");
          const actionable = !external && RECLAIMABLE.has(disposition);
          // Bridge cannot prove these safe, but a person looking at the row
          // can decide for themselves — never offered for a live session, an
          // unadopted worker output, or a checkout Bridge did not create,
          // since those stay "retained" and are never forcible.
          const forcible = !external && !actionable && FORCIBLE.has(disposition);
          return <li key={entry.id} className="flex flex-col gap-1 px-4 py-3">
            <div className="flex flex-wrap items-center justify-between gap-3">
              <span className="flex min-w-0 items-center gap-2">
                <span className="truncate text-sm font-medium text-foreground" title={entry.path}>
                  {entry.branch ?? repoName(entry.path)}
                </span>
                <span className="shrink-0 text-[11px] uppercase tracking-wide text-muted-foreground">{entry.kind}</span>
              </span>
              <span className="flex shrink-0 items-center gap-3 text-xs tabular-nums text-muted-foreground">
                <span>{age(entry.idleSeconds)}</span>
                <span>{bytes(entry.sizeBytes)}</span>
                {disposition && <StatusPill tone={DISPOSITION_TONE[disposition] ?? "neutral"}>
                  {DISPOSITION_LABEL[disposition] ?? disposition}
                </StatusPill>}
              </span>
            </div>
            <p className="break-all font-mono text-[11px] text-muted-foreground">{entry.path}</p>
            <p className="text-[11px] text-muted-foreground">{entry.sizeMeasuredAt ? `Measured ${new Date(entry.sizeMeasuredAt).toLocaleString()}` : "Not yet measured"}{entry.assessedAt ? ` · Assessed ${new Date(entry.assessedAt).toLocaleString()}` : " · Not yet assessed"}</p>
            <div className="flex items-start justify-between gap-3">
              <span className="min-w-0 text-xs text-muted-foreground">
                {external
                  ? "Not created by Bridge — shown for context, never reclaimed."
                  : entry.retainedReason ?? entry.path}
              </span>
              {actionable && <GhostButton
                onClick={() => setConfirming({ entry, force: false })}
                disabled={busy !== null}
                ariaLabel={`Reclaim ${entry.branch ?? entry.path}`}
              >
                {busy === entry.id ? "Reclaiming…" : "Reclaim"}
              </GhostButton>}
              {forcible && <TextButton
                tone="destructive"
                onClick={() => setConfirming({ entry, force: true })}
                disabled={busy !== null}
                ariaLabel={`Delete ${entry.branch ?? entry.path}`}
              >
                {busy === entry.id ? "Deleting…" : "Delete"}
              </TextButton>}
            </div>
          </li>;
        })}
      </ul>
      {!loading && entries.length > 0 && visible.length === 0 && <p className="p-6 text-center text-sm text-muted-foreground">No checkouts match these filters.</p>}
    </SettingsGroup>
  </SettingsPage>;
}
