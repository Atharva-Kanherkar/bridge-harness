import { useCallback, useEffect, useRef, useState, type DragEvent, type ReactNode } from "react";
import { ArrowDownToLine, Bot, ChevronDown, Columns2, Keyboard, Maximize2, Minimize2, Plus, RotateCcw, Rows2, Search, TerminalSquare, X } from "lucide-react";
import { bridgeApi } from "../api";
import { cn } from "../lib/utils";
import { addTab, closeLeaf, emptyLayout, insertSplit, leafIds, resizeNode, restoreLayout, type PaneNode, type SplitDirection, type TerminalLayout } from "../terminal/layout";
import type { TerminalRecord } from "../terminal/types";
import { TERMINAL_SHORTCUTS, terminalChord, terminalCommand, type TerminalCommand } from "../terminal/shortcuts";
import { TerminalSurface } from "./TerminalSurface";

const PANE_DRAG = "application/x-bridge-terminal-pane";
const TAB_DRAG = "application/x-bridge-terminal-tab";
const AGENTS = [{ id: "codex", label: "Codex" }, { id: "claude", label: "Claude Code" }, { id: "opencode", label: "OpenCode" }, { id: "cursor", label: "Cursor" }, { id: "grok", label: "Grok" }];
const writes = new Map<string, Promise<void>>();
function saveLayout(workspace: string, layout: TerminalLayout) {
  const next = (writes.get(workspace) ?? Promise.resolve()).catch(() => {}).then(() => bridgeApi.saveTerminalLayout(workspace, layout));
  writes.set(workspace, next);
  void next.finally(() => { if (writes.get(workspace) === next) writes.delete(workspace); }).catch(() => {});
  return next;
}
type WorkspaceState = { layout: TerminalLayout; records: Record<string, TerminalRecord> };
type PaneActions = {
  workspaceId: string;
  branch: string;
  layout: TerminalLayout;
  records: Record<string, TerminalRecord>;
  focus: (id: string) => void;
  split: (id: string, direction: SplitDirection) => void;
  close: (id: string) => void;
  restart: (id: string) => void;
  rename: (id: string, title: string) => void;
  maximize: (id: string) => void;
  move: (id: string, target: string, direction: SplitDirection, before: boolean) => void;
  resize: (path: string, ratio: number) => void;
  onRecord: (record: TerminalRecord) => void;
  searchRequest: { id: string; serial: number } | null;
  searchHandled: () => void;
};

function IconButton({ title, onClick, children }: { title: string; onClick: () => void; children: ReactNode }) {
  return <button type="button" title={title} aria-label={title} onClick={onClick} className="rounded p-1 text-muted-foreground transition-colors hover:bg-accent hover:text-foreground focus-visible:outline-2 focus-visible:outline-ring">{children}</button>;
}

function Pane({ id, actions }: { id: string; actions: PaneActions }) {
  const record = actions.records[id];
  const [renaming, setRenaming] = useState(false);
  const [title, setTitle] = useState("");
  const [drop, setDrop] = useState<"left" | "right" | "top" | "bottom">();
  if (!record) return null;
  const focused = actions.layout.activeLeafId === id;
  const expanded = actions.layout.expandedLeafId === id;
  function dropSide(event: DragEvent) {
    const rect = event.currentTarget.getBoundingClientRect();
    const x = (event.clientX - rect.left) / rect.width, y = (event.clientY - rect.top) / rect.height;
    const nearX = Math.min(x, 1 - x), nearY = Math.min(y, 1 - y);
    return nearX < nearY ? (x < .5 ? "left" : "right") : (y < .5 ? "top" : "bottom");
  }
  return <section aria-label={`Pane ${record.title}`} onPointerDownCapture={() => actions.focus(id)} onFocusCapture={() => actions.focus(id)}
    onDragOver={event => { if (event.dataTransfer.types.includes(PANE_DRAG)) { event.preventDefault(); event.dataTransfer.dropEffect = "move"; setDrop(dropSide(event)); } }}
    onDragLeave={event => { if (!event.currentTarget.contains(event.relatedTarget as Node | null)) setDrop(undefined); }}
    onDrop={event => {
      setDrop(undefined);
      try {
        const payload = JSON.parse(event.dataTransfer.getData(PANE_DRAG));
        if (payload.workspaceId !== actions.workspaceId || !actions.records[payload.id]) return;
        event.preventDefault(); const edge = dropSide(event);
        actions.move(payload.id, id, edge === "left" || edge === "right" ? "horizontal" : "vertical", edge === "left" || edge === "top");
      } catch { /* Ignore external drags. */ }
    }} className={cn("relative flex h-full min-h-0 min-w-0 flex-col overflow-hidden rounded-md border bg-code", focused ? "border-ring/65" : "border-border")}>
    <div draggable={!renaming} onDragStart={event => { event.dataTransfer.setData(PANE_DRAG, JSON.stringify({ workspaceId: actions.workspaceId, id })); event.dataTransfer.effectAllowed = "move"; }} className={cn("flex h-9 shrink-0 items-center gap-1.5 border-b border-border px-2", focused ? "bg-accent" : "bg-background")}>
      {record.agentId ? <Bot size={13} className="shrink-0 text-muted-foreground" aria-hidden="true" /> : <TerminalSquare size={13} className="shrink-0 text-muted-foreground" aria-hidden="true" />}
      {renaming ? <input autoFocus aria-label="Terminal title" maxLength={120} className="min-w-0 flex-1 rounded bg-background px-1 text-xs outline-none ring-1 ring-ring" value={title} onChange={event => setTitle(event.target.value)} onBlur={() => { if (title.trim()) actions.rename(id, title.trim()); setRenaming(false); }} onKeyDown={event => { if (event.key === "Enter") event.currentTarget.blur(); if (event.key === "Escape") { setRenaming(false); } event.stopPropagation(); }} /> : <button type="button" title="Rename terminal · drag to move pane" aria-label={`Rename ${record.title}`} className="min-w-0 flex-1 cursor-grab truncate text-left text-xs font-medium" onClick={() => { setTitle(record.title); setRenaming(true); }}>{record.title}</button>}
      <span title={record.status === "running" ? "Process running" : "Process ended"} className={cn("h-1.5 w-1.5 shrink-0 rounded-full", record.status === "running" ? "bg-success" : "bg-muted-foreground")} />
      <IconButton title={`Split right (${terminalChord("split-right")})`} onClick={() => actions.split(id, "horizontal")}><Columns2 size={13} /></IconButton>
      <IconButton title={`Split down (${terminalChord("split-down")})`} onClick={() => actions.split(id, "vertical")}><Rows2 size={13} /></IconButton>
      <IconButton title={expanded ? "Restore panes" : `Maximize pane (${terminalChord("maximize")})`} onClick={() => actions.maximize(id)}>{expanded ? <Minimize2 size={13} /> : <Maximize2 size={13} />}</IconButton>
      <IconButton title={`Close ${record.title}`} onClick={() => actions.close(id)}><X size={13} /></IconButton>
    </div>
    <TerminalSurface record={record} focused={focused} onRecord={actions.onRecord} onSearchHandled={actions.searchHandled} searchRequest={actions.searchRequest?.id === id ? actions.searchRequest.serial : 0} />
    {record.status !== "running" && <div className="flex shrink-0 items-center gap-2 border-t border-border bg-background px-2 py-1.5 text-[11px] text-muted-foreground"><span className="flex-1">Process ended{record.exitCode != null ? ` · exit ${record.exitCode}` : ""}. History restored.</span><button type="button" onClick={() => actions.restart(id)} className="inline-flex items-center gap-1 rounded px-2 py-1 text-foreground hover:bg-accent"><RotateCcw size={11} />Restart</button></div>}
    <div className="flex h-6 shrink-0 items-center gap-2 border-t border-border bg-background px-2 font-mono text-[10px] text-muted-foreground"><span title={record.cwd} className="min-w-0 flex-1 truncate">{record.cwd}</span><span className="max-w-28 truncate">{actions.branch}</span>{record.historyTruncated && <span title="Older history was trimmed to the storage budget">History trimmed</span>}</div>
    {drop && <div aria-hidden="true" className={cn("pointer-events-none absolute z-10 rounded border-2 border-ring bg-selection/40", drop === "left" && "inset-y-0 left-0 w-1/2", drop === "right" && "inset-y-0 right-0 w-1/2", drop === "top" && "inset-x-0 top-0 h-1/2", drop === "bottom" && "inset-x-0 bottom-0 h-1/2")} />}
  </section>;
}

function SplitTree({ node, path = "", actions }: { node: PaneNode; path?: string; actions: PaneActions }) {
  const container = useRef<HTMLDivElement>(null);
  if (node.type === "leaf") return <Pane id={node.leafId} actions={actions} />;
  const horizontal = node.direction === "horizontal";
  const firstMin = minimumSize(node.first), secondMin = minimumSize(node.second);
  const position = (x: number, y: number) => {
    const rect = container.current?.getBoundingClientRect();
    if (rect) actions.resize(path, horizontal ? (x - rect.left) / rect.width : (y - rect.top) / rect.height);
  };
  return <div ref={container} className="grid h-full min-h-0 min-w-0" style={horizontal ? { gridTemplateColumns: `minmax(${firstMin.width}px, ${node.ratio}fr) 6px minmax(${secondMin.width}px, ${1 - node.ratio}fr)` } : { gridTemplateRows: `minmax(${firstMin.height}px, ${node.ratio}fr) 6px minmax(${secondMin.height}px, ${1 - node.ratio}fr)` }}>
    <SplitTree node={node.first} path={`${path}0`} actions={actions} />
    <div role="separator" tabIndex={0} aria-label="Resize terminal split" aria-orientation={horizontal ? "vertical" : "horizontal"} aria-valuemin={10} aria-valuemax={90} aria-valuenow={Math.round(node.ratio * 100)}
      onPointerDown={event => { event.preventDefault(); event.currentTarget.setPointerCapture(event.pointerId); }}
      onPointerMove={event => { if (event.currentTarget.hasPointerCapture(event.pointerId)) position(event.clientX, event.clientY); }}
      onPointerUp={event => { position(event.clientX, event.clientY); event.currentTarget.releasePointerCapture(event.pointerId); }}
      onKeyDown={event => {
        if (!["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown", "Home"].includes(event.key)) return;
        event.preventDefault(); actions.resize(path, event.key === "Home" ? .5 : node.ratio + (["ArrowLeft", "ArrowUp"].includes(event.key) ? -.05 : .05));
      }} className={cn("rounded transition-colors hover:bg-ring/30 focus-visible:bg-ring/40 focus-visible:outline-none", horizontal ? "cursor-col-resize" : "cursor-row-resize")} />
    <SplitTree node={node.second} path={`${path}1`} actions={actions} />
  </div>;
}

function minimumSize(node: PaneNode): { width: number; height: number } {
  if (node.type === "leaf") return { width: 240, height: 170 };
  const a = minimumSize(node.first), b = minimumSize(node.second);
  return node.direction === "horizontal" ? { width: a.width + b.width + 6, height: Math.max(a.height, b.height) } : { width: Math.max(a.width, b.width), height: a.height + b.height + 6 };
}

function releaseTerminalFocus() {
  const active = document.activeElement;
  // Move focus before React removes a pane's textarea, so browser focus and
  // accessibility bookkeeping never retain a detached terminal input.
  if (active instanceof HTMLElement && active.closest(".xterm")?.closest("[data-terminal-workspace]")) active.blur();
}

export function TerminalWorkspace({ workspaceId, branch }: { workspaceId: string; branch: string }) {
  const [state, setState] = useState<WorkspaceState | null>(null);
  const [error, setError] = useState<string>();
  const [retrySave, setRetrySave] = useState(false);
  const [busy, setBusy] = useState(false);
  const [agentMenu, setAgentMenu] = useState(false);
  const [shortcuts, setShortcuts] = useState(false);
  const [searchRequest, setSearchRequest] = useState<{ id: string; serial: number } | null>(null);
  const current = useRef(state); current.current = state;
  const alive = useRef(true);
  const dirty = useRef(false);
  const errorHandler = useCallback((value: unknown) => { if (alive.current) { setError(String(value)); setRetrySave(false); } }, []);
  const load = useCallback(async () => {
    try {
      await writes.get(workspaceId)?.catch(() => {});
      const data = await bridgeApi.terminalWorkspace(workspaceId);
      if (!alive.current) return;
      setState({ records: Object.fromEntries(data.terminals.map(t => [t.terminalId, t])), layout: restoreLayout(data.layout, data.terminals) });
      setError(undefined);
    } catch (value) { errorHandler(value); }
  }, [workspaceId, errorHandler]);
  useEffect(() => { alive.current = true; void load(); return () => { alive.current = false; }; }, [load]);
  useEffect(() => {
    if (!state || !dirty.current) return;
    const layout = state.layout;
    const timer = setTimeout(() => { dirty.current = false; void saveLayout(workspaceId, layout).catch(value => { errorHandler(value); if (alive.current) setRetrySave(true); }); }, 200);
    return () => clearTimeout(timer);
  }, [state?.layout, workspaceId, errorHandler]);
  useEffect(() => () => {
    if (dirty.current && current.current) void saveLayout(workspaceId, current.current.layout).catch(() => {});
  }, [workspaceId]);
  const change = (update: (layout: TerminalLayout) => TerminalLayout, movesPanes = false) => {
    if (movesPanes) releaseTerminalFocus();
    dirty.current = true;
    setState(s => s ? { ...s, layout: update(s.layout) } : s);
  };
  async function launch(agentId?: string, target?: string, direction?: SplitDirection) {
    if (busy) return;
    setBusy(true); setAgentMenu(false); setError(undefined);
    try {
      const source = target ? await bridgeApi.terminalSnapshot(workspaceId, target) : undefined;
      const record = await bridgeApi.createTerminal({ workspaceId, terminalId: crypto.randomUUID(), agentId, cwd: source?.record.cwd, restart: false });
      if (!alive.current) return;
      releaseTerminalFocus();
      dirty.current = true;
      setState(s => {
        const previous = s ?? { layout: emptyLayout(), records: {} };
        const exists = target && previous.layout.tabs.some(t => leafIds(t.root).includes(target));
        return { records: { ...previous.records, [record.terminalId]: record }, layout: exists && direction ? insertSplit(previous.layout, target, record.terminalId, direction) : addTab(previous.layout, record.terminalId, record.title) };
      });
    } catch (value) { errorHandler(value); }
    finally { if (alive.current) setBusy(false); }
  }
  async function close(id: string) {
    try {
      await bridgeApi.closeTerminal(workspaceId, id);
      if (!alive.current) return;
      releaseTerminalFocus();
      dirty.current = true;
      setState(s => { if (!s) return s; const records = { ...s.records }; delete records[id]; return { records, layout: closeLeaf(s.layout, id) }; });
    } catch (value) { errorHandler(value); }
  }
  const onRecord = useCallback((record: TerminalRecord) => {
    if (!alive.current) return;
    setState(s => s?.records[record.terminalId] ? { ...s, records: { ...s.records, [record.terminalId]: record } } : s);
  }, []);
  async function restart(id: string) {
    if (busy) return;
    setBusy(true);
    try { onRecord(await bridgeApi.createTerminal({ workspaceId, terminalId: id, restart: true })); }
    catch (value) { errorHandler(value); }
    finally { if (alive.current) setBusy(false); }
  }
  async function rename(id: string, title: string) {
    try {
      onRecord(await bridgeApi.renameTerminal(workspaceId, id, title));
      change(layout => ({ ...layout, tabs: layout.tabs.map(t => leafIds(t.root)[0] === id ? { ...t, title } : t) }));
    } catch (value) { errorHandler(value); }
  }
  function selectTab(id: string) {
    change(layout => { const tab = layout.tabs.find(t => t.id === id); return tab ? { ...layout, activeTabId: id, activeLeafId: leafIds(tab.root)[0], expandedLeafId: null } : layout; }, true);
  }
  function detachToTab(id: string) {
    if (state?.records[id]) change(layout => addTab(closeLeaf(layout, id), id, state.records[id].title), true);
  }
  const layout = state?.layout ?? emptyLayout();
  const tab = layout.tabs.find(t => t.id === layout.activeTabId);
  const actions: PaneActions = {
    workspaceId, branch, layout, records: state?.records ?? {}, searchRequest, onRecord,
    searchHandled: () => setSearchRequest(null),
    focus: id => { if (current.current?.layout.activeLeafId !== id) change(l => ({ ...l, activeLeafId: id })); },
    split: (id, direction) => { void launch(undefined, id, direction); },
    close: id => { void close(id); }, restart: id => { void restart(id); }, rename: (id, title) => { void rename(id, title); },
    maximize: id => change(l => ({ ...l, expandedLeafId: l.expandedLeafId === id ? null : id, activeLeafId: id }), true),
    move: (id, target, direction, before) => change(l => insertSplit(l, target, id, direction, before), true),
    resize: (path, ratio) => change(l => ({ ...l, tabs: l.tabs.map(t => t.id === l.activeTabId ? { ...t, root: resizeNode(t.root, path, ratio) } : t) })),
  };
  function command(id: TerminalCommand) {
    const active = layout.activeLeafId;
    if (id === "new-tab") { void launch(); return; }
    if (!active || !tab) return;
    if (id === "split-right" || id === "split-down") actions.split(active, id === "split-right" ? "horizontal" : "vertical");
    if (id === "close-pane") actions.close(active);
    if (id === "maximize") actions.maximize(active);
    if (id === "search") setSearchRequest(value => ({ id: active, serial: (value?.serial ?? 0) + 1 }));
    if (id === "next-pane" || id === "previous-pane") {
      const ids = leafIds(tab.root), next = ids[(ids.indexOf(active) + (id === "next-pane" ? 1 : ids.length - 1)) % ids.length];
      change(l => ({ ...l, activeLeafId: next, expandedLeafId: l.expandedLeafId ? next : null }), !!layout.expandedLeafId);
    }
    if (id === "next-tab" || id === "previous-tab") selectTab(layout.tabs[(layout.tabs.indexOf(tab) + (id === "next-tab" ? 1 : layout.tabs.length - 1)) % layout.tabs.length].id);
  }
  const root: PaneNode | undefined = layout.expandedLeafId ? { type: "leaf", leafId: layout.expandedLeafId } : tab?.root;
  const minimum = root ? minimumSize(root) : undefined;
  return <div data-terminal-workspace className="flex min-h-0 flex-1 flex-col" onKeyDownCapture={event => { const id = terminalCommand(event.nativeEvent); if (id) { event.preventDefault(); event.stopPropagation(); command(id); } }}>
    <div className="flex shrink-0 flex-wrap items-center gap-2 border-b border-border px-4 py-2">
      <button type="button" disabled={busy || !state} title={terminalChord("new-tab")} className="inline-flex items-center gap-1.5 rounded-md border border-border-card bg-card px-2.5 py-1.5 text-xs font-medium shadow-control hover:bg-accent disabled:opacity-50" onClick={() => { void launch(); }}><Plus size={13} />New Terminal</button>
      <div className="relative">
        <button type="button" disabled={busy || !state} aria-expanded={agentMenu} className="inline-flex items-center gap-1.5 rounded-md px-2 py-1.5 text-xs font-medium hover:bg-accent disabled:opacity-50" onClick={() => setAgentMenu(v => !v)}><Bot size={13} />New Agent<ChevronDown size={11} /></button>
        {agentMenu && <div className="u-glass-popover absolute left-0 top-full z-30 mt-1 w-48 rounded-lg border border-border p-1 shadow-lg" onKeyDown={event => { if (event.key === "Escape") setAgentMenu(false); }}>
          <p className="px-2 py-1.5 text-[10px] text-muted-foreground">Run an installed agent CLI</p>
          {AGENTS.map(agent => <button key={agent.id} type="button" className="flex w-full items-center rounded px-2 py-1.5 text-left text-xs hover:bg-accent" onClick={() => { void launch(agent.id); }}>{agent.label}</button>)}
        </div>}
      </div>
      {busy && <span role="status" className="text-xs text-muted-foreground">Opening terminal…</span>}
      <div className="ml-auto flex items-center gap-1">
        {layout.activeLeafId && <IconButton title="Move focused pane to a new tab" onClick={() => detachToTab(layout.activeLeafId!)}><ArrowDownToLine size={14} /></IconButton>}
        <IconButton title={`Search scrollback (${terminalChord("search")})`} onClick={() => command("search")}><Search size={14} /></IconButton>
        <IconButton title="Terminal shortcuts" onClick={() => setShortcuts(v => !v)}><Keyboard size={15} /></IconButton>
      </div>
    </div>
    {error && <div role="alert" className="flex shrink-0 items-center gap-2 border-b border-border px-4 py-2 text-xs text-destructive"><span className="flex-1">{error}</span>{(!state || retrySave) && <button type="button" className="underline" onClick={() => { if (state) void saveLayout(workspaceId, state.layout).then(() => setError(undefined)).catch(value => { errorHandler(value); if (alive.current) setRetrySave(true); }); else void load(); }}>Retry</button>}<button type="button" aria-label="Dismiss terminal error" onClick={() => setError(undefined)}><X size={13} /></button></div>}
    {shortcuts && <div className="grid shrink-0 grid-cols-2 gap-x-5 gap-y-1 border-b border-border bg-muted/30 px-4 py-3 text-[11px] sm:grid-cols-3">{TERMINAL_SHORTCUTS.map(s => <div key={s.id} className="flex justify-between gap-3"><span className="text-muted-foreground">{s.label}</span><kbd className="font-mono">{terminalChord(s.id)}</kbd></div>)}</div>}
    {layout.tabs.length > 0 && <div role="tablist" aria-label="Terminal tabs" className="flex shrink-0 gap-1 overflow-x-auto border-b border-border px-3 pt-2">
      {layout.tabs.map(t => <button type="button" role="tab" aria-selected={t.id === layout.activeTabId} key={t.id} draggable onDragStart={event => { event.dataTransfer.setData(TAB_DRAG, t.id); event.dataTransfer.effectAllowed = "move"; }}
        onDragOver={event => { if (event.dataTransfer.types.some(type => type === PANE_DRAG || type === TAB_DRAG)) event.preventDefault(); }}
        onDrop={event => {
          event.preventDefault(); const from = event.dataTransfer.getData(TAB_DRAG);
          if (from && from !== t.id) change(l => { const tabs = l.tabs.filter(tab => tab.id !== from), moving = l.tabs.find(tab => tab.id === from); if (moving) tabs.splice(tabs.findIndex(tab => tab.id === t.id), 0, moving); return { ...l, tabs }; });
          else try { const payload = JSON.parse(event.dataTransfer.getData(PANE_DRAG)); if (payload.workspaceId === workspaceId && state?.records[payload.id]) actions.move(payload.id, leafIds(t.root)[0], "horizontal", false); } catch { /* External drag. */ }
        }} onClick={() => selectTab(t.id)} className={cn("flex max-w-56 shrink-0 items-center gap-2 rounded-t-md border border-b-0 px-3 py-2 text-xs", t.id === layout.activeTabId ? "border-border bg-code text-foreground" : "border-transparent text-muted-foreground hover:bg-accent")}><TerminalSquare size={12} /><span className="truncate">{t.title}</span>{leafIds(t.root).length > 1 && <span className="text-[10px] text-muted-foreground">{leafIds(t.root).length}</span>}</button>)}
    </div>}
    <div className="min-h-0 flex-1 overflow-auto p-2">
      {!state ? <div role="status" className="grid h-full place-items-center text-xs text-muted-foreground">{error ? "Terminal workspace could not be opened." : "Restoring workspace…"}</div> : root ? <div role="tabpanel" className="h-full w-full" style={minimum ? { minWidth: minimum.width, minHeight: minimum.height } : undefined}><SplitTree node={root} actions={actions} /></div> : <div className="flex h-full min-h-60 flex-col items-center justify-center gap-3 px-8 text-center"><TerminalSquare size={30} strokeWidth={1.2} className="text-muted-foreground" /><h2 className="font-display text-xl">A terminal for every stream of work</h2><p className="max-w-sm text-sm leading-relaxed text-muted-foreground">Open a shell or an agent CLI, then split your workspace as you go. Your layout and terminal history are saved here.</p><button type="button" disabled={busy} onClick={() => { void launch(); }} className="mt-2 rounded-md border border-border-card bg-card px-4 py-2 text-xs font-medium shadow-control hover:bg-accent">Open a terminal</button></div>}
    </div>
  </div>;
}
