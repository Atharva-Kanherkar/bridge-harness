// Storage: what Bridge's worktrees cost, and the only place a person can act on
// one.
//
// The page is built around a single honest distinction — a checkout Bridge can
// prove is expendable, and one it cannot. So a row either offers Reclaim or
// says, in words, why nothing may touch it. There is no disabled button with a
// tooltip: the reason *is* the control's replacement, because "at risk" without
// "uncommitted changes" tells a person nothing they can act on.
//
// Sizes are the last measurement, not a live figure. Measuring means walking a
// directory that can hold a few hundred thousand files, which is the sweep's
// job on its own schedule, so the page says when it last looked rather than
// pretending to be current.

import { useCallback, useEffect, useState } from "react";
import { bridgeApi as api } from "../../api";
import type { WorktreeInventoryEntry, WorktreeUsage } from "../../types";
import { GhostButton, SettingsGroup, SettingsPage, StatusPill, type PillTone } from "./kit";

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

  const load = useCallback(async () => {
    try {
      const [nextUsage, nextEntries] = await Promise.all([api.worktreeUsage(), api.listWorktrees()]);
      setUsage(nextUsage);
      setEntries(nextEntries);
    } catch (error) {
      onError?.(error instanceof Error ? error.message : String(error));
    }
  }, [onError]);

  useEffect(() => { void load(); }, [load]);

  const reclaim = async (entry: WorktreeInventoryEntry) => {
    setBusy(entry.id);
    setNote(null);
    try {
      const result = await api.reclaimWorktree(entry.id);
      setNote(result.reclaimed
        ? `Reclaimed ${bytes(result.bytesFreed)} from ${repoName(entry.path)}.`
        : result.detail ?? "Nothing was reclaimed.");
      await load();
    } catch (error) {
      onError?.(error instanceof Error ? error.message : String(error));
    } finally {
      setBusy(null);
    }
  };

  const sweep = async () => {
    setBusy("sweep");
    setNote(null);
    try {
      const result = await api.sweepWorktrees();
      setNote(result.removed === 0
        ? "Nothing could be reclaimed safely. Every remaining checkout is in use or holds work."
        : `Reclaimed ${result.removed} checkout${result.removed === 1 ? "" : "s"}, freeing ${bytes(result.removedBytes)}.`);
      await load();
    } catch (error) {
      onError?.(error instanceof Error ? error.message : String(error));
    } finally {
      setBusy(null);
    }
  };

  const overBudget = usage ? usage.totalBytes >= usage.maxTotalBytes : false;

  return <SettingsPage
    title="Storage"
    description="Bridge cuts a Git worktree for each isolated chat, worker, and pull request it checks out. Agents build inside them, so they grow."
    action={<GhostButton onClick={sweep} disabled={busy !== null} ariaLabel="Reclaim everything that is safe to reclaim">
      {busy === "sweep" ? "Sweeping…" : "Sweep"}
    </GhostButton>}
  >
    {usage && <SettingsGroup
      label="Worktrees"
      note={`Bridge keeps at most ${usage.maxPerRepo} worktrees and ${bytes(usage.maxTotalBytes)} per repository. Over either limit it reclaims the least recently used checkouts it can prove are expendable, and reports the rest rather than forcing them.`}
    >
      <div className="flex flex-wrap items-baseline gap-x-6 gap-y-2 px-4 py-3">
        <span className="text-2xl font-semibold tabular-nums text-foreground">{bytes(usage.totalBytes)}</span>
        <span className="text-xs text-muted-foreground">
          across {usage.totalCount} checkout{usage.totalCount === 1 ? "" : "s"}
        </span>
        {usage.reclaimableBytes > 0 && <span className="text-xs text-success">
          {bytes(usage.reclaimableBytes)} reclaimable
        </span>}
        {usage.retainedCount > 0 && <span className="text-xs text-muted-foreground">
          {usage.retainedCount} kept for a reason
        </span>}
        {overBudget && <StatusPill tone="warning">Over the limit</StatusPill>}
      </div>
      {usage.repositories.length > 1 && <ul className="border-t border-border/60">
        {usage.repositories.map(repo => <li key={repo.repoRoot} className="flex items-center justify-between gap-4 px-4 py-2 text-xs">
          <span className="truncate font-mono text-muted-foreground" title={repo.repoRoot}>{repoName(repo.repoRoot)}</span>
          <span className="shrink-0 tabular-nums text-muted-foreground">
            {repo.count} · {bytes(repo.sizeBytes)}
          </span>
        </li>)}
      </ul>}
    </SettingsGroup>}

    {note && <p className="px-1 text-xs text-muted-foreground">{note}</p>}

    <SettingsGroup label="Checkouts" note={entries.length === 0 ? "Bridge has not created any worktrees yet." : undefined}>
      <ul className="divide-y divide-border/60">
        {entries.map(entry => {
          const external = entry.state === "external";
          const disposition = entry.disposition ?? (external ? "retained" : "");
          const actionable = !external && RECLAIMABLE.has(disposition);
          return <li key={entry.id} className="flex flex-col gap-1 px-4 py-3">
            <div className="flex items-center justify-between gap-3">
              <span className="flex min-w-0 items-center gap-2">
                <span className="truncate font-medium text-foreground" title={entry.path}>
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
            <div className="flex items-center justify-between gap-3">
              <span className="min-w-0 truncate text-xs text-muted-foreground">
                {external
                  ? "Not created by Bridge — shown for context, never reclaimed."
                  : entry.retainedReason ?? entry.path}
              </span>
              {actionable && <GhostButton
                onClick={() => void reclaim(entry)}
                disabled={busy !== null}
                ariaLabel={`Reclaim ${entry.branch ?? entry.path}`}
              >
                {busy === entry.id ? "Reclaiming…" : "Reclaim"}
              </GhostButton>}
            </div>
          </li>;
        })}
      </ul>
    </SettingsGroup>
  </SettingsPage>;
}
