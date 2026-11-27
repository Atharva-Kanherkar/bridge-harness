import { useEffect, useRef, useState } from "react";
import { ChevronDown, ChevronUp, Search, X } from "lucide-react";
import type { Terminal } from "@xterm/xterm";
import { terminalSearchDecorations } from "../terminal/theme";
import { acquireTerminalEngine, releaseTerminalEngine, type TerminalEngine, type TerminalViewState } from "../terminal/engine";
import type { TerminalRecord } from "../terminal/types";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Skeleton, SkeletonRegion } from "@/components/ui/skeleton";
import { useLatest } from "../useLatest";
import { useResetOnChange } from "../useResetOnChange";

/** One terminal on screen. The xterm, its checkpoint replay, and its input
 *  live in a `TerminalEngine`; this view gives it a host, a size, and the
 *  search and status chrome. Agent Fleet panes and the dock's shell tabs
 *  both render through here. */
export function TerminalSurface({ record, focused, onRecord, onSearchHandled = () => {}, searchRequest = 0, onTerminal }: {
  record: TerminalRecord;
  focused: boolean;
  onRecord: (record: TerminalRecord) => void;
  onSearchHandled?: () => void;
  searchRequest?: number;
  /** The live xterm, then null when this view lets go of it. */
  onTerminal?: (terminal: Terminal | null) => void;
}) {
  const host = useRef<HTMLDivElement>(null);
  const engineRef = useRef<TerminalEngine>();
  const latest = useLatest({ onRecord, onSearchHandled, onTerminal, focused });
  const [view, setView] = useState<TerminalViewState>({ restoring: true });
  const [searchOpen, setSearchOpen] = useState(!!searchRequest);
  const [query, setQuery] = useState("");
  const [matches, setMatches] = useState("");
  const { workspaceId, terminalId, generation } = record;

  useEffect(() => {
    const element = host.current;
    if (!element) return;
    const engine = acquireTerminalEngine(record);
    engineRef.current = engine;
    engine.bind({ record: next => latest.current.onRecord(next), state: setView, matches: setMatches });
    engine.setFocused(latest.current.focused);
    engine.attach(element);
    const observer = new ResizeObserver(() => engine.layout());
    observer.observe(element);
    latest.current.onTerminal?.(engine.terminal);
    return () => {
      observer.disconnect();
      // oxlint-disable-next-line react-hooks/exhaustive-deps -- the newest callback is the one to tell; useLatest keeps it current
      latest.current.onTerminal?.(null);
      engineRef.current = undefined;
      releaseTerminalEngine(engine);
    };
    // Identity creates presentation. Mutable metadata must never remount xterm.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [workspaceId, terminalId, generation]);

  useEffect(() => { engineRef.current?.setFocused(focused && !searchOpen); }, [focused, searchOpen]);
  useResetOnChange([searchRequest], () => { if (searchRequest) setSearchOpen(true); });
  // Telling the parent it was handled is its state, so it waits for the commit.
  useEffect(() => {
    if (searchRequest) latest.current.onSearchHandled();
  }, [searchRequest, latest]);
  function find(previous = false) {
    const options = { decorations: terminalSearchDecorations() };
    if (previous) engineRef.current?.search.findPrevious(query, options); else engineRef.current?.search.findNext(query, options);
  }
  return <div className="relative flex min-h-0 min-w-0 flex-1 flex-col bg-code">
    {searchOpen && <div className="flex shrink-0 items-center gap-1 border-b border-border bg-background px-2 py-1 text-xs">
      <Search size={12} aria-hidden="true" />
      <Input unstyled size="bare" autoFocus aria-label="Search terminal scrollback" className="m-1 min-w-0 flex-1" value={query} onChange={event => { setQuery(event.target.value); engineRef.current?.search.findNext(event.target.value, { incremental: true, decorations: terminalSearchDecorations() }); }} onKeyDown={event => { if (event.key === "Enter") find(event.shiftKey); if (event.key === "Escape") setSearchOpen(false); event.stopPropagation(); }} />
      <span className="text-micro text-muted-foreground">{query && matches}</span>
      <Button variant="quiet" size="icon-xs" aria-label="Previous match" onClick={() => find(true)}><ChevronUp size={13} /></Button>
      <Button variant="quiet" size="icon-xs" aria-label="Next match" onClick={() => find()}><ChevronDown size={13} /></Button>
      <Button variant="quiet" size="icon-xs" aria-label="Close search" onClick={() => setSearchOpen(false)}><X size={13} /></Button>
    </div>}
    {/* Padding sits on this wrapper, never on the host the engine measures.
        xterm 6 leaves its viewport at xterm.css's black instead of the theme
        background, which showed as a black band under the last row; that
        stylesheet is unlayered, so only an important utility outranks it. */}
    <div className="min-h-0 flex-1 overflow-hidden p-2">
      <div ref={host} aria-label={`${record.title} terminal`} className="h-full w-full overflow-hidden [&_.xterm]:h-full [&_.xterm-viewport]:bg-transparent! [&_.xterm-viewport]:scrollbar-thin" />
    </div>
    {view.restoring && <SkeletonRegion label="Restoring terminal" className="pointer-events-none absolute bottom-2 right-3"><Skeleton className="h-2.5 w-16" /></SkeletonRegion>}
    {view.error && <div role="alert" className="flex items-center gap-2 border-t border-border bg-background px-3 py-2 text-xs text-destructive"><span className="min-w-0 flex-1">{view.error}</span><Button variant="link" size="inline" className="shrink-0" onClick={() => engineRef.current?.retry()}>Reconnect</Button></div>}
  </div>;
}
