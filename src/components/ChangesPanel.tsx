import { PaneState } from "@/components/ui/pane";
import { SkeletonRows } from "@/components/ui/skeleton";
import { Suspense, lazy, useCallback, useEffect, useRef, useState, useMemo } from "react";
import { Check, ChevronDown, Code2, Quote, FolderGit2, CircleAlert } from "lucide-react";
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

/** How often an on-screen Changes pane re-reads the changeset. The stats drift
 *  is the fast path; this catches the edits it cannot see: an agent rewriting
 *  an already-changed line leaves the "+1 −1" counts exactly as they were. */
export const CHANGES_POLL_MS = 4000;

const sameFile = (a: WorkspaceFileChange, b: WorkspaceFileChange) =>
  a.path === b.path && a.patch === b.patch && a.additions === b.additions && a.deletions === b.deletions
  && a.changeKind === b.changeKind && a.binary === b.binary && a.importance === b.importance
  && a.lowSignal === b.lowSignal && a.patchTruncated === b.patchTruncated && a.previousPath === b.previousPath
  && a.labels.join("\0") === b.labels.join("\0");

/** A poll that found the same changeset must not re-render every open diff. */
export function sameChangeset(a: WorkspaceChangesResult | undefined, b: WorkspaceChangesResult): boolean {
  return !!a && a.repositoryState === b.repositoryState && a.baseCommit === b.baseCommit
    && a.filesTruncated === b.filesTruncated && a.totalFiles === b.totalFiles
    && a.files.length === b.files.length && a.files.every((file, index) => sameFile(file, b.files[index]));
}

const IMPORTANCE_RANK: Record<RiskTier, number> = { high: 0, medium: 1, low: 2 };
const IMPORTANCE_BADGE: Record<RiskTier, { label: string; variant: "error" | "warning" | "outline" }> = {
  high: { label: "High", variant: "error" },
  medium: { label: "Medium", variant: "warning" },
  low: { label: "Low", variant: "outline" },
};

function FileDiffView({ patch, patchTruncated, binary, path, onQuoteHunk }: { patch: string; patchTruncated: boolean; binary: boolean; path: string; onQuoteHunk?: (range: HunkRange) => void }) {
  if (binary) return <div className="px-3.5 py-4 text-meta text-muted-foreground">Binary file — no diff to show.</div>;
  if (!patch.trim()) return <div className="px-3.5 py-4 text-meta text-muted-foreground">
    {patchTruncated ? "Patch preview omitted because the changeset reached its size limit. Open the file to review it." : "No diff content."}
  </div>;
  return <>
    <PatchView patch={patch} path={path} onQuoteHunk={onQuoteHunk} />
    {patchTruncated && <p role="status" className="border-t border-warning/25 bg-warning/8 px-3.5 py-2 text-caption text-warning">
      Patch preview truncated — additional diff content exists. Open the file to review the remainder.
    </p>}
  </>;
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
  const editable = !file.binary && file.changeKind !== "deleted";
  // "Low" is the default state, so labelling it adds noise to every row. Only
  // a file that actually wants attention gets a badge.
  const badge = file.importance === "low" ? undefined : IMPORTANCE_BADGE[file.importance];
  const cut = file.path.lastIndexOf("/") + 1;
  const kindLabel = file.changeKind === "mode_only" ? "mode" : file.changeKind;
  return <div className={cn("transition-opacity", viewed && !expanded && "opacity-55")}>
    <div className="group flex flex-wrap items-center gap-x-2 gap-y-1 px-2 py-2">
      <button type="button" onClick={onToggleExpanded} aria-expanded={expanded} aria-label={file.path} title={file.previousPath ? `${file.previousPath} → ${file.path}` : file.path} className="flex min-w-0 flex-1 items-center gap-1.5 rounded-md px-1.5 py-1.5 text-left hover:bg-accent @max-[560px]/changes:basis-full">
        <ChevronDown size={13} className={cn("shrink-0 text-muted-foreground transition-transform", !expanded && "-rotate-90")} aria-hidden="true" />
        <span className="flex min-w-0 font-mono text-meta">
          {(file.previousPath || cut > 0) && <span className="truncate text-muted-foreground">{file.previousPath && `${file.previousPath} → `}{file.path.slice(0, cut)}</span>}
          <span className="max-w-full shrink-0 truncate text-foreground">{file.path.slice(cut)}</span>
        </span>
      </button>
      {dirty && <span className="shrink-0 text-caption text-warning" title="This file has unsaved edits in the inline editor">unsaved</span>}
      {onQuote && <button
        type="button"
        onClick={() => onQuote(file.path)}
        aria-label={`Reference ${file.path} in the composer`}
        title="Reference in the composer"
        className="grid h-7 w-7 shrink-0 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
      >
        <Quote size={11} strokeWidth={1.8} aria-hidden="true" />
      </button>}
      {onOpenFile && file.changeKind !== "deleted" && <button
        type="button"
        onClick={() => onOpenFile(file.path)}
        aria-label={`Open ${file.path} in the Code pane`}
        title="Open in the Code pane"
        className="grid h-7 w-7 shrink-0 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
      >
        <Code2 size={11} strokeWidth={1.8} aria-hidden="true" />
      </button>}
      {file.changeKind !== "modified" && <Badge variant="outline" size="sm" className="shrink-0">{kindLabel}</Badge>}
      {file.labels.map(label => <Badge key={`${file.path}:${label}`} variant="outline" size="sm" className="shrink-0" title={`Path label: ${label}`}>{label}</Badge>)}
      {badge && <Badge variant={badge.variant} size="sm" className="hidden shrink-0 sm:inline-flex">{badge.label}</Badge>}
      <span className="hidden shrink-0 items-center gap-1.5 font-mono text-caption tabular-nums sm:flex">
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
          "grid h-7 w-7 shrink-0 place-items-center rounded-md border transition-colors",
          viewed ? "border-success/40 bg-success/10 text-success" : "border-border text-muted-foreground hover:bg-accent hover:text-foreground",
        )}
      >
        <Check size={12} aria-hidden="true" />
      </button>
    </div>
    {(expanded || dirty) && <div className={cn("border-t border-border bg-code", !expanded && "hidden")}>
      {editable && <div className="flex items-center gap-1 border-b border-border px-2 py-1">
        {(["diff", "edit"] as const).map(option => <button
          key={option}
          type="button"
          onClick={() => { setMode(option); if (option === "edit") setEverEdited(true); }}
          aria-pressed={mode === option}
          className={cn(
            "h-7 rounded-md px-2 text-meta capitalize transition-colors",
            mode === option ? "bg-accent text-foreground" : "text-muted-foreground hover:text-foreground",
          )}
        >{option}</button>)}
      </div>}
      <div className={cn(mode === "edit" && editable && "hidden")}>
        <FileDiffView patch={file.patch} patchTruncated={file.patchTruncated} binary={file.binary} path={file.path} onQuoteHunk={onQuote ? range => onQuote(file.path, range) : undefined} />
      </div>
      {everEdited && editable && <div className={cn(mode !== "edit" && "hidden")}>
        <Suspense fallback={<div className="px-3.5 py-4 text-meta text-muted-foreground">Opening editor…</div>}>
          <InlineFileEditor workspaceId={workspaceId} path={file.path} onDirtyChange={setDirty} onSaved={onSaved} />
        </Suspense>
      </div>}
    </div>}
  </div>;
}

export function ChangesPanel({ workspace, visible = false, onQuote, onOpenFile, onSaved }: {
  workspace: Workspace;
  /** On screen: poll the changeset as well as following the stats drift. */
  visible?: boolean;
  onQuote?: (path: string, range?: HunkRange) => void;
  onOpenFile?: (path: string) => void;
  /** A file was saved from an inline editor, so the workspace stats are stale. */
  onSaved?: () => void;
}) {
  const [changes, setChanges] = useState<WorkspaceChangesResult>();
  const [loadError, setLoadError] = useState<string>();
  const [expandedPaths, setExpandedPaths] = useState<Set<string>>(new Set());
  // Viewed is a claim about *this* patch. Keyed by the patch text, so a file
  // the agent changes again after you marked it comes back unviewed (as on
  // GitHub) instead of hiding new work behind a faded row.
  const [viewedPatches, setViewedPatches] = useState<Map<string, string>>(new Map());
  const [showLowSignal, setShowLowSignal] = useState(false);
  const requestGeneration = useRef(0);
  const statsSeen = useRef<string>();

  /** Re-read the changeset in place. Saving from an expanded row calls this,
   *  so the diff under the editor catches up without collapsing the review. */
  const reloadChanges = useCallback(async () => {
    const generation = ++requestGeneration.current;
    try {
      const next = await bridgeApi.workspaceChanges(workspace.id);
      if (generation !== requestGeneration.current) return;
      setChanges(current => sameChangeset(current, next) ? current : next);
      setLoadError(undefined);
    } catch (error) {
      if (generation !== requestGeneration.current) return;
      setLoadError(errorMessage(error));
    }
  }, [workspace.id]);

  useEffect(() => {
    setChanges(undefined); setLoadError(undefined); setExpandedPaths(new Set()); setViewedPatches(new Map()); setShowLowSignal(false); statsSeen.current = undefined;
    void reloadChanges();
    return () => { requestGeneration.current += 1; };
  }, [reloadChanges]);

  // The workspace's own stats are the change signal: the runtime pushes them
  // while an agent writes, so a drift re-reads the diff without a manual
  // refresh — in place, after the burst settles.
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

  // While on screen, re-read on a slow serial clock (first read after one
  // interval: mounting already loaded). A failed poll keeps the last good diff.
  useEffect(() => {
    if (!visible) return;
    let timer: number | undefined;
    let live = true;
    const tick = () => {
      timer = window.setTimeout(() => void reloadChanges().finally(() => { if (live) tick(); }), CHANGES_POLL_MS);
    };
    tick();
    return () => { live = false; window.clearTimeout(timer); };
  }, [visible, reloadChanges]);

  const handleSaved = useCallback(() => {
    void reloadChanges();
    onSaved?.();
  }, [reloadChanges, onSaved]);

  const sortedFiles = useMemo(() => [...(changes?.files ?? [])].sort((a, b) =>
    IMPORTANCE_RANK[a.importance] - IMPORTANCE_RANK[b.importance] || a.path.localeCompare(b.path)
  ), [changes]);
  const visibleFiles = sortedFiles.filter(file => showLowSignal || !file.lowSignal);
  const lowSignalCount = sortedFiles.length - sortedFiles.filter(file => !file.lowSignal).length;

  const toggleExpanded = (path: string) => setExpandedPaths(previous => {
    const next = new Set(previous);
    if (next.has(path)) next.delete(path); else next.add(path);
    return next;
  });
  const isViewed = (file: WorkspaceFileChange) => viewedPatches.get(file.path) === file.patch;
  const toggleViewed = (file: WorkspaceFileChange) => setViewedPatches(previous => {
    const next = new Map(previous);
    if (next.get(file.path) === file.patch) next.delete(file.path); else next.set(file.path, file.patch);
    return next;
  });

  if (loadError) return <PaneState icon={CircleAlert} title="Could not load changes" role="alert" action={<button type="button" onClick={() => void reloadChanges()} className="min-h-8 rounded-lg border border-border bg-card px-3 text-ui hover:bg-accent">Try again</button>}>{loadError}</PaneState>;
  if (!changes) return <SkeletonRows label="Loading changes" rows={5} lead="dot" meta={false} framed={false} className="min-h-0 flex-1 overflow-hidden" />;
  if (changes.repositoryState === "not_git") return <PaneState icon={FolderGit2} title="No Git repository">This folder is available in Code, but Changes needs a Git repository.</PaneState>;

  const totalAdditions = changes.files.reduce((sum, file) => sum + file.additions, 0);
  const totalDeletions = changes.files.reduce((sum, file) => sum + file.deletions, 0);
  const viewedCount = sortedFiles.filter(isViewed).length;
  const reportedFileCount = changes.totalFiles ?? changes.files.length;
  const fileCountLabel = changes.filesTruncated && changes.totalFiles == null ? `${changes.files.length}+` : `${reportedFileCount}`;
  const diffBasis = changes.repositoryState === "unborn"
    ? `${workspace.branch} · before first commit (against empty repository)`
    : `${workspace.branch} · uncommitted vs HEAD${changes.baseCommit ? ` (${changes.baseCommit.slice(0, 8)})` : ""}`;

  return <div className="@container/changes h-full w-full overflow-y-auto px-3 py-4 sm:px-4">
    <div className="text-meta font-medium text-muted-foreground">CHANGES</div>
    <h2 className="my-1.5 font-heading text-[20px] tracking-[-0.015em] text-foreground">{changes.files.length ? `${fileCountLabel} file${reportedFileCount === 1 ? "" : "s"} changed` : changes.repositoryState === "unborn" ? "No changes before first commit" : "Workspace is clean"}</h2>
    {/* What this diff is measured against — with worktree isolation, "which
        checkout am I looking at" should never be a question. */}
    <p className="mb-2 font-mono text-caption text-muted-foreground">
      {diffBasis}
    </p>
    {/* The turn cards in the chat show what this chat's agent edited; this
        pane is the folder's git state. Without the line, "3 files" in the
        chat and "241 files" here read as the same claim disagreeing. */}
    {changes.files.length > 0 && <p className="mb-2 text-caption leading-relaxed text-muted-foreground">
      Everything uncommitted in this folder, from any chat, agent, or editor. Each turn in the chat lists only its own edits.
    </p>}
    {changes.files.length === 0
      ? <p className="text-ui leading-relaxed text-muted-foreground">{changes.repositoryState === "unborn" ? "Create or stage a file to begin the first review." : "No uncommitted changes against HEAD."}</p>
      : <>
        <div className="mb-3 flex flex-wrap items-center gap-x-3 gap-y-1.5 font-mono text-meta">
          <span className="text-success">+{totalAdditions}</span>
          <span className="text-destructive">−{totalDeletions}</span>
          <DiffStatBar additions={totalAdditions} deletions={totalDeletions} className="w-24" />
          <span className="ml-auto text-muted-foreground">{viewedCount}/{sortedFiles.length}{changes.filesTruncated ? " shown" : ""} viewed</span>
        </div>
        {changes.filesTruncated && <p role="status" className="mb-3 rounded-md border border-warning/25 bg-warning/8 px-3 py-2 text-caption text-warning">
          {changes.totalFiles == null
            ? `Showing the first ${changes.files.length} files. Additional changed files exist beyond the scan limit.`
            : `Showing ${changes.files.length} of ${changes.totalFiles} changed files. Additional files were omitted from this bounded review.`}
        </p>}
        {/* One list with dividers, not a stack of floating cards — a review
            reads down a column of paths, and cards fight that. */}
        <div className="divide-y divide-border overflow-hidden rounded-lg border border-border bg-card">
          {visibleFiles.map(file => <ChangeFileRow
            key={file.path}
            file={file}
            workspaceId={workspace.id}
            onSaved={handleSaved}
            viewed={isViewed(file)}
            expanded={expandedPaths.has(file.path)}
            onToggleViewed={() => toggleViewed(file)}
            onToggleExpanded={() => toggleExpanded(file.path)}
            onQuote={onQuote}
            onOpenFile={onOpenFile}
          />)}
        </div>
        {lowSignalCount > 0 && <button type="button" onClick={() => setShowLowSignal(value => !value)} className="mt-2.5 px-1.5 py-1 text-left text-meta text-muted-foreground transition-colors hover:text-foreground">
          {showLowSignal ? "Hide low-signal files" : `${lowSignalCount} low-signal file${lowSignalCount === 1 ? "" : "s"} hidden — show`}
        </button>}
      </>}
  </div>;
}
