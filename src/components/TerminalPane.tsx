import { useCallback, useEffect, useRef, useState } from "react";
import { Plus, X } from "lucide-react";
import { Terminal, type ITheme } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { bridgeApi } from "../api";
import { cn } from "@/lib/utils";
import { SCROLLBACK_PER_SESSION_LIMIT, rememberScrollback, scrollbackFor } from "../terminalScrollback";

// The terminal pane: several real shells per workspace, each a PTY in the
// workspace's worktree. A dev server, a test watcher, and an ad-hoc prompt
// are three shells, not one. Every shell's xterm stays mounted across
// switches, closing is an explicit act on the tab, and output on a shell you
// are not watching marks its tab instead of vanishing.

/** First design token that resolves on the live document, or undefined. */
function token(styles: CSSStyleDeclaration, ...names: string[]): string | undefined {
  for (const name of names) {
    const value = styles.getPropertyValue(name).trim();
    if (value) return value;
  }
  return undefined;
}

/** Selection needs alpha, and xterm only parses hex/rgb — so derive it here. */
function translucent(color: string | undefined, alpha: number): string | undefined {
  if (!color) return undefined;
  const hex = color.replace("#", "");
  const full = hex.length === 3 ? [...hex].map(digit => digit + digit).join("") : hex;
  if (!/^[\da-f]{6}$/i.test(full)) return color;
  const value = Number.parseInt(full, 16);
  return `rgba(${(value >> 16) & 255}, ${(value >> 8) & 255}, ${value & 255}, ${alpha})`;
}

// xterm paints to a canvas and cannot read CSS variables, so the palette is
// resolved from the tokens on <html> and rebuilt whenever the theme flips.
function terminalTheme(): ITheme {
  const styles = getComputedStyle(document.documentElement);
  const surface = token(styles, "--code", "--background");
  const foreground = token(styles, "--foreground");
  return {
    background: surface,
    foreground,
    cursor: foreground,
    cursorAccent: surface,
    selectionBackground: translucent(foreground, 0.24),
    black: token(styles, "--muted"),
    brightBlack: token(styles, "--syn-comment"),
    red: token(styles, "--destructive"),
    brightRed: token(styles, "--destructive"),
    green: token(styles, "--success"),
    brightGreen: token(styles, "--syn-string"),
    yellow: token(styles, "--warning"),
    brightYellow: token(styles, "--syn-number"),
    blue: token(styles, "--info"),
    brightBlue: token(styles, "--syn-function"),
    magenta: token(styles, "--syn-keyword"),
    brightMagenta: token(styles, "--syn-type"),
    cyan: token(styles, "--syn-tag"),
    brightCyan: token(styles, "--syn-tag"),
    white: token(styles, "--muted-foreground"),
    brightWhite: foreground,
  };
}

type ShellMark = "output" | "exited";

export type TerminalActivity = { running: number; attention: boolean };

/** Which shells each workspace has, surviving pane unmounts the way
 *  scrollback does — the roster is UI memory, the PTYs live in the runtime. */
const rosters = new Map<string, { ids: string[]; labels: Map<string, string>; active: string; counter: number }>();

function rosterFor(workspaceId: string) {
  let roster = rosters.get(workspaceId);
  if (!roster) {
    roster = { ids: [], labels: new Map(), active: "", counter: 0 };
    rosters.set(workspaceId, roster);
  }
  return roster;
}

function ShellHost({ workspaceId, terminalId, active, paneVisible, register }: {
  workspaceId: string;
  terminalId: string;
  active: boolean;
  paneVisible: boolean;
  register: (terminalId: string, term: Terminal | null) => void;
}) {
  const host = useRef<HTMLDivElement>(null);
  const fitRef = useRef<FitAddon>();

  useEffect(() => {
    if (!host.current) return;
    const term = new Terminal({
      fontFamily: "'SFMono-Regular', 'SF Mono', Menlo, monospace", fontSize: 13, lineHeight: 1.42,
      cursorBlink: true, cursorStyle: "bar", convertEol: true,
      theme: terminalTheme(),
    });
    const fit = new FitAddon(); term.loadAddon(fit); term.open(host.current); fit.fit();
    fitRef.current = fit;
    register(terminalId, term);
    const theme = new MutationObserver(() => { term.options.theme = terminalTheme(); });
    theme.observe(document.documentElement, { attributes: true, attributeFilter: ["class", "data-theme"] });
    const previous = scrollbackFor(`${workspaceId}:${terminalId}`); if (previous) term.write(previous);
    else if (!("__TAURI_INTERNALS__" in window)) term.writeln("\x1b[90mBridge workspace shell · terminal is isolated from the agent conversation.\x1b[0m\r\n$ ");
    void bridgeApi.openTerminal(workspaceId, terminalId).catch(error => term.writeln(`\r\n\x1b[31m${String(error)}\x1b[0m`));
    const data = term.onData(value => void bridgeApi.writeTerminal(workspaceId, terminalId, value));
    const resize = new ResizeObserver(() => { fit.fit(); void bridgeApi.resizeTerminal(workspaceId, terminalId, term.rows, term.cols); }); resize.observe(host.current);
    return () => { register(terminalId, null); theme.disconnect(); resize.disconnect(); data.dispose(); term.dispose(); };
  }, [workspaceId, terminalId, register]);

  // A hidden xterm measures as zero; refit when this shell comes back.
  useEffect(() => {
    if (active && paneVisible) fitRef.current?.fit();
  }, [active, paneVisible]);

  return <div
    className={cn("absolute inset-0 p-[14px_12px] [&_.xterm]:h-full [&_.xterm-viewport]:scrollbar-thin [&_.xterm-viewport]:scrollbar-thumb-foreground/12 [&_.xterm-viewport]:scrollbar-track-transparent", !active && "hidden")}
    ref={host}
    data-shell-host={terminalId}
    aria-label={`Workspace shell ${terminalId}`}
  />;
}

export function TerminalPane({ workspaceId, workspacePath, visible = true, onActivity }: {
  workspaceId?: string;
  workspacePath?: string;
  visible?: boolean;
  onActivity?: (activity: TerminalActivity) => void;
}) {
  const [shells, setShells] = useState<string[]>([]);
  const [active, setActive] = useState<string>("");
  const [marks, setMarks] = useState<Map<string, ShellMark>>(new Map());
  const [labels, setLabels] = useState<Map<string, string>>(new Map());
  const [renaming, setRenaming] = useState<string>();
  const terms = useRef(new Map<string, Terminal>());
  const activeRef = useRef(active);
  activeRef.current = active;
  const visibleRef = useRef(visible);
  visibleRef.current = visible;

  const register = useCallback((terminalId: string, term: Terminal | null) => {
    if (term) terms.current.set(terminalId, term);
    else terms.current.delete(terminalId);
  }, []);

  const openShell = useCallback((workspace: string, terminalId: string, activate = true) => {
    const roster = rosterFor(workspace);
    if (!roster.ids.includes(terminalId)) roster.ids.push(terminalId);
    const ordinal = Number(/^t(\d+)$/.exec(terminalId)?.[1] ?? NaN);
    if (Number.isFinite(ordinal)) roster.counter = Math.max(roster.counter, ordinal);
    if (activate) roster.active = terminalId;
    setShells([...roster.ids]);
    setLabels(new Map(roster.labels));
    if (activate) {
      setActive(terminalId);
      setMarks(previous => {
        if (!previous.has(terminalId)) return previous;
        const next = new Map(previous);
        next.delete(terminalId);
        return next;
      });
    }
  }, []);

  // Reattach instead of respawn: the runtime knows which shells are still
  // alive; the roster remembers which the user had. Their union is the strip.
  useEffect(() => {
    if (!workspaceId) return;
    let live = true;
    const roster = rosterFor(workspaceId);
    void bridgeApi.listTerminals(workspaceId).then(alive => {
      if (!live) return;
      for (const terminalId of alive) {
        if (!roster.ids.includes(terminalId)) roster.ids.push(terminalId);
        // Keep the counter ahead of every live id, or the next "+" would
        // regenerate an existing one and merely activate its tab.
        const ordinal = Number(/^t(\d+)$/.exec(terminalId)?.[1] ?? NaN);
        if (Number.isFinite(ordinal)) roster.counter = Math.max(roster.counter, ordinal);
      }
      if (roster.ids.length === 0) {
        roster.counter += 1;
        openShell(workspaceId, `t${roster.counter}`);
        return;
      }
      if (!roster.ids.includes(roster.active)) roster.active = roster.ids[0];
      setShells([...roster.ids]);
      setLabels(new Map(roster.labels));
      setActive(roster.active);
    });
    return () => { live = false; };
  }, [workspaceId, openShell]);

  // One subscription for the whole pane: chunks route to their shell's xterm,
  // and bytes for a shell that is not on screen mark its tab instead.
  useEffect(() => {
    if (!workspaceId) return;
    let offChunk: (() => void) | undefined;
    let offExit: (() => void) | undefined;
    void bridgeApi.onTerminal(chunk => {
      if (chunk.sessionId !== workspaceId) return;
      rememberScrollback(`${chunk.sessionId}:${chunk.terminalId}`, chunk.data);
      terms.current.get(chunk.terminalId)?.write(chunk.data);
      setMarks(previous => {
        const background = chunk.terminalId !== activeRef.current || !visibleRef.current;
        const next = background ? "output" : undefined;
        if (previous.get(chunk.terminalId) === next) return previous;
        const map = new Map(previous);
        if (next) map.set(chunk.terminalId, next);
        else map.delete(chunk.terminalId);
        return map;
      });
    }).then(fn => { offChunk = fn; });
    void bridgeApi.onTerminalExited(exit => {
      if (exit.sessionId !== workspaceId) return;
      setMarks(previous => new Map(previous).set(exit.terminalId, "exited"));
      terms.current.get(exit.terminalId)?.writeln("\r\n\x1b[90m[shell exited]\x1b[0m");
    }).then(fn => { offExit = fn; });
    return () => { offChunk?.(); offExit?.(); };
  }, [workspaceId]);

  useEffect(() => {
    const exited = shells.filter(id => marks.get(id) === "exited").length;
    onActivity?.({ running: Math.max(0, shells.length - exited), attention: exited > 0 });
  }, [shells, marks, onActivity]);

  if (!workspaceId) return <div className="grid h-full place-items-center text-muted-foreground/85 text-xs">Select a workspace to open its shell.</div>;

  const roster = rosterFor(workspaceId);
  const labelOf = (terminalId: string) => labels.get(terminalId) ?? terminalId.replace(/^t/, "shell ");

  return <div className="flex h-full flex-col bg-code">
    <div className="flex min-h-10 shrink-0 items-center gap-0.5 overflow-x-auto border-b border-border px-1.5">
      {shells.map(terminalId => {
        const mark = marks.get(terminalId);
        return <span key={terminalId} className={cn("group/shell flex h-8 shrink-0 items-center gap-1 rounded-md px-1.5 text-[11px]", terminalId === active ? "bg-card text-foreground" : "text-muted-foreground hover:bg-accent")}>
          {mark === "exited"
            ? <span className="h-1.5 w-1.5 rounded-full bg-muted-foreground/50" title="Shell exited" data-shell-mark="exited" />
            : mark === "output"
              ? <span className="h-1.5 w-1.5 rounded-full bg-success" title="New output" data-shell-mark="output" />
              : null}
          {renaming === terminalId
            ? <input
                autoFocus
                defaultValue={labelOf(terminalId)}
                aria-label={`Rename ${terminalId}`}
                onBlur={event => {
                  const value = event.target.value.trim();
                  if (value) { roster.labels.set(terminalId, value); setLabels(new Map(roster.labels)); }
                  setRenaming(undefined);
                }}
                onKeyDown={event => { if (event.key === "Enter") (event.target as HTMLInputElement).blur(); if (event.key === "Escape") setRenaming(undefined); }}
                className="h-7 w-24 rounded-md border border-border bg-background px-1 text-[11px] text-foreground outline-none"
              />
            : <button
                type="button"
                onClick={() => { roster.active = terminalId; setActive(terminalId); setMarks(previous => { if (previous.get(terminalId) !== "output") return previous; const next = new Map(previous); next.delete(terminalId); return next; }); }}
                onDoubleClick={() => setRenaming(terminalId)}
                title={`${labelOf(terminalId)} — double-click to rename`}
                className="min-h-7 max-w-[120px] truncate"
              >{labelOf(terminalId)}</button>}
          <button
            type="button"
            onClick={() => {
              void bridgeApi.closeTerminal(workspaceId, terminalId);
              roster.ids = roster.ids.filter(id => id !== terminalId);
              roster.labels.delete(terminalId);
              if (roster.active === terminalId) roster.active = roster.ids[roster.ids.length - 1] ?? "";
              setShells([...roster.ids]);
              setActive(roster.active);
              setMarks(previous => { const next = new Map(previous); next.delete(terminalId); return next; });
            }}
            aria-label={`Close ${labelOf(terminalId)}`}
            title="Close this shell"
            className="grid h-6 w-6 shrink-0 place-items-center rounded-md text-muted-foreground hover:bg-accent hover:text-foreground"
          ><X size={10} aria-hidden="true" /></button>
        </span>;
      })}
      <button
        type="button"
        onClick={() => { roster.counter += 1; openShell(workspaceId, `t${roster.counter}`); }}
        aria-label="New shell"
        title="New shell"
        className="ml-1 inline-flex h-7 w-7 shrink-0 items-center justify-center rounded-md text-muted-foreground hover:bg-accent hover:text-foreground"
      ><Plus size={13} strokeWidth={1.8} aria-hidden="true" /></button>
    </div>
    <div className="relative min-h-0 flex-1">
      {shells.map(terminalId => <ShellHost
        key={`${workspaceId}:${terminalId}`}
        workspaceId={workspaceId}
        terminalId={terminalId}
        active={terminalId === active}
        paneVisible={visible}
        register={register}
      />)}
    </div>
    {/* Which checkout am I in should never be a question — and neither should
        how much history this surface keeps. */}
    <div className="flex min-h-8 shrink-0 flex-wrap items-center gap-2 border-t border-border px-2.5 font-mono text-[11px] text-muted-foreground">
      <span className="truncate">{workspacePath ?? workspaceId}</span>
      <span className="ml-auto shrink-0">{shells.length} shell{shells.length === 1 ? "" : "s"} · {Math.round(SCROLLBACK_PER_SESSION_LIMIT / 1_000_000)} MB scrollback</span>
    </div>
  </div>;
}
