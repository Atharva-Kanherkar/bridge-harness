import { useCallback, useEffect, useRef, useState } from "react";
import { Plus, RotateCcw, X } from "lucide-react";
import type { Terminal } from "@xterm/xterm";
import { backendMode, bridgeApi } from "../api";
import { errorMessage } from "../errors";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { RowButton } from "@/components/ui/row";
import { Tip } from "@/components/ui/tooltip";
import { terminalRunInput } from "../terminal/runCommand";
import { onTerminalRun, type TerminalRun } from "../terminal/runQueue";
import { TERMINAL_SCROLLBACK, type TerminalRecord } from "../terminal/types";
import { TerminalSurface } from "./TerminalSurface";

// The terminal pane: several real shells per workspace, each a PTY in the
// workspace's worktree. A dev server, a test watcher, and an ad-hoc prompt
// are three shells, not one. Every shell stays mounted across switches,
// closing is an explicit act on the tab, and output on a shell you are not
// watching marks its tab instead of vanishing.
//
// Each shell renders through `TerminalSurface`, the same engine Agent Fleet
// uses: the runtime's screen checkpoint plus sequenced frames, so a reopened
// pane shows the exact screen rather than a replayed byte log, and a hidden
// pane is never squeezed to two columns.

type ShellMark = "output" | "exited";

export type TerminalActivity = { running: number; attention: boolean };

/** How long a shell this pane just spawned may take to draw its first prompt
 *  before a run is typed anyway, and the quiet that means it has drawn. */
const PROMPT_WAIT_MS = 4_000;
const PROMPT_QUIET_MS = 150;
const SHELL_MOUNT_WAIT_MS = 5_000;

const sleep = (ms: number) => new Promise<void>(resolve => { window.setTimeout(resolve, ms); });

/** Which shells each workspace has, surviving pane unmounts. The roster is
 *  UI memory, the PTYs and their history live in the runtime. */
const rosters = new Map<string, { ids: string[]; labels: Map<string, string>; active: string; counter: number }>();

function rosterFor(workspaceId: string) {
  let roster = rosters.get(workspaceId);
  if (!roster) {
    roster = { ids: [], labels: new Map(), active: "", counter: 0 };
    rosters.set(workspaceId, roster);
  }
  return roster;
}

function without<K, V>(map: Map<K, V>, key: K): Map<K, V> {
  if (!map.has(key)) return map;
  const next = new Map(map);
  next.delete(key);
  return next;
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
  const [records, setRecords] = useState<Map<string, TerminalRecord>>(new Map());
  const [failures, setFailures] = useState<Map<string, string>>(new Map());
  const [renaming, setRenaming] = useState<string>();
  /** The shell the user last put the cursor in; it takes focus when shown. */
  const [focusShell, setFocusShell] = useState<string>();
  const terms = useRef(new Map<string, Terminal>());
  const activeRef = useRef(active);
  activeRef.current = active;
  const visibleRef = useRef(visible);
  visibleRef.current = visible;
  const alive = useRef(true);
  useEffect(() => { alive.current = true; return () => { alive.current = false; }; }, []);

  // What a chat run needs from each shell: its xterm, the promise its PTY
  // opened with, when this pane spawned it, and when it last spoke.
  const opened = useRef(new Map<string, Promise<unknown>>());
  const shellWaiters = useRef(new Map<string, () => void>());
  const born = useRef(new Map<string, number>());
  const spokeAt = useRef(new Map<string, number>());
  const marksRef = useRef(marks);
  marksRef.current = marks;
  const runs = useRef(Promise.resolve());

  const registerTerm = useCallback((terminalId: string, term: Terminal | null) => {
    if (term) {
      terms.current.set(terminalId, term);
      shellWaiters.current.get(terminalId)?.();
    } else {
      terms.current.delete(terminalId);
    }
  }, []);

  const onRecord = useCallback((record: TerminalRecord) => {
    if (!alive.current) return;
    setRecords(previous => previous.get(record.terminalId) === record ? previous : new Map(previous).set(record.terminalId, record));
  }, []);

  /** Attach to the shell's PTY, starting it when it is not running. Reopening
   *  a live id returns its record; an ended one starts a fresh process. */
  const startShell = useCallback((workspace: string, terminalId: string) => {
    const opening = bridgeApi.createTerminal({ workspaceId: workspace, terminalId, restart: true });
    opened.current.set(terminalId, opening);
    opening.then(
      record => {
        if (!alive.current) return;
        onRecord(record);
        setFailures(previous => without(previous, terminalId));
      },
      error => { if (alive.current) setFailures(previous => new Map(previous).set(terminalId, errorMessage(error))); },
    );
    return opening;
  }, [onRecord]);

  const openShell = useCallback((workspace: string, terminalId: string, activate = true) => {
    const roster = rosterFor(workspace);
    if (!roster.ids.includes(terminalId)) { roster.ids.push(terminalId); born.current.set(terminalId, Date.now()); }
    const ordinal = Number(/^t(\d+)$/.exec(terminalId)?.[1] ?? NaN);
    if (Number.isFinite(ordinal)) roster.counter = Math.max(roster.counter, ordinal);
    if (activate) roster.active = terminalId;
    setShells([...roster.ids]);
    setLabels(new Map(roster.labels));
    if (activate) {
      setActive(terminalId);
      setMarks(previous => without(previous, terminalId));
    }
  }, []);

  // Every shell on the strip has a PTY behind it.
  useEffect(() => {
    if (!workspaceId) return;
    for (const terminalId of shells) if (!opened.current.has(terminalId)) void startShell(workspaceId, terminalId).catch(() => undefined);
  }, [workspaceId, shells, startShell]);

  const restartShell = useCallback((terminalId: string) => {
    if (!workspaceId) return;
    born.current.set(terminalId, Date.now());
    setMarks(previous => without(previous, terminalId));
    setFocusShell(terminalId);
    void startShell(workspaceId, terminalId).catch(() => undefined);
  }, [workspaceId, startShell]);

  /** The shell's xterm once its surface has mounted. */
  const shellFor = useCallback((terminalId: string) => {
    const mounted = terms.current.get(terminalId);
    if (mounted) return Promise.resolve(mounted);
    return new Promise<Terminal>((resolve, reject) => {
      const timer = window.setTimeout(() => { shellWaiters.current.delete(terminalId); reject(new Error("The shell did not start.")); }, SHELL_MOUNT_WAIT_MS);
      shellWaiters.current.set(terminalId, () => {
        window.clearTimeout(timer);
        shellWaiters.current.delete(terminalId);
        resolve(terms.current.get(terminalId)!);
      });
    });
  }, []);

  /** A shell this pane just spawned is still sourcing its rc files: typing
   *  into it now races the prompt. Wait for it to speak and fall quiet. */
  const promptDrawn = useCallback(async (terminalId: string) => {
    const birth = born.current.get(terminalId);
    if (birth === undefined || backendMode() === "mock") return;
    while (Date.now() - birth < PROMPT_WAIT_MS) {
      const spoke = spokeAt.current.get(terminalId);
      if (spoke !== undefined && Date.now() - spoke >= PROMPT_QUIET_MS) return;
      await sleep(50);
    }
  }, []);

  /** A run the chat asked for: typed into the active shell, or a new one when
   *  there is none or it has exited, then Enter. One write, so the paste and
   *  the Enter cannot arrive out of order. */
  const takeRun = useCallback(async (workspace: string, run: TerminalRun) => {
    const roster = rosterFor(workspace);
    let target = roster.ids.includes(roster.active) ? roster.active : "";
    if (!target || marksRef.current.get(target) === "exited") {
      roster.counter += 1;
      target = `t${roster.counter}`;
      openShell(workspace, target);
    } else if (activeRef.current !== target) {
      openShell(workspace, target);
    }
    try {
      const term = await shellFor(target);
      await opened.current.get(target);
      await promptDrawn(target);
      if (!run.open()) return;
      await bridgeApi.writeTerminal(workspace, target, terminalRunInput(run.command, term.modes.bracketedPasteMode));
      // The browser preview has no shell to echo what it was given.
      if (backendMode() === "mock") term.write(`${run.command.replace(/\n/g, "\r\n")}\r\n\x1b[90m(preview: runs in Bridge desktop)\x1b[0m\r\n$ `);
      born.current.delete(target);
      setFocusShell(target);
      term.focus();
      run.settle(roster.labels.get(target) ?? target.replace(/^t/, "shell "));
    } catch (error) {
      run.fail(error);
    }
  }, [openShell, shellFor, promptDrawn]);

  // Reattach instead of respawn: the runtime knows which shells are still
  // alive; the roster remembers which the user had. Their union is the strip.
  // Runs from the chat are taken only once the roster is known, so a run can
  // never spawn a shell beside live ones the list had not returned yet.
  useEffect(() => {
    if (!workspaceId) return;
    let live = true;
    let offRuns: (() => void) | undefined;
    const takeRuns = () => {
      offRuns = onTerminalRun(workspaceId, run => {
        runs.current = runs.current.then(() => takeRun(workspaceId, run));
      });
    };
    const roster = rosterFor(workspaceId);
    // A list that fails still opens the pane on the roster alone: a live
    // shell it missed is reattached by its id, not respawned. Before, the
    // pane stayed empty and took no runs, so every Run in the chat spun
    // until it timed out.
    void bridgeApi.listTerminals(workspaceId).catch((): string[] => []).then(alive => {
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
        takeRuns();
        return;
      }
      if (!roster.ids.includes(roster.active)) roster.active = roster.ids[0];
      setShells([...roster.ids]);
      setLabels(new Map(roster.labels));
      setActive(roster.active);
      takeRuns();
    });
    return () => { live = false; offRuns?.(); };
  }, [workspaceId, openShell, takeRun]);

  // Activity only: the bytes themselves reach each surface as sequenced
  // frames, so nothing here writes to an xterm.
  useEffect(() => {
    if (!workspaceId) return;
    let offChunk: (() => void) | undefined;
    let offExit: (() => void) | undefined;
    let live = true;
    void bridgeApi.onTerminal(chunk => {
      if (chunk.sessionId !== workspaceId) return;
      spokeAt.current.set(chunk.terminalId, Date.now());
      setMarks(previous => {
        const background = chunk.terminalId !== activeRef.current || !visibleRef.current;
        const next = background ? "output" : undefined;
        if (previous.get(chunk.terminalId) === next) return previous;
        const map = new Map(previous);
        if (next) map.set(chunk.terminalId, next);
        else map.delete(chunk.terminalId);
        return map;
      });
    }).then(fn => { if (live) offChunk = fn; else fn(); });
    void bridgeApi.onTerminalExited(exit => {
      if (exit.sessionId !== workspaceId) return;
      setMarks(previous => new Map(previous).set(exit.terminalId, "exited"));
    }).then(fn => { if (live) offExit = fn; else fn(); });
    return () => { live = false; offChunk?.(); offExit?.(); };
  }, [workspaceId]);

  useEffect(() => {
    const exited = shells.filter(id => marks.get(id) === "exited").length;
    onActivity?.({ running: Math.max(0, shells.length - exited), attention: exited > 0 });
  }, [shells, marks, onActivity]);

  if (!workspaceId) return <div className="grid h-full place-items-center text-muted-foreground/85 text-xs">Select a workspace to open its shell.</div>;

  const roster = rosterFor(workspaceId);
  // Shells opened here are numbered; one Agent Fleet started reads as its title.
  const labelOf = (terminalId: string) => labels.get(terminalId)
    ?? (/^t\d+$/.test(terminalId) ? terminalId.replace(/^t/, "shell ") : records.get(terminalId)?.title ?? "shell");
  const activate = (terminalId: string) => {
    roster.active = terminalId;
    setActive(terminalId);
    setFocusShell(terminalId);
    setMarks(previous => previous.get(terminalId) === "output" ? without(previous, terminalId) : previous);
  };

  return <div className="flex h-full flex-col bg-code">
    <div className="flex min-h-10 shrink-0 items-center gap-0.5 overflow-x-auto border-b border-border px-1.5">
      {shells.map(terminalId => {
        const mark = marks.get(terminalId);
        return <span key={terminalId} className={cn("group/shell flex h-8 shrink-0 items-center gap-1 rounded-md px-1.5 text-caption", terminalId === active ? "bg-card text-foreground" : "text-muted-foreground hover:bg-accent")}>
          {mark === "exited"
            ? <Tip label="Shell exited"><span className="h-1.5 w-1.5 rounded-full bg-muted-foreground/50" data-shell-mark="exited" /></Tip>
            : mark === "output"
              ? <Tip label="New output"><span className="h-1.5 w-1.5 rounded-full bg-success" data-shell-mark="output" /></Tip>
              : null}
          {renaming === terminalId
            ? <Input
                size="sm"
                className="w-24"
                autoFocus
                defaultValue={labelOf(terminalId)}
                aria-label={`Rename ${terminalId}`}
                onBlur={event => {
                  const value = event.target.value.trim();
                  if (value) { roster.labels.set(terminalId, value); setLabels(new Map(roster.labels)); }
                  setRenaming(undefined);
                }}
                onKeyDown={event => { if (event.key === "Enter") (event.target as HTMLInputElement).blur(); if (event.key === "Escape") setRenaming(undefined); }}
              />
            : <Tip label={`${labelOf(terminalId)} — double-click to rename`}><RowButton
                size="bare"
                tone="plain"
                onClick={() => activate(terminalId)}
                onDoubleClick={() => setRenaming(terminalId)}
                className="max-w-[120px]"
              ><span className="flex min-h-7 min-w-0 items-center truncate text-inherit">{labelOf(terminalId)}</span></RowButton></Tip>}
          <Button
            variant="quiet"
            size="icon-2xs"
            onClick={() => {
              void bridgeApi.closeTerminal(workspaceId, terminalId).catch(() => undefined);
              roster.ids = roster.ids.filter(id => id !== terminalId);
              roster.labels.delete(terminalId);
              opened.current.delete(terminalId);
              if (roster.active === terminalId) roster.active = roster.ids[roster.ids.length - 1] ?? "";
              setShells([...roster.ids]);
              setActive(roster.active);
              setMarks(previous => without(previous, terminalId));
              setRecords(previous => without(previous, terminalId));
              setFailures(previous => without(previous, terminalId));
            }}
            aria-label={`Close ${labelOf(terminalId)}`}
            tooltip="Close this shell"
          ><X size={10} aria-hidden="true" /></Button>
        </span>;
      })}
      <Button
        variant="quiet"
        size="icon-xs"
        className="ml-1"
        onClick={() => { roster.counter += 1; const terminalId = `t${roster.counter}`; openShell(workspaceId, terminalId); setFocusShell(terminalId); }}
        aria-label="New shell"
        tooltip="New shell"
      ><Plus size={13} strokeWidth={1.8} aria-hidden="true" /></Button>
    </div>
    <div className="relative min-h-0 flex-1">
      {shells.map(terminalId => {
        const record = records.get(terminalId);
        const failure = failures.get(terminalId);
        const ended = marks.get(terminalId) === "exited" || (record !== undefined && record.status !== "running");
        return <div
          key={`${workspaceId}:${terminalId}`}
          data-shell-host={terminalId}
          aria-label={`Workspace shell ${terminalId}`}
          className={cn("absolute inset-0 flex flex-col", terminalId !== active && "hidden")}
        >
          {record
            ? <TerminalSurface
                record={record}
                focused={visible && terminalId === active && focusShell === terminalId}
                onRecord={onRecord}
                onTerminal={term => registerTerm(terminalId, term)}
              />
            : failure
              ? <div role="alert" className="flex flex-1 flex-col items-center justify-center gap-2 px-6 text-center text-xs text-destructive">
                  <span>{failure}</span>
                  <Button variant="outline" size="xs" onClick={() => restartShell(terminalId)}>Try again</Button>
                </div>
              : <div role="status" className="grid flex-1 place-items-center text-xs text-muted-foreground">Starting shell…</div>}
          {ended && <div className="flex shrink-0 items-center gap-2 border-t border-border bg-background px-2.5 py-1.5 text-caption text-muted-foreground">
            <span className="min-w-0 flex-1 truncate">Shell ended{record?.exitCode != null ? ` · exit ${record.exitCode}` : ""}.</span>
            <Button variant="ghost" size="2xs" onClick={() => restartShell(terminalId)}><RotateCcw aria-hidden="true" />Restart</Button>
          </div>}
        </div>;
      })}
    </div>
    {/* Which checkout am I in should never be a question — and neither should
        how much history this surface keeps. */}
    <div className="flex min-h-8 shrink-0 flex-wrap items-center gap-2 border-t border-border px-2.5 font-mono text-caption text-muted-foreground">
      <span className="truncate">{workspacePath ?? workspaceId}</span>
      <span className="ml-auto shrink-0">{shells.length} shell{shells.length === 1 ? "" : "s"} · {TERMINAL_SCROLLBACK.toLocaleString("en-US")} lines scrollback</span>
    </div>
  </div>;
}
