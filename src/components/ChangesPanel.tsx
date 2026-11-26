import { PaneState } from "@/components/ui/pane";
import { SkeletonRegion, SkeletonRows, SkeletonText } from "@/components/ui/skeleton";
import { Suspense, lazy, useCallback, useEffect, useRef, useState, useMemo, type ReactNode } from "react";
import { Check, ChevronDown, Code2, Quote, FileDiff, FolderGit2, CircleAlert } from "lucide-react";
import type { RiskTier, SessionChangesResult, SessionChangesUnavailable, Workspace, WorkspaceChangesResult, WorkspaceFileChange } from "../types";
import { bridgeApi } from "../api";
import { friendlyError } from "../errors";
import { cn } from "@/lib/utils";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { RowButton } from "@/components/ui/row";
import { Segmented } from "@/components/ui/segmented";
import { Tip } from "@/components/ui/tooltip";
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

/** Which changes the pane shows: what this chat's own turns changed, or
 *  everything uncommitted in the folder. */
export type ChangesScope = "chat" | "folder";

const UNAVAILABLE: Record<SessionChangesUnavailable, { title: string; body: string }> = {
  no_turns: { title: "No turns recorded yet", body: "This chat's changes appear after its first turn. Until then, the folder's uncommitted changes are one click away." },
  snapshot_failed: { title: "Could not measure this chat's changes", body: "Snapshotting the folder failed or ran out of time. The folder's uncommitted changes are still available." },
  not_git: { title: "No Git repository", body: "This chat's folder is not a Git repository." },
  no_folder: { title: "No folder", body: "This chat has no folder to measure." },
};

/** "42 minutes ago", coarse on purpose: it says when the baseline was taken. */
export function sinceLabel(iso: string | null | undefined, now = Date.now()): string {
  const then = iso ? Date.parse(iso) : Number.NaN;
  if (Number.isNaN(then)) return "";
  const minutes = Math.max(0, Math.round((now - then) / 60_000));
  if (minutes < 1) return "just now";
  if (minutes < 60) return `${minutes} minute${minutes === 1 ? "" : "s"} ago`;
  const hours = Math.round(minutes / 60);
  if (hours < 48) return `${hours} hour${hours === 1 ? "" : "s"} ago`;
  const days = Math.round(hours / 24);
  return `${days} days ago`;
}

const sameChat = (a: SessionChangesResult | undefined, b: SessionChangesResult) =>
  !!a && a.available === b.available && a.unavailableReason === b.unavailableReason && a.turns === b.turns
  && a.since === b.since && a.additions === b.additions && a.deletions === b.deletions
  && (a.changes == null ? b.changes == null : b.changes != null && sameChangeset(a.changes, b.changes));

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
      <Tip label={file.previousPath ? `${file.previousPath} → ${file.path}` : file.path}><RowButton size="sm" onClick={onToggleExpanded} aria-expanded={expanded} aria-label={file.path} className="flex-1 @max-[560px]/changes:basis-full">
        <ChevronDown size={13} className={cn("shrink-0 text-muted-foreground transition-transform", !expanded && "-rotate-90")} aria-hidden="true" />
        <span className="flex min-w-0 font-mono text-meta">
          {(file.previousPath || cut > 0) && <span className="truncate text-muted-foreground">{file.previousPath && `${file.previousPath} → `}{file.path.slice(0, cut)}</span>}
          <span className="max-w-full shrink-0 truncate text-foreground">{file.path.slice(cut)}</span>
        </span>
      </RowButton></Tip>
      {dirty && <Tip label="This file has unsaved edits in the inline editor"><span className="shrink-0 text-caption text-warning">unsaved</span></Tip>}
      {onQuote && <Button
        variant="quiet"
        size="icon-xs"
        onClick={() => onQuote(file.path)}
        aria-label={`Reference ${file.path} in the composer`}
        tooltip="Reference in the composer"
      >
        <Quote aria-hidden="true" />
      </Button>}
      {onOpenFile && file.changeKind !== "deleted" && <Button
        variant="quiet"
        size="icon-xs"
        onClick={() => onOpenFile(file.path)}
        aria-label={`Open ${file.path} in the Code pane`}
        tooltip="Open in the Code pane"
      >
        <Code2 aria-hidden="true" />
      </Button>}
      {file.changeKind !== "modified" && <Badge variant="outline" size="sm" className="shrink-0">{kindLabel}</Badge>}
      {file.labels.map(label => <Tip key={`${file.path}:${label}`} label={`Path label: ${label}`}><Badge variant="outline" size="sm" className="shrink-0">{label}</Badge></Tip>)}
      {badge && <Badge variant={badge.variant} size="sm" className="hidden shrink-0 sm:inline-flex">{badge.label}</Badge>}
      <span className="hidden shrink-0 items-center gap-1.5 font-mono text-caption tabular-nums sm:flex">
        <span className="text-success">+{file.additions}</span>
        <span className="text-destructive">−{file.deletions}</span>
        <DiffStatBar additions={file.additions} deletions={file.deletions} />
      </span>
      <Button
        variant="outline"
        size="icon-xs"
        onClick={onToggleViewed}
        aria-pressed={viewed}
        tooltip={viewed ? "Mark as not viewed" : "Mark as viewed"}
        aria-label={viewed ? `Mark ${file.path} as not viewed` : `Mark ${file.path} as viewed`}
      >
        <Check className={viewed ? "text-success" : "text-muted-foreground"} aria-hidden="true" />
      </Button>
    </div>
    {(expanded || dirty) && <div className={cn("border-t border-border bg-code", !expanded && "hidden")}>
      {editable && <div className="flex items-center gap-1 border-b border-border px-2 py-1">
        <Segmented label="View" value={mode} options={[{ value: "diff" as const, label: "Diff" }, { value: "edit" as const, label: "Edit" }]} onChange={option => { setMode(option); if (option === "edit") setEverEdited(true); }} />
      </div>}
      <div className={cn(mode === "edit" && editable && "hidden")}>
        <FileDiffView patch={file.patch} patchTruncated={file.patchTruncated} binary={file.binary} path={file.path} onQuoteHunk={onQuote ? range => onQuote(file.path, range) : undefined} />
      </div>
      {everEdited && editable && <div className={cn(mode !== "edit" && "hidden")}>
        <Suspense fallback={<SkeletonRegion label="Loading editor" className="px-3.5 py-4"><SkeletonText lines={4} /></SkeletonRegion>}>
          <InlineFileEditor workspaceId={workspaceId} path={file.path} onDirtyChange={setDirty} onSaved={onSaved} />
        </Suspense>
      </div>}
    </div>}
  </div>;
}

export function ChangesPanel({ workspace, sessionId, chatSignal, visible = false, onQuote, onOpenFile, onSaved }: {
  workspace: Workspace;
  /** The chat on screen. With it the pane opens on what this chat's turns
   *  changed; without it only the folder's uncommitted changes exist. */
  sessionId?: string;
  /** Changes when the chat's own stats drift, like the workspace stats do. */
  chatSignal?: string;
  /** On screen: poll the changeset as well as following the stats drift. */
  visible?: boolean;
  onQuote?: (path: string, range?: HunkRange) => void;
  onOpenFile?: (path: string) => void;
  /** A file was saved from an inline editor, so the workspace stats are stale. */
  onSaved?: () => void;
}) {
  const [scope, setScope] = useState<ChangesScope>(sessionId ? "chat" : "folder");
  // What was loaded, and for which scope: switching scopes must never show
  // one scope's files under the other's heading.
  const [loaded, setLoaded] = useState<{ scope: ChangesScope; changes?: WorkspaceChangesResult; chat?: SessionChangesResult }>();
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
      if (scope === "chat" && sessionId) {
        const chat = await bridgeApi.sessionChanges(sessionId, true);
        if (generation !== requestGeneration.current) return;
        setLoaded(current => current?.scope === "chat" && sameChat(current.chat, chat) ? current : { scope: "chat", chat, changes: chat.changes ?? undefined });
      } else {
        const next = await bridgeApi.workspaceChanges(workspace.id);
        if (generation !== requestGeneration.current) return;
        setLoaded(current => current?.scope === "folder" && current.changes && sameChangeset(current.changes, next) ? current : { scope: "folder", changes: next });
      }
      setLoadError(undefined);
    } catch (error) {
      if (generation !== requestGeneration.current) return;
      setLoadError(friendlyError(error, "Bridge couldn't read the changes. Try again."));
    }
  }, [scope, sessionId, workspace.id]);

  useEffect(() => {
    setLoaded(undefined); setLoadError(undefined); setExpandedPaths(new Set()); setViewedPatches(new Map()); setShowLowSignal(false); statsSeen.current = undefined;
    void reloadChanges();
    return () => { requestGeneration.current += 1; };
  }, [reloadChanges]);

  // The workspace's own stats are the change signal: the runtime pushes them
  // while an agent writes, so a drift re-reads the diff without a manual
  // refresh — in place, after the burst settles.
  useEffect(() => {
    const stats = `${workspace.dirtyFiles}:${workspace.additions}:${workspace.deletions}:${chatSignal ?? ""}`;
    if (statsSeen.current === undefined) {
      statsSeen.current = stats;
      return;
    }
    if (statsSeen.current === stats) return;
    statsSeen.current = stats;
    const timer = window.setTimeout(() => void reloadChanges(), STATS_REFRESH_DEBOUNCE_MS);
    return () => window.clearTimeout(timer);
  }, [workspace.dirtyFiles, workspace.additions, workspace.deletions, chatSignal, reloadChanges]);

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

  const current = loaded?.scope === scope ? loaded : undefined;
  const changes = current?.changes;
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

  // The switch is part of every state, so a chat with nothing to show can
  // still reach the folder, and the folder can always come back to the chat.
  const scopeSwitch = sessionId ? <Segmented label="Which changes" className="mb-3 w-fit" value={scope} options={[{ value: "chat" as const, label: "This chat" }, { value: "folder" as const, label: "All uncommitted" }]} onChange={setScope} /> : null;
  const frame = (body: ReactNode) => <div className="@container/changes flex h-full w-full flex-col overflow-y-auto px-3 py-4 sm:px-4">
    <div className="text-meta font-medium text-muted-foreground">CHANGES</div>
    <div className="mt-2">{scopeSwitch}</div>
    {body}
  </div>;

  if (loadError) return frame(<PaneState icon={CircleAlert} title="Could not load changes" role="alert" action={<Button variant="outline" size="sm" onClick={() => void reloadChanges()}>Try again</Button>}>{loadError}</PaneState>);
  if (!current) return frame(<SkeletonRows label="Loading changes" rows={5} lead="dot" meta={false} framed={false} className="min-h-0 flex-1 overflow-hidden" />);
  if (current.chat && !current.chat.available) {
    const reason = UNAVAILABLE[current.chat.unavailableReason ?? "no_turns"];
    return frame(<PaneState icon={FileDiff} title={reason.title} role="status" action={<Button variant="outline" size="sm" onClick={() => setScope("folder")}>Show all uncommitted</Button>}>{reason.body}</PaneState>);
  }
  if (!changes) return frame(<SkeletonRows label="Loading changes" rows={5} lead="dot" meta={false} framed={false} className="min-h-0 flex-1 overflow-hidden" />);
  if (changes.repositoryState === "not_git") return frame(<PaneState icon={FolderGit2} title="No Git repository">This folder is available in Code, but Changes needs a Git repository.</PaneState>);
  const chatScope = current.scope === "chat" ? current.chat : undefined;

  // A chat reports totals across all its files; the folder's bounded window
  // only has the files it carries, and says so below when it is cut.
  const totalAdditions = chatScope ? chatScope.additions : changes.files.reduce((sum, file) => sum + file.additions, 0);
  const totalDeletions = chatScope ? chatScope.deletions : changes.files.reduce((sum, file) => sum + file.deletions, 0);
  const totalsCoverShownOnly = !chatScope && changes.filesTruncated;
  const viewedCount = sortedFiles.filter(isViewed).length;
  const reportedFileCount = changes.totalFiles ?? changes.files.length;
  const fileCountLabel = changes.filesTruncated && changes.totalFiles == null ? `${changes.files.length}+` : `${reportedFileCount}`;
  const diffBasis = chatScope
    ? `${workspace.branch} · ${chatScope.turns} turn${chatScope.turns === 1 ? "" : "s"} in this chat${chatScope.since ? ` · since ${sinceLabel(chatScope.since)}` : ""}`
    : changes.repositoryState === "unborn"
      ? `${workspace.branch} · before first commit (against empty repository)`
      : `${workspace.branch} · uncommitted vs HEAD${changes.baseCommit ? ` (${changes.baseCommit.slice(0, 8)})` : ""}`;
  const emptyHeading = chatScope ? "No changes from this chat" : changes.repositoryState === "unborn" ? "No changes before first commit" : "Workspace is clean";

  return <div className="@container/changes h-full w-full overflow-y-auto px-3 py-4 sm:px-4">
    <div className="text-meta font-medium text-muted-foreground">CHANGES</div>
    <div className="mt-2">{scopeSwitch}</div>
    <h2 className="my-1.5 font-heading text-xl tracking-tight text-foreground">{changes.files.length ? `${fileCountLabel} file${reportedFileCount === 1 ? "" : "s"} changed` : emptyHeading}</h2>
    {/* What this diff is measured against — with worktree isolation, "which
        checkout am I looking at" should never be a question. */}
    <p className="mb-2 font-mono text-caption text-muted-foreground">
      {diffBasis}
    </p>
    {/* The turn cards in the chat show what this chat's agent edited; this
        pane is the folder's git state. Without the line, "3 files" in the
        chat and "241 files" here read as the same claim disagreeing. */}
    {changes.files.length > 0 && <p className="mb-2 text-caption leading-relaxed text-muted-foreground">
      {chatScope
        ? "Only files this chat's turns changed, measured from before its first turn."
        : "Everything uncommitted in this folder, from any chat, agent, or editor."}
    </p>}
    {changes.files.length === 0
      ? <p className="text-ui leading-relaxed text-muted-foreground">{chatScope ? "This chat's turns have not changed any files, or their changes were undone." : changes.repositoryState === "unborn" ? "Create or stage a file to begin the first review." : "No uncommitted changes against HEAD."}</p>
      : <>
        <div className="mb-3 flex flex-wrap items-center gap-x-3 gap-y-1.5 font-mono text-meta">
          <span className="text-success">+{totalAdditions}</span>
          <span className="text-destructive">−{totalDeletions}</span>
          <DiffStatBar additions={totalAdditions} deletions={totalDeletions} className="w-24" />
          {totalsCoverShownOnly && <span className="text-muted-foreground">in the {changes.files.length} shown</span>}
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
        {lowSignalCount > 0 && <Button variant="text" size="xs" onClick={() => setShowLowSignal(value => !value)} className="mt-2.5 w-fit">
          {showLowSignal ? "Hide low-signal files" : `${lowSignalCount} low-signal file${lowSignalCount === 1 ? "" : "s"} hidden — show`}
        </Button>}
      </>}
  </div>;
}
