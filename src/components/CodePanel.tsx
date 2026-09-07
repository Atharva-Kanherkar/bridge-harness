import { memo, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { ChevronRight, CornerDownLeft, FileSearch, LoaderCircle, RefreshCw, X } from "lucide-react";
import { bridgeApi } from "../api";
import { errorMessage } from "../errors";
import { ancestorPaths, buildFileTree, collapseChains, rankPaths, type TreeNode } from "../fileTree";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { Kbd } from "@/components/ui/kbd";
import { Dialog, DialogPopup } from "./ui/dialog";
import { PaneState } from "./ui/pane";
import { CodeEditor } from "./editor/CodeEditor";
import { isDirty, isReadOnly, loadBuffer, saveBuffer, stateAfterEdit, statusLabel, type FileBuffer } from "./editor/fileBuffer";
import { STATS_REFRESH_DEBOUNCE_MS } from "./ChangesPanel";

/** Rows the tree actually paints. Deep repositories stay responsive because
 *  only expanded directories contribute. */
const MAX_TREE_ROWS = 1500;

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

const TreeRow = memo(function TreeRow({ row, active, onToggle, onOpen }: {
  row: Row;
  active: boolean;
  onToggle: (path: string) => void;
  onOpen: (path: string) => void;
}) {
  const isDir = Boolean(row.node.children);
  return <button
    type="button"
    onClick={() => (isDir ? onToggle(row.node.path) : onOpen(row.node.path))}
    aria-expanded={isDir ? row.expanded : undefined}
    className={cn(
      "flex h-7 w-full items-center gap-1 truncate rounded-[5px] pr-2 text-left font-mono text-[12px] transition-colors",
      active ? "bg-accent text-foreground" : "text-muted-foreground hover:bg-accent/60 hover:text-foreground",
    )}
    style={{ paddingLeft: `${6 + row.depth * 11}px` }}
  >
    {isDir
      ? <ChevronRight size={11} className={cn("shrink-0 text-muted-foreground transition-transform", row.expanded && "rotate-90")} aria-hidden="true" />
      : <span className="w-[11px] shrink-0" aria-hidden="true" />}
    <span className="truncate">{row.node.name}</span>
  </button>;
});

/** ⌘P. Ranked over the same path list the tree is built from. */
function FilePalette({ paths, onPick, onClose }: { paths: string[]; onPick: (path: string) => void; onClose: () => void }) {
  const [query, setQuery] = useState("");
  const [index, setIndex] = useState(0);
  const results = useMemo(() => rankPaths(paths, query), [paths, query]);
  const active = results[Math.min(index, results.length - 1)];
  useEffect(() => setIndex(0), [query]);

  return <Dialog open onOpenChange={open => { if (!open) onClose(); }}>
    <DialogPopup aria-label="Open file" showCloseButton={false} className="max-h-[70dvh] max-w-xl overflow-hidden p-0">
      <div className="flex items-center gap-2 border-b border-border px-3">
        <FileSearch size={14} className="shrink-0 text-muted-foreground" aria-hidden="true" />
        <input
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
          className="h-11 flex-1 bg-transparent font-mono text-[12.5px] text-foreground outline-none placeholder:text-muted-foreground"
        />
        <Kbd>esc</Kbd>
      </div>
      <div className="max-h-[46vh] overflow-y-auto p-1">
        {results.length === 0
          ? <p className="px-2 py-6 text-center text-[12px] text-muted-foreground">No file matches “{query}”.</p>
          : results.map((path, position) => {
            const cut = path.lastIndexOf("/") + 1;
            return <button
              key={path}
              type="button"
              onMouseEnter={() => setIndex(position)}
              onClick={() => onPick(path)}
              className={cn(
                "flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left font-mono text-[12px]",
                path === active ? "bg-accent text-foreground" : "text-muted-foreground",
              )}
            >
              <span className="truncate">
                <span className="text-foreground">{path.slice(cut)}</span>
                {cut > 0 && <span className="ml-2 text-muted-foreground">{path.slice(0, cut - 1)}</span>}
              </span>
              {path === active && <CornerDownLeft size={11} className="ml-auto shrink-0 text-muted-foreground" aria-hidden="true" />}
            </button>;
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
  const [open, setOpen] = useState<FileBuffer[]>([]);
  const [activePath, setActivePath] = useState<string>();
  const [paletteOpen, setPaletteOpen] = useState(false);
  // Live buffers live outside React: a keystroke must not re-render the tree.
  const buffers = useRef(new Map<string, string>());
  // One counter per path, bumped on close. A load that finishes after its tab
  // was closed — or after a second load for the same path started — is stale,
  // and applying it would resurrect the tab or reset what you just typed.
  const opens = useRef(new Map<string, number>());
  const loading = useRef(new Set<string>());

  const loadTree = useCallback(async () => {
    setLoadingTree(true);
    try {
      setPaths(await bridgeApi.listWorkspaceTree(workspaceId));
      setTreeError(undefined);
    } catch (error) {
      setTreeError(errorMessage(error));
    } finally {
      setLoadingTree(false);
    }
  }, [workspaceId]);

  useEffect(() => { void loadTree(); }, [loadTree]);

  const tree = useMemo(() => collapseChains(buildFileTree(paths)), [paths]);
  const rows = useMemo(() => flatten(tree, expanded), [tree, expanded]);
  const active = open.find(file => file.path === activePath);

  const openFile = useCallback(async (path: string) => {
    setPaletteOpen(false);
    setActivePath(path);
    // Reveal it in the tree, so the palette and the tree never disagree.
    setExpanded(previous => {
      const next = new Set(previous);
      for (const ancestor of ancestorPaths(path)) next.add(ancestor);
      return next;
    });
    // `open` is a closed-over snapshot, so two quick opens of one path can both
    // pass this guard; `loading` is the ref that actually serialises them.
    if (open.some(file => file.path === path) || loading.current.has(path)) return;
    loading.current.add(path);
    const generation = opens.current.get(path) ?? 0;
    try {
      const buffer = await loadBuffer(workspaceId, path);
      if ((opens.current.get(path) ?? 0) !== generation) return;
      buffers.current.set(path, buffer.saved);
      setOpen(files => files.some(entry => entry.path === path) ? files : [...files, buffer]);
    } finally {
      loading.current.delete(path);
    }
  }, [open, workspaceId]);

  // One open per reveal nonce. The ref carries the last honoured request so a
  // re-render with the same reveal object is inert.
  const revealSeen = useRef(0);
  useEffect(() => {
    if (!reveal || reveal.nonce === revealSeen.current) return;
    revealSeen.current = reveal.nonce;
    void openFile(reveal.path);
  }, [reveal, openFile]);

  const closeFile = useCallback((path: string) => {
    // Retire any load still in flight for this path along with the tab.
    opens.current.set(path, (opens.current.get(path) ?? 0) + 1);
    buffers.current.delete(path);
    setOpen(files => {
      const remaining = files.filter(file => file.path !== path);
      setActivePath(current => current === path ? remaining[remaining.length - 1]?.path : current);
      return remaining;
    });
  }, []);

  /** Typing only re-renders when the *dirty flag* flips, not on every key. */
  const handleChange = useCallback((path: string, value: string) => {
    buffers.current.set(path, value);
    setOpen(files => {
      const file = files.find(entry => entry.path === path);
      const next = file && stateAfterEdit(file, value);
      // Returning the same array is what stops a keystroke re-rendering the
      // tree and the tab strip.
      if (!next) return files;
      return files.map(entry => entry.path === path ? { ...entry, state: next } : entry);
    });
  }, []);

  const patch = useCallback((path: string, change: Partial<FileBuffer>) => {
    setOpen(files => files.map(file => file.path === path ? { ...file, ...change } : file));
  }, []);

  const save = useCallback(async (path: string, force = false) => {
    const file = open.find(entry => entry.path === path);
    if (!file || isReadOnly(file) || file.state === "saving") return;
    const content = buffers.current.get(path) ?? file.saved;
    patch(path, { state: "saving", message: undefined });
    const next = await saveBuffer(workspaceId, file, content, force);
    setOpen(files => files.map(entry => entry.path === path ? next : entry));
    if (next.state === "clean") onSaved?.();
  }, [open, onSaved, patch, workspaceId]);

  /** Discard the local buffer and take what is on disk now. */
  const reload = useCallback(async (path: string) => {
    const seed = (open.find(entry => entry.path === path)?.seed ?? 0) + 1;
    const generation = opens.current.get(path) ?? 0;
    const buffer = await loadBuffer(workspaceId, path, seed);
    if ((opens.current.get(path) ?? 0) !== generation) return;
    buffers.current.set(path, buffer.saved);
    setOpen(files => files.map(entry => entry.path === path ? buffer : entry));
  }, [open, workspaceId]);

  // The agent writes the same tree this panel edits. When the workspace's
  // stats settle after a drift, compare every open buffer against disk: a
  // clean buffer takes the new bytes, a dirty one turns its existing conflict
  // state on — the same state a refused save produces — and keeps the unsaved
  // text. Debounced like the Changes pane, because stats tick many times a
  // second under a writing agent, and each probe is a blocking IPC call.
  const driftSeen = useRef(driftSignal);
  useEffect(() => {
    if (driftSignal === undefined || driftSignal === driftSeen.current) return;
    driftSeen.current = driftSignal;
    if (open.length === 0) return;
    let live = true;
    const timer = window.setTimeout(() => void (async () => {
      for (const file of open) {
        if (!live) break;
        if (file.state === "saving" || file.state === "error") continue;
        try {
          // Captured before the read: a tab closed (or closed and reopened)
          // while the probe was in flight must not be resurrected by it.
          const generation = opens.current.get(file.path) ?? 0;
          const disk = await bridgeApi.readWorkspaceFile(workspaceId, file.path);
          if (!live) break;
          if ((opens.current.get(file.path) ?? 0) !== generation) continue;
          if (disk.sha256 === file.baseSha) continue;
          if (isDirty(file)) {
            if (file.state !== "conflict") patch(file.path, { state: "conflict", message: "Changed on disk while you were editing" });
          } else {
            // The probe already carried the bytes; reseed from them instead of
            // reading the same file a second time.
            const seed = (open.find(entry => entry.path === file.path)?.seed ?? 0) + 1;
            buffers.current.set(file.path, disk.content);
            setOpen(files => files.map(entry => entry.path === file.path ? {
              path: file.path, baseSha: disk.sha256, saved: disk.content, binary: disk.binary,
              tooLarge: disk.tooLarge, sizeBytes: disk.sizeBytes, seed, state: "clean",
            } : entry));
          }
        } catch {
          // A vanished file surfaces on the next save; drift polling stays quiet.
        }
      }
    })(), STATS_REFRESH_DEBOUNCE_MS);
    return () => { live = false; window.clearTimeout(timer); };
  }, [driftSignal, open, workspaceId, patch]);


  // ⌘P and ⌘S also work when focus is in the tree or the tab strip; the
  // editor has its own ⌘S so a save never depends on where the caret is.
  useEffect(() => {
    if (!visible) return;
    const onKeyDown = (event: KeyboardEvent) => {
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
    <aside className="hidden w-52 shrink-0 flex-col border-r border-border bg-muted/20 @min-[640px]/code:flex">
      <div className="flex h-10 shrink-0 items-center gap-1 border-b border-border pl-3 pr-1.5">
        <span className="flex-1 truncate text-[12px] font-medium text-muted-foreground">Files</span>
        <Button type="button" variant="ghost" size="icon-sm" className="text-muted-foreground" onClick={() => setPaletteOpen(true)} aria-label="Go to file">
          <FileSearch size={13} aria-hidden="true" />
        </Button>
        <Button type="button" variant="ghost" size="icon-sm" className="text-muted-foreground" onClick={() => void loadTree()} aria-label="Refresh file list">
          <RefreshCw size={12} className={cn(loadingTree && "animate-spin")} aria-hidden="true" />
        </Button>
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto p-1">
        {treeError
          ? <p className="px-2 py-3 text-[12px] leading-relaxed text-destructive">{treeError}</p>
          : rows.map(row => <TreeRow
            key={`${row.node.path}:${row.node.children ? "d" : "f"}`}
            row={row}
            active={row.node.path === activePath}
            onToggle={toggleDir}
            onOpen={path => void openFile(path)}
          />)}
        {rows.length >= MAX_TREE_ROWS && <p className="px-2 py-2 text-[11px] text-muted-foreground">Showing the first {MAX_TREE_ROWS} rows — use ⌘P to reach the rest.</p>}
      </div>
    </aside>

    <div className="flex min-w-0 flex-1 flex-col">
      <div className="flex min-h-10 shrink-0 items-stretch border-b border-border bg-muted/20">
        <div className="flex min-w-0 flex-1 items-stretch overflow-x-auto">
          {open.map(file => {
            const name = file.path.slice(file.path.lastIndexOf("/") + 1);
            const marked = isDirty(file);
            return <div
              key={file.path}
              className={cn(
                "group/tab flex shrink-0 items-center gap-1.5 border-r border-border pl-3 pr-1.5 font-mono text-[12px] transition-colors",
                file.path === activePath ? "bg-code text-foreground" : "text-muted-foreground hover:bg-accent/50",
              )}
            >
              <button type="button" onClick={() => setActivePath(file.path)} className="min-h-8 max-w-[180px] truncate py-1" title={file.path}>{name}</button>
              <button
                type="button"
                onClick={() => closeFile(file.path)}
                aria-label={marked ? `Close ${file.path}, discarding unsaved changes` : `Close ${file.path}`}
                title={marked ? "Close and discard unsaved changes" : "Close"}
                className="grid h-6 w-6 shrink-0 place-items-center rounded-[3px] text-muted-foreground hover:bg-accent hover:text-foreground"
              >
                {/* The dot is the unsaved marker; it becomes the close affordance on hover. */}
                {marked
                  ? <>
                    <span className={cn("h-1.5 w-1.5 rounded-full group-hover/tab:hidden group-focus-within/tab:hidden", file.state === "conflict" ? "bg-warning" : "bg-foreground/70")} aria-hidden="true" />
                    <X size={11} className="hidden group-hover/tab:block group-focus-within/tab:block" aria-hidden="true" />
                  </>
                  : <X size={11} className="text-muted-foreground" aria-hidden="true" />}
              </button>
            </div>;
          })}
        </div>
        <Button type="button" variant="ghost" size="icon-sm" className="my-auto mr-1.5 shrink-0" onClick={() => setPaletteOpen(true)} aria-label="Find a file" title="Go to file · ⌘P"><FileSearch size={15} aria-hidden="true" /></Button>
      </div>

      <div className="relative flex min-h-0 flex-1 flex-col bg-code">
        {!active
          ? <PaneState icon={FileSearch} title="Pick a file to start editing." action={<Button variant="outline" size="sm" onClick={() => setPaletteOpen(true)}>Go to file <Kbd>⌘P</Kbd></Button>}>Browse the project or search for a file. Use <Kbd>⌘S</Kbd> to save your changes.</PaneState>
          : active.state === "error" && !active.baseSha
            ? <p className="p-6 text-[12.5px] leading-relaxed text-destructive">{active.message}</p>
            : active.tooLarge
              ? <p className="p-6 text-[12.5px] leading-relaxed text-muted-foreground">{active.path} is {Math.round(active.sizeBytes / 1024)} KB — too large to open in the editor.</p>
              : active.binary
                ? <p className="p-6 text-[12.5px] leading-relaxed text-muted-foreground">{active.path} is binary, so there is nothing to edit here.</p>
                : <CodeEditor
                  key={`${active.path}:${active.seed}`}
                  docKey={`${active.path}:${active.seed}`}
                  doc={active.saved}
                  path={active.path}
                  revealLine={reveal && reveal.path === active.path && reveal.line !== undefined ? { line: reveal.line, nonce: reveal.nonce } : undefined}
                  onChange={value => handleChange(active.path, value)}
                  onSave={() => void save(active.path)}
                  visible={visible}
                  className="h-full cm-roomy"
                />}
      </div>

      {active && <div className="flex min-h-8 shrink-0 flex-wrap items-center gap-2 border-t border-border px-3 font-mono text-[11px] text-muted-foreground">
        <span className="truncate">{active.path}</span>
        <span className="ml-auto flex shrink-0 items-center gap-2">
          {active.state === "saving" && <LoaderCircle size={11} className="animate-spin" aria-hidden="true" />}
          <span className={cn(
            active.state === "conflict" && "text-warning",
            active.state === "error" && "text-destructive",
            active.state === "dirty" && "text-foreground",
          )}>{statusLabel(active)}</span>
          {active.state === "conflict" && <>
            <button type="button" onClick={() => void reload(active.path)} className="underline decoration-dotted underline-offset-2 hover:text-foreground">Reload</button>
            <button type="button" onClick={() => void save(active.path, true)} className="underline decoration-dotted underline-offset-2 hover:text-foreground">Overwrite</button>
          </>}
          {(active.state === "dirty" || active.state === "error") && <button type="button" onClick={() => void save(active.path)} className="underline decoration-dotted underline-offset-2 hover:text-foreground">Save ⌘S</button>}
        </span>
      </div>}
    </div>

    {paletteOpen && <FilePalette paths={paths} onPick={path => void openFile(path)} onClose={() => setPaletteOpen(false)} />}
    {dirtyCount > 0 && <span className="sr-only" aria-live="polite">{dirtyCount} file{dirtyCount === 1 ? "" : "s"} with unsaved changes</span>}
  </div>;
}
