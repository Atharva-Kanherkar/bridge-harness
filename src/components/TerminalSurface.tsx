import { useEffect, useRef, useState } from "react";
import { ChevronDown, ChevronUp, Search, X } from "lucide-react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { SearchAddon } from "@xterm/addon-search";
import { Unicode11Addon } from "@xterm/addon-unicode11";
import { WebLinksAddon } from "@xterm/addon-web-links";
import { openExternalUrl } from "../externalLinks";
import { bridgeApi } from "../api";
import { terminalTheme, terminalSearchDecorations } from "../terminal/theme";
import { attachWebgl } from "../terminal/webgl";
import { TerminalReplay } from "../terminal/replay";
import type { TerminalRecord } from "../terminal/types";

export function TerminalSurface({ record, focused, onRecord, searchRequest = 0 }: {
  record: TerminalRecord;
  focused: boolean;
  onRecord: (record: TerminalRecord) => void;
  searchRequest?: number;
}) {
  const host = useRef<HTMLDivElement>(null);
  const termRef = useRef<Terminal>();
  const searchRef = useRef<SearchAddon>();
  const retry = useRef<() => void>(() => {});
  const latest = useRef({ onRecord, focused });
  latest.current = { onRecord, focused };
  const [error, setError] = useState<string>();
  const [restoring, setRestoring] = useState(true);
  const [searchOpen, setSearchOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [matches, setMatches] = useState("");
  const { workspaceId, terminalId, generation } = record;

  useEffect(() => {
    if (!host.current) return;
    let disposed = false;
    let replaying = true;
    let status = record.status;
    let scheduled = 0;
    let lastSize = "";
    let resizing = false;
    let pendingSize: { rows: number; cols: number } | undefined;
    const terminal = new Terminal({ fontFamily: "'Geist Mono Variable', monospace", fontSize: 12, lineHeight: 1.35, cursorBlink: true, cursorStyle: "bar", scrollback: 5000, allowProposedApi: true, theme: terminalTheme(), vtExtensions: { kittyKeyboard: true } });
    const fit = new FitAddon();
    const search = new SearchAddon();
    terminal.loadAddon(fit);
    terminal.loadAddon(search);
    terminal.loadAddon(new Unicode11Addon());
    terminal.unicode.activeVersion = "11";
    terminal.loadAddon(new WebLinksAddon((event, url) => {
      if ((event.metaKey || event.ctrlKey) && /^https?:\/\//i.test(url)) void openExternalUrl(url).catch(report);
    }));
    terminal.open(host.current);
    termRef.current = terminal; searchRef.current = search;
    const write = (data: string) => new Promise<void>(resolve => terminal.write(data, resolve));
    function report(value: unknown) { if (!disposed) setError(String(value)); }
    async function flushResize() {
      if (resizing) return;
      resizing = true;
      try {
        while (pendingSize && !disposed) {
          const size = pendingSize;
          pendingSize = undefined;
          await bridgeApi.resizeTerminal(workspaceId, terminalId, size.rows, size.cols);
        }
      } catch (error) { lastSize = ""; report(error); }
      finally { resizing = false; }
    }
    function scheduleFit() {
      cancelAnimationFrame(scheduled);
      scheduled = requestAnimationFrame(() => {
        if (disposed || replaying || !host.current?.clientWidth || !host.current.clientHeight) return;
        const size = fit.proposeDimensions();
        if (!size) return;
        const cols = Math.max(2, Math.min(1000, size.cols)), rows = Math.max(2, Math.min(500, size.rows));
        terminal.resize(cols, rows);
        const key = `${cols}:${rows}`;
        if (status === "running" && key !== lastSize) {
          lastSize = key;
          // Keep only the latest pending dimensions during rapid dragging.
          pendingSize = { rows, cols };
          void flushResize();
        }
      });
    }
    const feed = new TerminalReplay({
      snapshot: () => bridgeApi.terminalSnapshot(workspaceId, terminalId),
      restore: async snapshot => {
        replaying = true; setRestoring(true);
        terminal.reset();
        terminal.resize(snapshot.record.cols, snapshot.record.rows);
        await write(snapshot.ansi);
        if (disposed) return;
        status = snapshot.record.status;
        latest.current.onRecord(snapshot.record);
        setError(undefined); setRestoring(false);
        replaying = false; lastSize = ""; scheduleFit();
        if (latest.current.focused) terminal.focus();
      },
      frame: async frame => {
        if (frame.rows && frame.cols) terminal.resize(frame.cols, frame.rows);
        if (frame.data) await write(frame.data);
        if (frame.status) {
          status = frame.status;
          void bridgeApi.terminalSnapshot(workspaceId, terminalId).then(snapshot => { if (!disposed) latest.current.onRecord(snapshot.record); }).catch(report);
        }
      },
      error: value => { replaying = true; setRestoring(false); report(value); },
    });
    const unlisten: (() => void)[] = [];
    // Install both listeners before requesting the snapshot boundary.
    async function connect() {
      if (unlisten.length) { await feed.recover(); return; }
      const results = await Promise.allSettled([
        bridgeApi.onTerminalFrame(frame => { if (frame.workspaceId === workspaceId && frame.terminalId === terminalId) feed.receive(frame); }),
        bridgeApi.onTerminalLagged(() => { void feed.recover(); }),
      ]);
      const listeners = results.flatMap(result => result.status === "fulfilled" ? [result.value] : []);
      const failure = results.find(result => result.status === "rejected");
      if (disposed || failure) listeners.forEach(fn => fn());
      if (disposed) return;
      if (failure?.status === "rejected") { setRestoring(false); report(failure.reason); }
      else { unlisten.push(...listeners); await feed.recover(); }
    }
    retry.current = () => { void connect(); };
    void connect();
    const input = terminal.onData(data => { if (!replaying && status === "running") void bridgeApi.writeTerminal(workspaceId, terminalId, data).catch(report); });
    terminal.attachCustomKeyEventHandler(event => {
      if (event.type !== "keydown") return true;
      if (event.metaKey && event.key.toLowerCase() === "c" && terminal.hasSelection()) { void navigator.clipboard.writeText(terminal.getSelection()).catch(report); return false; }
      return true;
    });
    const searchResult = search.onDidChangeResults(result => setMatches(result.resultCount ? `${result.resultIndex + 1} / ${result.resultCount}` : "No matches"));
    const observer = new ResizeObserver(scheduleFit); observer.observe(host.current);
    const theme = new MutationObserver(() => { terminal.options.theme = terminalTheme(); scheduleFit(); });
    theme.observe(document.documentElement, { attributes: true, attributeFilter: ["class", "data-theme"] });
    const gpu = attachWebgl(terminal, scheduleFit);
    return () => {
      disposed = true; feed.dispose(); unlisten.forEach(fn => fn()); cancelAnimationFrame(scheduled);
      observer.disconnect(); theme.disconnect(); input.dispose(); searchResult.dispose(); gpu(); terminal.dispose(); termRef.current = undefined;
    };
    // Identity creates presentation. Mutable metadata must never remount xterm.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [workspaceId, terminalId, generation]);

  useEffect(() => { if (focused && !searchOpen) termRef.current?.focus(); }, [focused, searchOpen]);
  useEffect(() => { if (searchRequest) setSearchOpen(true); }, [searchRequest]);
  function find(previous = false) {
    const options = { decorations: terminalSearchDecorations() };
    if (previous) searchRef.current?.findPrevious(query, options); else searchRef.current?.findNext(query, options);
  }
  return <div className="relative flex min-h-0 min-w-0 flex-1 flex-col bg-code">
    {searchOpen && <div className="flex shrink-0 items-center gap-1 border-b border-border bg-background px-2 py-1">
      <Search size={12} aria-hidden="true" />
      <input autoFocus aria-label="Search terminal scrollback" className="min-w-0 flex-1 bg-transparent px-1 py-1 text-xs outline-none" value={query} onChange={event => { setQuery(event.target.value); searchRef.current?.findNext(event.target.value, { incremental: true, decorations: terminalSearchDecorations() }); }} onKeyDown={event => { if (event.key === "Enter") find(event.shiftKey); if (event.key === "Escape") setSearchOpen(false); event.stopPropagation(); }} />
      <span className="text-[10px] text-muted-foreground">{query && matches}</span>
      <button type="button" aria-label="Previous match" className="rounded p-1 hover:bg-accent" onClick={() => find(true)}><ChevronUp size={13} /></button>
      <button type="button" aria-label="Next match" className="rounded p-1 hover:bg-accent" onClick={() => find()}><ChevronDown size={13} /></button>
      <button type="button" aria-label="Close search" className="rounded p-1 hover:bg-accent" onClick={() => setSearchOpen(false)}><X size={13} /></button>
    </div>}
    <div ref={host} aria-label={`${record.title} terminal`} className="min-h-0 flex-1 overflow-hidden p-2 [&_.xterm]:h-full [&_.xterm-viewport]:scrollbar-thin" />
    {restoring && <div role="status" className="pointer-events-none absolute bottom-2 right-3 rounded bg-background px-2 py-1 text-[10px] text-muted-foreground">Restoring terminal…</div>}
    {error && <div role="alert" className="flex items-center gap-2 border-t border-border bg-background px-3 py-2 text-xs text-destructive"><span className="min-w-0 flex-1">{error}</span><button type="button" className="shrink-0 underline" onClick={() => retry.current()}>Reconnect</button></div>}
  </div>;
}
