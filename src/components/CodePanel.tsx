import { memo, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { ChevronRight, CornerDownLeft, FileSearch, Folder, FolderOpen, PanelLeft, RefreshCw, X } from "lucide-react";
import { bridgeApi } from "../api";
import { friendlyError } from "../errors";
import { ancestorPaths, buildFileTree, collapseChains, rankPaths, type TreeNode } from "../fileTree";
import { glyphFor } from "./fileGlyph";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Kbd } from "@/components/ui/kbd";
import { RowButton } from "@/components/ui/row";
import { Tip } from "@/components/ui/tooltip";
import { Dialog, DialogPopup } from "./ui/dialog";
import { PaneState } from "./ui/pane";
import { CodeEditor } from "./editor/CodeEditor";
import { isDirty, isReadOnly, loadBuffer, saveBuffer, settleSave, stateAfterEdit, statusLabel, type FileBuffer } from "./editor/fileBuffer";
import { STATS_REFRESH_DEBOUNCE_MS } from "./ChangesPanel";
import { modKey } from "@/lib/platform";
import { SkeletonDot } from "@/components/ui/skeleton";
import { useLatest } from "../useLatest";

/** Rows the tree actually paints. Deep repositories stay responsive because
 *  only expanded directories contribute. */
const MAX_TREE_ROWS = 1500;

/** How often an on-screen Code pane re-reads its open files. Slow on purpose:
 *  the stats drift is the fast path, this only catches what it cannot see. */
export const OPEN_FILE_PROBE_MS = 4000;

/** A tab: its buffer, plus the last in-place replacement handed to its editor. */
type OpenFile = FileBuffer & { sync?: { text: string; version: number } };

const isEditable = (file: OpenFile) => Boolean(file.baseSha) && !isReadOnly(file);

const sameList = (a: string[], b: string[]) => a.length === b.length && a.every((value, index) => value === b[index]);

interface Row {
  node: TreeNode;
  depth: number;
  expanded: boolean;
}

function flatten(nodes: TreeNode[], expanded: Set<string>, depth = 0, out: Row[] = []): Row[] {
  for (const node of nodes) {
    if (out.length >= MAX_TREE_ROWS) return out;
    const isOpen = Boolean(node.children) && expanded.has(node.path);
    out.push({ node, depth, expanded: isOpen });
    if (isOpen && node.children) flatten(node.children, expanded, depth + 1, out);
  }
  return out;
}

/**
 * One row of the tree.
 *
 * Depth is drawn, not padded: a row at depth *n* renders *n* hairline guides,
 * which both indents it and shows which trunk it hangs off. Left padding alone
 * gave a deep tree no visible structure at all — every row was the same
 * monospace grey at a slightly different offset.
 */
const TreeRow = memo(function TreeRow({ row, active, onToggle, onOpen }: {
  row: Row;
  active: boolean;
  onToggle: (path: string) => void;
  onOpen: (path: string) => void;
}) {
  const isDir = Boolean(row.node.children);
  const { Icon, tint } = isDir
    ? { Icon: row.expanded ? FolderOpen : Folder, tint: "text-muted-foreground" }
    : glyphFor(row.node.path);
  return <RowButton
    size="bare"
    active={active}
    onClick={() => (isDir ? onToggle(row.node.path) : onOpen(row.node.path))}
    aria-expanded={isDir ? row.expanded : undefined}
    className="group/row"
  ><span className="flex h-7 w-full min-w-0 items-stretch truncate pr-2 font-mono text-meta">
    {Array.from({ length: row.depth }, (_, level) => <span
      key={level}
      // The guide is the indent. It sits at the *right* edge of each 11px
      // step, which lines it up with the chevron column of the parent
      // directory rather than floating between rows.
      className="w-[11px] shrink-0 border-r border-border/50"
      aria-hidden="true"
    />)}
    <span className="flex min-w-0 flex-1 items-center gap-1 pl-1.5">
      {isDir
        ? <ChevronRight size={11} className={cn("shrink-0 text-muted-foreground transition-transform", row.expanded && "rotate-90")} aria-hidden="true" />
        : <span className="w-[11px] shrink-0" aria-hidden="true" />}
      {/* Files tint from the syntax ramp — the same palette the file's own
          code is painted with, so the tree previews what you are about to
          open. Directories stay achromatic: they are chrome, not content. */}
      <Icon size={12} strokeWidth={1.75} className={cn("shrink-0", tint)} aria-hidden="true" />
      <span className={cn(
        "truncate",
        // A directory is a container and reads as one: full foreground and a
        // touch of weight. Files stay quiet until hovered or active.
        isDir ? "font-medium text-foreground/80" : active ? "text-foreground" : "text-muted-foreground group-hover/row:text-foreground",
      )}>{row.node.name}</span>
    </span>
  </span></RowButton>;
});

/** ⌘P. Ranked over the same path list the tree is built from. */
function FilePalette({ paths, onPick, onClose }: { paths: string[]; onPick: (path: string) => void; onClose: () => void }) {
  const [query, setQuery] = useState("");
  const [index, setIndex] = useState(0);
  const results = useMemo(() => rankPaths(paths, query), [paths, query]);
  const active = results[Math.min(index, results.length - 1)];
  const list = useRef<HTMLDivElement>(null);
  useEffect(() => setIndex(0), [query]);
  // Arrowing past the fold used to move the highlight off-screen while the
  // list stayed put.
  useEffect(() => {
    list.current?.querySelector<HTMLElement>("[data-active='true']")?.scrollIntoView?.({ block: "nearest" });
  }, [active]);

  return <Dialog open onOpenChange={open => { if (!open) onClose(); }}>
    <DialogPopup aria-label="Open file" showCloseButton={false} className="max-h-[70dvh] max-w-xl overflow-hidden">
      <div className="flex items-center gap-2 border-b border-border px-3">
        <FileSearch size={14} className="shrink-0 text-muted-foreground" aria-hidden="true" />
        <Input
          unstyled
          mono
          size="lg"
          className="flex-1"
          autoFocus
          value={query}
          onChange={event => setQuery(event.target.value)}
          onKeyDown={event => {
            if (event.key === "ArrowDown") { event.preventDefault(); setIndex(value => Math.min(value + 1, results.length - 1)); }
            else if (event.key === "ArrowUp") { event.preventDefault(); setIndex(value => Math.max(value - 1, 0)); }
            else if (event.key === "Enter" && active) { event.preventDefault(); onPick(active); }
            else if (event.key === "Escape") { event.preventDefault(); onClose(); }
          }}
          placeholder="Go to file…"
          aria-label="Go to file"
        />
        <Kbd>esc</Kbd>
      </div>
      <div ref={list} className="max-h-[46vh] overflow-y-auto p-1">
        {results.length === 0
          ? <p className="px-2 py-6 text-center text-meta text-muted-foreground">No file matches “{query}”.</p>
          : results.map((path, position) => {
            const cut = path.lastIndexOf("/") + 1;
            const { Icon, tint } = glyphFor(path);
            return <RowButton
              key={path}
              tone="muted"
              active={path === active}
              onMouseEnter={() => setIndex(position)}
              onClick={() => onPick(path)}
            >
              <Icon size={12} strokeWidth={1.75} className={cn("shrink-0", tint)} aria-hidden="true" />
              <span className="truncate font-mono text-meta">
                <span className="text-foreground">{path.slice(cut)}</span>
                {cut > 0 && <span className="ml-2 text-muted-foreground">{path.slice(0, cut - 1)}</span>}
              </span>
              {path === active && <CornerDownLeft size={11} className="ml-auto shrink-0 text-muted-foreground" aria-hidden="true" />}
            </RowButton>;
          })}
      </div>
    </DialogPopup>
  </Dialog>;
}

/**
 * The Code tab: a file tree, open-file tabs, and a real editor over the
 * workspace's working tree.
 *
 * Agents write this same tree, so every save carries the hash the buffer was
 * read at and the backend refuses a write when disk has moved underneath.
 * That turns the one genuinely dangerous case — a human and an agent editing
 * the same file — into a visible choice instead of a silent lost update.
 */
export function CodePanel({ workspaceId, visible = true, reveal, driftSignal, onSaved }: {
  workspaceId: string;
  /** False while another tab is showing: the panel stays mounted, but its
   *  shortcuts must not steal ⌘P and ⌘S from whatever is on screen. */
  visible?: boolean;
  /** An outside request — a diff row, a chat mention — to open a file here,
   *  at a line when one is known. The nonce is the request identity: one
   *  open per nonce, so a re-render with the same request does not
   *  re-activate the tab. */
  reveal?: { path: string; line?: number; nonce: number };
  /** Changes when the workspace's stats drift — the cue to compare every
   *  open buffer against the disk the agent just wrote. */
  driftSignal?: string;
  onSaved?: () => void;
}) {
  const [paths, setPaths] = useState<string[]>([]);
  const [treeError, setTreeError] = useState<string>();
  const [loadingTree, setLoadingTree] = useState(true);
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const [open, setOpen] = useState<OpenFile[]>([]);
  const [activePath, setActivePath] = useState<string>();
  const [paletteOpen, setPaletteOpen] = useState(false);
  // Below the container breakpoint the tree is a drawer over the editor. It
  // used to be `hidden` outright, and the dock's default width sits below that
  // breakpoint, so the Code tab opened with no way to browse at all.
  const [treeOpen, setTreeOpen] = useState(false);
  // Live buffers live outside React: a keystroke must not re-render the tree.
  const buffers = useRef(new Map<string, string>());
  // One counter per path, bumped on close. A load that finishes after its tab
  // was closed — or after a second load for the same path started — is stale,
  // and applying it would resurrect the tab or reset what you just typed.
  const opens = useRef(new Map<string, number>());
  const loading = useRef(new Set<string>());
  // Paths with a write in flight, and the text each write carries. A ref, not
  // `state === "saving"`: ⌘S and the Save link in one tick both read the same
  // pre-render snapshot, and the second write, still carrying the old hash,
  // came back as a false "changed on disk" conflict.
  const saving = useRef(new Map<string, string>());
  // The latest committed tabs, for callbacks that must not be rebuilt (and
  // must not cancel their own timers) every time a tab's state flips.
  const openRef = useLatest(open);
  const mounted = useRef(true);
  useEffect(() => () => { mounted.current = false; }, []);

  const loadTree = useCallback(async (quiet = false) => {
    if (!quiet) setLoadingTree(true);
    try {
      const next = await bridgeApi.listWorkspaceTree(workspaceId);
      if (!mounted.current) return;
      // A background refresh that found nothing new must not rebuild the tree.
      setPaths(current => sameList(current, next) ? current : next);
      setTreeError(undefined);
    } catch (error) {
      // A failed background refresh keeps the tree you were using.
      if (!quiet && mounted.current) setTreeError(friendlyError(error, "Couldn't list the files in this workspace."));
    } finally {
      if (!quiet && mounted.current) setLoadingTree(false);
    }
  }, [workspaceId]);

  useEffect(() => { void loadTree(); }, [loadTree]);

  const tree = useMemo(() => collapseChains(buildFileTree(paths)), [paths]);
  const rows = useMemo(() => flatten(tree, expanded), [tree, expanded]);
  const active = open.find(file => file.path === activePath);

  const openFile = useCallback(async (path: string) => {
    setPaletteOpen(false);
    setTreeOpen(false);
    setActivePath(path);
    // Reveal it in the tree, so the palette and the tree never disagree.
    setExpanded(previous => {
      const next = new Set(previous);
      for (const ancestor of ancestorPaths(path)) next.add(ancestor);
      return next;
    });
    if (openRef.current.some(file => file.path === path) || loading.current.has(path)) return;
    loading.current.add(path);
    const generation = opens.current.get(path) ?? 0;
    try {
      const buffer = await loadBuffer(workspaceId, path);
      if ((opens.current.get(path) ?? 0) !== generation || !mounted.current) return;
      buffers.current.set(path, buffer.saved);
      setOpen(files => files.some(entry => entry.path === path) ? files : [...files, buffer]);
    } finally {
      loading.current.delete(path);
    }
  }, [workspaceId, openRef]);

  // One open per reveal nonce. The ref carries the last honoured request so a
  // re-render with the same reveal object is inert.
  const revealSeen = useRef(0);
  useEffect(() => {
    if (!reveal || reveal.nonce === revealSeen.current) return;
    revealSeen.current = reveal.nonce;
    void openFile(reveal.path);
  }, [reveal, openFile]);

  const closeFile = useCallback((path: string) => {
    const files = openRef.current;
    const file = files.find(entry => entry.path === path);
    const live = buffers.current.get(path);
    const pendingWrite = saving.current.get(path);
    const unsaved = file && (isDirty(file) || (pendingWrite !== undefined && live !== pendingWrite));
    if (unsaved && !window.confirm(`Discard unsaved changes to ${path}?`)) return;
    // Retire any load still in flight for this path along with the tab.
    opens.current.set(path, (opens.current.get(path) ?? 0) + 1);
    buffers.current.delete(path);
    const index = files.findIndex(entry => entry.path === path);
    const remaining = files.filter(entry => entry.path !== path);
    // The neighbour takes focus, as in every tabbed editor, not whichever
    // tab happened to be opened last.
    setActivePath(current => current === path ? (remaining[index] ?? remaining[index - 1])?.path : current);
    setOpen(current => current.filter(entry => entry.path !== path));
  }, [openRef]);

  /** Typing only re-renders when the *dirty flag* flips, not on every key. */
  const handleChange = useCallback((path: string, value: string) => {
    buffers.current.set(path, value);
    setOpen(files => {
      const file = files.find(entry => entry.path === path);
      const next = file && stateAfterEdit(file, value);
      // Returning the same array is what stops a keystroke re-rendering the
      // tree and the tab strip.
      if (!next) return files;
      return files.map(entry => entry.path === path ? { ...entry, state: next, message: next === "clean" ? undefined : entry.message } : entry);
    });
  }, []);

  const patch = useCallback((path: string, change: Partial<OpenFile>) => {
    setOpen(files => files.map(file => file.path === path ? { ...file, ...change } : file));
  }, []);

  const save = useCallback(async (path: string, force = false) => {
    const file = openRef.current.find(entry => entry.path === path);
    if (!file || isReadOnly(file) || saving.current.has(path)) return;
    // A tab whose read failed holds nothing to write.
    if (file.state === "error" && !file.baseSha) return;
    const content = buffers.current.get(path) ?? file.saved;
    // ⌘S on a clean tab is a habit, not a request to rewrite the file.
    if (!force && file.state === "clean" && content === file.saved) return;
    saving.current.set(path, content);
    const generation = opens.current.get(path) ?? 0;
    patch(path, { state: "saving", message: undefined });
    try {
      const next = await saveBuffer(workspaceId, file, content, force);
      if (next.state === "clean") onSaved?.();
      if ((opens.current.get(path) ?? 0) !== generation || !mounted.current) return;
      const settled = settleSave(next, buffers.current.get(path));
      setOpen(files => files.map(entry => entry.path === path ? { ...settled, sync: entry.sync } : entry));
    } finally {
      saving.current.delete(path);
    }
  }, [onSaved, patch, workspaceId, openRef]);

  /** Discard the local buffer and take what is on disk now, in place, so the
   *  caret and scroll survive. */
  const reload = useCallback(async (path: string) => {
    const generation = opens.current.get(path) ?? 0;
    const buffer = await loadBuffer(workspaceId, path);
    if ((opens.current.get(path) ?? 0) !== generation || !mounted.current) return;
    buffers.current.set(path, buffer.saved);
    setOpen(files => files.map(entry => entry.path === path
      ? { ...buffer, seed: entry.seed, sync: { text: buffer.saved, version: (entry.sync?.version ?? 0) + 1 } }
      : entry));
  }, [workspaceId]);

  /**
   * Compare every open buffer against disk. A clean buffer takes the new
   * bytes in place; a dirty one turns its conflict state on (the same state
   * a refused save produces) and keeps the unsaved text.
   *
   * Reads the tabs from a ref and decides inside the state updater: a probe
   * is a chain of awaits, and the tabs it started from are stale by the time
   * each read lands.
   */
  const probing = useRef(false);
  const probe = useCallback(async () => {
    if (probing.current) return;
    probing.current = true;
    try {
      for (const file of openRef.current) {
        if (!mounted.current) return;
        if (file.state === "error" || saving.current.has(file.path)) continue;
        try {
          // Captured before the read: a tab closed (or closed and reopened)
          // while the probe was in flight must not be resurrected by it.
          const generation = opens.current.get(file.path) ?? 0;
          const disk = await bridgeApi.readWorkspaceFile(workspaceId, file.path);
          if (!mounted.current) return;
          if ((opens.current.get(file.path) ?? 0) !== generation || saving.current.has(file.path)) continue;
          setOpen(files => files.map(entry => {
            if (entry.path !== file.path || entry.baseSha === disk.sha256 || entry.state === "saving" || entry.state === "error") return entry;
            if (isDirty(entry)) {
              return entry.state === "conflict" ? entry : { ...entry, state: "conflict", message: "Changed on disk while you were editing" };
            }
            buffers.current.set(entry.path, disk.content);
            return {
              ...entry, baseSha: disk.sha256, saved: disk.content, binary: disk.binary,
              tooLarge: disk.tooLarge, sizeBytes: disk.sizeBytes, state: "clean", message: undefined,
              sync: { text: disk.content, version: (entry.sync?.version ?? 0) + 1 },
            };
          }));
        } catch {
          // A vanished file surfaces on the next save; drift polling stays quiet.
        }
      }
    } finally {
      probing.current = false;
    }
  }, [workspaceId, openRef]);

  // The agent writes the same tree this panel edits. When the workspace's
  // stats settle after a drift, probe the open buffers and pick up files the
  // agent created. Debounced like the Changes pane, because stats tick many
  // times a second under a writing agent. Depends only on the signal and
  // stable callbacks: it used to depend on the tab list too, so any tab
  // update inside the debounce window cancelled the probe for good.
  const driftSeen = useRef(driftSignal);
  useEffect(() => {
    if (driftSignal === undefined || driftSignal === driftSeen.current) return;
    driftSeen.current = driftSignal;
    const timer = window.setTimeout(() => {
      void probe();
      void loadTree(true);
    }, STATS_REFRESH_DEBOUNCE_MS);
    return () => window.clearTimeout(timer);
  }, [driftSignal, probe, loadTree]);

  // Line counts are a coarse signal: an agent rewriting an already-changed
  // line leaves "+1 −1" exactly as it was. While the pane is on screen, probe
  // on a slow clock too, so what you are reading is never the old bytes.
  useEffect(() => {
    if (!visible) return;
    let timer: number | undefined;
    let live = true;
    const tick = () => {
      timer = window.setTimeout(() => void (async () => {
        if (openRef.current.length) await probe();
        if (live) tick();
      })(), OPEN_FILE_PROBE_MS);
    };
    tick();
    return () => { live = false; window.clearTimeout(timer); };
  }, [visible, probe, openRef]);

  // ⌘P and ⌘S also work when focus is in the tree or the tab strip; the
  // editor has its own ⌘S so a save never depends on where the caret is.
  useEffect(() => {
    if (!visible) return;
    const onKeyDown = (event: KeyboardEvent) => {
      // A dialog above the pane (the apps launcher pins on ⌘P), or the
      // editor's own ⌘S binding, already took it.
      if (event.defaultPrevented) return;
      if (!(event.metaKey || event.ctrlKey)) return;
      const key = event.key.toLowerCase();
      if (key === "p") { event.preventDefault(); setPaletteOpen(value => !value); }
      else if (key === "s" && activePath) { event.preventDefault(); void save(activePath); }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [visible, activePath, save]);

  const toggleDir = useCallback((path: string) => {
    setExpanded(previous => {
      const next = new Set(previous);
      if (!next.delete(path)) next.add(path);
      return next;
    });
  }, []);

  const dirtyCount = open.filter(isDirty).length;

  return <div className="@container/code relative flex h-full min-h-0">
    {/* oxlint-disable-next-line bridge/no-raw-controls -- a scrim, not a control: a click outside the drawer closes it */}
    {treeOpen && <button
      type="button"
      aria-label="Close file list"
      onClick={() => setTreeOpen(false)}
      className="absolute inset-0 z-20 bg-background/40 @min-[640px]/code:hidden"
    />}
    <aside
      onKeyDown={event => { if (event.key === "Escape" && treeOpen) { event.preventDefault(); setTreeOpen(false); } }}
      className={cn(
        "w-52 shrink-0 flex-col border-r border-border bg-muted/20",
        treeOpen
          ? "absolute inset-y-0 left-0 z-30 flex bg-background shadow-lg @min-[640px]/code:static @min-[640px]/code:bg-muted/20 @min-[640px]/code:shadow-none"
          : "hidden @min-[640px]/code:flex",
      )}
    >
      <div className="flex h-10 shrink-0 items-center gap-1 border-b border-border pl-3 pr-1.5">
        <span className="flex-1 truncate text-meta font-medium text-muted-foreground">Files</span>
        <Button variant="quiet" size="icon-sm" onClick={() => setPaletteOpen(true)} aria-label="Go to file" tooltip="Go to file">
          <FileSearch size={13} aria-hidden="true" />
        </Button>
        <Button variant="quiet" size="icon-sm" loading={loadingTree} onClick={() => void loadTree()} aria-label="Refresh file list" tooltip="Refresh">
          <RefreshCw size={12} aria-hidden="true" />
        </Button>
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto p-1">
        {treeError
          ? <p className="px-2 py-3 text-meta leading-relaxed text-destructive">{treeError}</p>
          : rows.map(row => <TreeRow
            key={`${row.node.path}:${row.node.children ? "d" : "f"}`}
            row={row}
            active={row.node.path === activePath}
            onToggle={toggleDir}
            onOpen={path => void openFile(path)}
          />)}
        {rows.length >= MAX_TREE_ROWS && <p className="px-2 py-2 text-caption text-muted-foreground">Showing the first {MAX_TREE_ROWS} rows — use {modKey("P")} to reach the rest.</p>}
      </div>
    </aside>

    <div className="flex min-w-0 flex-1 flex-col">
      <div className="flex min-h-10 shrink-0 items-stretch border-b border-border bg-muted/20">
        <Button
          type="button"
          variant="ghost"
          size="icon-sm"
          className="my-auto ml-1 shrink-0 @min-[640px]/code:hidden"
          onClick={() => setTreeOpen(value => !value)}
          aria-label="Show file list"
          aria-expanded={treeOpen}
          tooltip="Files"
        ><PanelLeft size={14} aria-hidden="true" /></Button>
        <div className="flex min-w-0 flex-1 items-stretch overflow-x-auto">
          {open.map(file => {
            const name = file.path.slice(file.path.lastIndexOf("/") + 1);
            const marked = isDirty(file);
            const { Icon, tint } = glyphFor(file.path);
            return <div
              key={file.path}
              className={cn(
                "group/tab relative flex shrink-0 items-center gap-1.5 border-r border-border pl-3 pr-1.5 font-mono text-meta transition-colors",
                file.path === activePath ? "bg-code text-foreground" : "text-muted-foreground hover:bg-accent/50",
              )}
            >
              {/* The active tab is joined to the editor below it by a top
                  rule, so the tab strip reads as a set of folders over one
                  surface rather than a row of equal-weight buttons. */}
              {file.path === activePath && <span className="pointer-events-none absolute inset-x-0 top-0 h-[2px] bg-foreground/70" aria-hidden="true" />}
              <Tip label={file.path}><RowButton size="bare" tone="plain" onClick={() => setActivePath(file.path)} className="max-w-[180px]">
                <span className="flex min-h-8 min-w-0 items-center gap-1.5 py-1 text-inherit">
                  <Icon size={12} strokeWidth={1.75} className={cn("shrink-0", tint)} aria-hidden="true" />
                  <span className="truncate">{name}</span>
                </span>
              </RowButton></Tip>
              <Button
                variant="quiet"
                size="icon-2xs"
                onClick={() => closeFile(file.path)}
                aria-label={marked ? `Close ${file.path}, discarding unsaved changes` : `Close ${file.path}`}
                tooltip={marked ? "Close and discard unsaved changes" : "Close"}
              >
                {/* The dot is the unsaved marker; it becomes the close affordance on hover. */}
                {marked
                  ? <>
                    <span className={cn("h-1.5 w-1.5 rounded-full group-hover/tab:hidden group-focus-within/tab:hidden", file.state === "conflict" ? "bg-warning" : file.state === "error" ? "bg-destructive" : "bg-foreground/70")} aria-hidden="true" />
                    <X size={11} className="hidden group-hover/tab:block group-focus-within/tab:block" aria-hidden="true" />
                  </>
                  : <X size={11} className="text-muted-foreground" aria-hidden="true" />}
              </Button>
            </div>;
          })}
        </div>
        <Button variant="ghost" size="icon-sm" className="my-auto mr-1.5 shrink-0" onClick={() => setPaletteOpen(true)} aria-label="Find a file" tooltip="Go to file" tooltipSide="bottom"><FileSearch size={15} aria-hidden="true" /></Button>
      </div>

      <div className="relative flex min-h-0 flex-1 flex-col bg-code">
        {!active && <PaneState
          icon={FileSearch}
          title="Pick a file to start editing."
          action={<div className="flex flex-wrap justify-center gap-2">
            <Button variant="outline" size="sm" className="@min-[640px]/code:hidden" onClick={() => setTreeOpen(true)}>Browse files</Button>
            <Button variant="outline" size="sm" onClick={() => setPaletteOpen(true)}>Go to file <Kbd>{modKey("P")}</Kbd></Button>
          </div>}
        >Browse the project or search for a file. Use <Kbd>{modKey("S")}</Kbd> to save your changes.</PaneState>}
        {active && active.state === "error" && !active.baseSha && <p className="p-6 text-meta leading-relaxed text-destructive">{active.message}</p>}
        {active && active.tooLarge && <p className="p-6 text-meta leading-relaxed text-muted-foreground">{active.path} is {Math.round(active.sizeBytes / 1024)} KB — too large to open in the editor.</p>}
        {active && !active.tooLarge && active.binary && <p className="p-6 text-meta leading-relaxed text-muted-foreground">{active.path} is binary, so there is nothing to edit here.</p>}
        {/* Every open text file keeps its own editor, hidden when its tab is
            not showing. One editor re-seeded per tab switch threw away the
            caret, scroll, and undo history, and seeded from the last saved
            text, so unsaved edits vanished from view while ⌘S still wrote
            them. */}
        {open.filter(isEditable).map(file => <div key={`${file.path}:${file.seed}`} className={cn("min-h-0 flex-1", file.path !== activePath && "hidden")}>
          <CodeEditor
            docKey={`${file.path}:${file.seed}`}
            doc={file.saved}
            path={file.path}
            syncDoc={file.sync}
            revealLine={file.path === activePath && reveal && reveal.path === file.path && reveal.line !== undefined ? { line: reveal.line, nonce: reveal.nonce } : undefined}
            onChange={value => handleChange(file.path, value)}
            onSave={() => void save(file.path)}
            visible={visible && file.path === activePath}
            className="h-full cm-roomy"
          />
        </div>)}
      </div>

      {active && <div className="flex min-h-8 shrink-0 flex-wrap items-center gap-2 border-t border-border px-3 font-mono text-caption text-muted-foreground">
        <span className="truncate">{active.path}</span>
        <span className="ml-auto flex shrink-0 items-center gap-2">
          {active.state === "saving" && <SkeletonDot className="size-[11px]" />}
          <span className={cn(
            active.state === "conflict" && "text-warning",
            active.state === "error" && "text-destructive",
            active.state === "dirty" && "text-foreground",
          )}>{statusLabel(active)}</span>
          {active.state === "conflict" && <>
            <Button variant="link" size="inline" onClick={() => void reload(active.path)}>Reload</Button>
            <Button variant="link" size="inline" onClick={() => void save(active.path, true)}>Overwrite</Button>
          </>}
          {(active.state === "dirty" || (active.state === "error" && active.baseSha)) && <Button variant="link" size="inline" onClick={() => void save(active.path)}>Save {modKey("S")}</Button>}
          {active.state === "error" && !active.baseSha && <Button variant="link" size="inline" onClick={() => void reload(active.path)}>Retry</Button>}
        </span>
      </div>}
    </div>

    {paletteOpen && <FilePalette paths={paths} onPick={path => void openFile(path)} onClose={() => setPaletteOpen(false)} />}
    {dirtyCount > 0 && <span className="sr-only" aria-live="polite">{dirtyCount} file{dirtyCount === 1 ? "" : "s"} with unsaved changes</span>}
  </div>;
}
