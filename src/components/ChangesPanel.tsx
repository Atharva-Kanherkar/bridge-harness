import { Suspense, lazy, useCallback, useEffect, useRef, useState, useMemo } from "react";
import { Check, ChevronDown, Code2, Quote } from "lucide-react";
import type { RiskTier, Workspace, WorkspaceChangesResult, WorkspaceFileChange } from "../types";
import { bridgeApi } from "../api";
import { errorMessage } from "../errors";
import { cn } from "@/lib/utils";
import { Badge } from "@/components/ui/badge";
import { PatchView, type HunkRange } from "./DiffView";

const InlineFileEditor = lazy(() => import("./editor/InlineFileEditor").then(module => ({ default: module.InlineFileEditor })));

// The Changes pane: the live diff of the workspace, reviewed beside the
// conversation rather than instead of it. It fills whatever width the dock
// gives it, reloads in place when the workspace's stats drift under a running
// turn, and hands its findings onward — a file or hunk quoted into the
// composer, or a file opened in the Code pane.

/** Stats drift arrives in bursts while an agent writes; one reload after the
 *  burst is worth ten during it. */
export const STATS_REFRESH_DEBOUNCE_MS = 300;

const IMPORTANCE_RANK: Record<RiskTier, number> = { high: 0, medium: 1, low: 2 };
const IMPORTANCE_BADGE: Record<RiskTier, { label: string; variant: "error" | "warning" | "outline" }> = {
  high: { label: "High", variant: "error" },
  medium: { label: "Medium", variant: "warning" },
  low: { label: "Low", variant: "outline" },
};

function FileDiffView({ patch, binary, path, onQuoteHunk }: { patch: string; binary: boolean; path: string; onQuoteHunk?: (range: HunkRange) => void }) {
  if (binary) return <div className="px-3.5 py-4 text-[11.5px] text-muted-foreground">Binary file — no diff to show.</div>;
  if (!patch.trim()) return <div className="px-3.5 py-4 text-[11.5px] text-muted-foreground">No diff content.</div>;
  return <PatchView patch={patch} path={path} onQuoteHunk={onQuoteHunk} />;
}

/** Proportional add/delete bar. Silent when a file has no line changes. */
function DiffStatBar({ additions, deletions, className }: { additions: number; deletions: number; className?: string }) {
  const total = additions + deletions;
  if (!total) return null;
  return <span className={cn("flex h-1 w-10 shrink-0 overflow-hidden rounded-full bg-muted", className)} aria-hidden="true">
    <span className="bg-success" style={{ width: `${(additions / total) * 100}%` }} />
    <span className="bg-destructive" style={{ width: `${(deletions / total) * 100}%` }} />
  </span>;
}

function ChangeFileRow({ file, viewed, expanded, workspaceId, onToggleViewed, onToggleExpanded, onSaved, onQuote, onOpenFile }: {
  file: WorkspaceFileChange;
  viewed: boolean;
  expanded: boolean;
  workspaceId: string;
  onToggleViewed: () => void;
  onToggleExpanded: () => void;
  onSaved: () => void;
  onQuote?: (path: string, range?: HunkRange) => void;
  onOpenFile?: (path: string) => void;
}) {
  // Reading the diff and fixing what you just read are the same motion, so
  // the row carries both. Diff stays the default: review first.
  const [mode, setMode] = useState<"diff" | "edit">("diff");
  // Once a file has been edited the editor stays mounted — hidden behind the
  // diff, and kept alive through a collapse while it still holds unsaved text.
  // Unmounting it was the same data loss the tab switch used to cause.
  const [everEdited, setEverEdited] = useState(false);
  const [dirty, setDirty] = useState(false);
  // "Low" is the default state, so labelling it adds noise to every row. Only
  // a file that actually wants attention gets a badge.
  const badge = file.importance === "low" ? undefined : IMPORTANCE_BADGE[file.importance];
  const cut = file.path.lastIndexOf("/") + 1;
  return <div className={cn("transition-opacity", viewed && !expanded && "opacity-55")}>
    <div className="group flex items-center gap-2 px-2 py-1">
      <button type="button" onClick={onToggleExpanded} aria-expanded={expanded} className="flex min-w-0 flex-1 items-center gap-1.5 rounded-md px-1.5 py-1.5 text-left hover:bg-accent">
        <ChevronDown size={13} className={cn("shrink-0 text-muted-foreground/60 transition-transform", !expanded && "-rotate-90")} aria-hidden="true" />
        <span className="truncate font-mono text-[12px]">
          {cut > 0 && <span className="text-muted-foreground/70">{file.path.slice(0, cut)}</span>}
          <span className="text-foreground">{file.path.slice(cut)}</span>
        </span>
      </button>
      {dirty && <span className="shrink-0 text-[10.5px] text-warning" title="This file has unsaved edits in the inline editor">unsaved</span>}
      {onQuote && <button
        type="button"
        onClick={() => onQuote(file.path)}
        aria-label={`Reference ${file.path} in the composer`}
        title="Reference in the composer"
        className="hidden h-6 w-6 shrink-0 place-items-center rounded-md text-muted-foreground/70 hover:bg-accent hover:text-foreground focus-visible:grid group-hover:grid"
      >
        <Quote size={11} strokeWidth={1.8} aria-hidden="true" />
      </button>}
      {onOpenFile && <button
        type="button"
        onClick={() => onOpenFile(file.path)}
        aria-label={`Open ${file.path} in the Code pane`}
        title="Open in the Code pane"
        className="hidden h-6 w-6 shrink-0 place-items-center rounded-md text-muted-foreground/70 hover:bg-accent hover:text-foreground focus-visible:grid group-hover:grid"
      >
        <Code2 size={11} strokeWidth={1.8} aria-hidden="true" />
      </button>}
      {badge && <Badge variant={badge.variant} size="sm" className="hidden shrink-0 sm:inline-flex">{badge.label}</Badge>}
      <span className="hidden shrink-0 items-center gap-1.5 font-mono text-[10.5px] tabular-nums sm:flex">
        <span className="text-success">+{file.additions}</span>
        <span className="text-destructive">−{file.deletions}</span>
        <DiffStatBar additions={file.additions} deletions={file.deletions} />
      </span>
      <button
        type="button"
        onClick={onToggleViewed}
        aria-pressed={viewed}
        title={viewed ? "Mark as not viewed" : "Mark as viewed"}
        aria-label={viewed ? `Mark ${file.path} as not viewed` : `Mark ${file.path} as viewed`}
        className={cn(
          "grid h-6 w-6 shrink-0 place-items-center rounded-md border transition-colors",
          viewed ? "border-success/40 bg-success/10 text-success" : "border-border text-muted-foreground/60 hover:bg-accent hover:text-foreground",
        )}
      >
        <Check size={12} aria-hidden="true" />
      </button>
    </div>
    {(expanded || dirty) && <div className={cn("border-t border-border bg-code", !expanded && "hidden")}>
      {!file.binary && <div className="flex items-center gap-1 border-b border-border px-2 py-1">
        {(["diff", "edit"] as const).map(option => <button
          key={option}
          type="button"
          onClick={() => { setMode(option); if (option === "edit") setEverEdited(true); }}
          aria-pressed={mode === option}
          className={cn(
            "h-[20px] rounded-[5px] px-2 text-[10.5px] capitalize transition-colors",
            mode === option ? "bg-accent text-foreground" : "text-muted-foreground hover:text-foreground",
          )}
        >{option}</button>)}
      </div>}
      <div className={cn(mode === "edit" && !file.binary && "hidden")}>
        <FileDiffView patch={file.patch} binary={file.binary} path={file.path} onQuoteHunk={onQuote ? range => onQuote(file.path, range) : undefined} />
      </div>
      {everEdited && !file.binary && <div className={cn(mode !== "edit" && "hidden")}>
        <Suspense fallback={<div className="px-3.5 py-4 text-[11.5px] text-muted-foreground">Opening editor…</div>}>
          <InlineFileEditor workspaceId={workspaceId} path={file.path} onDirtyChange={setDirty} onSaved={onSaved} />
        </Suspense>
      </div>}
    </div>}
  </div>;
}

export function ChangesPanel({ workspace, onQuote, onOpenFile }: {
  workspace: Workspace;
  onQuote?: (path: string, range?: HunkRange) => void;
  onOpenFile?: (path: string) => void;
}) {
  const [changes, setChanges] = useState<WorkspaceChangesResult>();
  const [loadError, setLoadError] = useState<string>();
  const [expandedPaths, setExpandedPaths] = useState<Set<string>>(new Set());
  const [viewedPaths, setViewedPaths] = useState<Set<string>>(new Set());
  const [showLowSignal, setShowLowSignal] = useState(false);

  /** Re-read the changeset in place. Saving from an expanded row calls this,
   *  so the diff under the editor catches up without collapsing the review. */
  const reloadChanges = useCallback(async () => {
    try {
      setChanges(await bridgeApi.workspaceChanges(workspace.id));
      setLoadError(undefined);
    } catch (error) {
      setLoadError(errorMessage(error));
    }
  }, [workspace.id]);

  useEffect(() => {
    setChanges(undefined); setLoadError(undefined); setExpandedPaths(new Set()); setShowLowSignal(false);
    void reloadChanges();
  }, [reloadChanges]);

  // The workspace's own stats are the change signal: the runtime pushes them
  // while an agent writes, so a drift re-reads the diff without a manual
  // refresh — in place, after the burst settles.
  const statsSeen = useRef<string>();
  useEffect(() => {
    const stats = `${workspace.dirtyFiles}:${workspace.additions}:${workspace.deletions}`;
    if (statsSeen.current === undefined) {
      statsSeen.current = stats;
      return;
    }
    if (statsSeen.current === stats) return;
    statsSeen.current = stats;
    const timer = window.setTimeout(() => void reloadChanges(), STATS_REFRESH_DEBOUNCE_MS);
    return () => window.clearTimeout(timer);
  }, [workspace.dirtyFiles, workspace.additions, workspace.deletions, reloadChanges]);

  const sortedFiles = useMemo(() => [...(changes?.files ?? [])].sort((a, b) =>
    IMPORTANCE_RANK[a.importance] - IMPORTANCE_RANK[b.importance] || a.path.localeCompare(b.path)
  ), [changes]);
  const visibleFiles = sortedFiles.filter(file => showLowSignal || !file.lowSignal);
  const lowSignalCount = sortedFiles.length - sortedFiles.filter(file => !file.lowSignal).length;

  const toggle = (setter: typeof setExpandedPaths, path: string) => setter(previous => {
    const next = new Set(previous);
    if (next.has(path)) next.delete(path); else next.add(path);
    return next;
  });

  if (loadError) return <div className="p-[38px_28px]">
    <div className="text-muted-foreground/65 text-[10.5px] font-semibold tracking-[0.1em]">CHANGES</div>
    <p className="mt-2.5 text-destructive text-[13px]">{loadError}</p>
  </div>;

  if (!changes) return <div className="p-[38px_28px]">
    <div className="text-muted-foreground/65 text-[10.5px] font-semibold tracking-[0.1em]">CHANGES</div>
    <p className="mt-2.5 text-muted-foreground text-[13px]">Loading changes…</p>
  </div>;

  const totalAdditions = changes.files.reduce((sum, file) => sum + file.additions, 0);
  const totalDeletions = changes.files.reduce((sum, file) => sum + file.deletions, 0);
  const viewedCount = sortedFiles.filter(file => viewedPaths.has(file.path)).length;

  return <div className="h-full w-full overflow-y-auto px-3 py-4 sm:px-4">
    <div className="text-[10.5px] font-semibold tracking-[0.1em] text-muted-foreground/65">CHANGES</div>
    <h2 className="my-1.5 font-heading text-[16px] tracking-[-0.015em] text-foreground">{changes.files.length ? `${changes.files.length} file${changes.files.length === 1 ? "" : "s"} changed` : "Workspace is clean"}</h2>
    {/* What this diff is measured against — with worktree isolation, "which
        checkout am I looking at" should never be a question. */}
    <p className="mb-2 font-mono text-[10.5px] text-muted-foreground">
      {workspace.branch} · uncommitted vs HEAD{changes.baseCommit ? ` (${changes.baseCommit.slice(0, 8)})` : ""}
    </p>
    {changes.files.length === 0
      ? <p className="text-[13px] leading-relaxed text-muted-foreground">No uncommitted changes against HEAD.</p>
      : <>
        <div className="mb-3 flex flex-wrap items-center gap-x-3 gap-y-1.5 font-mono text-[11.5px]">
          <span className="text-success">+{totalAdditions}</span>
          <span className="text-destructive">−{totalDeletions}</span>
          <DiffStatBar additions={totalAdditions} deletions={totalDeletions} className="w-24" />
          <span className="ml-auto text-muted-foreground">{viewedCount}/{sortedFiles.length} viewed</span>
        </div>
        {/* One list with dividers, not a stack of floating cards — a review
            reads down a column of paths, and cards fight that. */}
        <div className="divide-y divide-border overflow-hidden rounded-lg border border-border bg-card">
          {visibleFiles.map(file => <ChangeFileRow
            key={file.path}
            file={file}
            workspaceId={workspace.id}
            onSaved={() => void reloadChanges()}
            viewed={viewedPaths.has(file.path)}
            expanded={expandedPaths.has(file.path)}
            onToggleViewed={() => toggle(setViewedPaths, file.path)}
            onToggleExpanded={() => toggle(setExpandedPaths, file.path)}
            onQuote={onQuote}
            onOpenFile={onOpenFile}
          />)}
        </div>
        {lowSignalCount > 0 && <button type="button" onClick={() => setShowLowSignal(value => !value)} className="mt-2.5 px-1.5 py-1 text-left text-[11.5px] text-muted-foreground transition-colors hover:text-foreground">
          {showLowSignal ? "Hide low-signal files" : `${lowSignalCount} low-signal file${lowSignalCount === 1 ? "" : "s"} hidden — show`}
        </button>}
      </>}
  </div>;
}
