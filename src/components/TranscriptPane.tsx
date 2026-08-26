import { useEffect, useMemo, useRef, useState } from "react";
import { Check, ChevronRight, Copy, Eye } from "lucide-react";
import type { AgentEvent, SessionEntry } from "../types";
import type { SessionHead } from "../protocol/generated/protocol";
import { cn } from "@/lib/utils";
// The live view merges delta frames for readability; this pane must not —
// the raw stream is the product. Dedupe by id, order by sequence, keep all.
function mergeRaw(current: AgentEvent[], incoming: AgentEvent[]): AgentEvent[] {
  if (!incoming.length) return current;
  const byId = new Map(current.map(event => [event.id, event]));
  for (const event of incoming) if (!byId.has(event.id)) byId.set(event.id, event);
  return [...byId.values()].sort((a, b) => a.sequence - b.sequence);
}

// The transcript pane: the raw normalized event stream and the session forest
// behind the rendered chat. The conversation is a lossy projection of this
// data; when the two disagree — a tool that looked successful, a turn that
// stopped for no visible reason — this is where the disagreement becomes
// visible. It reads the same durable records the UI reads, adds no storage,
// and changes nothing.

export const TRANSCRIPT_PAGE_SIZE = 200;

export type TranscriptLoader = (request: { beforeSequence?: number; tail: boolean }) => Promise<AgentEvent[]>;

function shortId(id: string): string {
  return id.length <= 10 ? id : `${id.slice(0, 8)}…`;
}

function eventTime(iso: string): string {
  const date = new Date(iso);
  return Number.isNaN(date.getTime()) ? "" : date.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" });
}

function CopyJson({ value, label }: { value: unknown; label: string }) {
  const [copied, setCopied] = useState(false);
  return <button
    type="button"
    aria-label={label}
    title={label}
    onClick={() => {
      void navigator.clipboard?.writeText(JSON.stringify(value, null, 2));
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1400);
    }}
    className="inline-flex h-5 items-center gap-1 rounded px-1 text-[10px] text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
  >
    {copied ? <Check size={10} aria-hidden="true" /> : <Copy size={10} aria-hidden="true" />}
    {copied ? "Copied" : "Copy"}
  </button>;
}

export function TranscriptPane({ sessionId, events, entries = [], head, leaves = [], loadOlder, onRevealEntry }: {
  sessionId: string;
  /** Live events, already session-scoped. Merged with loaded pages by id. */
  events: AgentEvent[];
  entries?: SessionEntry[];
  head?: SessionHead | null;
  leaves?: SessionEntry[];
  /** Backed by replaySessionEvents: tail=true for the newest page, otherwise
   *  the window before beforeSequence. */
  loadOlder?: TranscriptLoader;
  onRevealEntry?: (entryId: string) => void;
}) {
  const [view, setView] = useState<"stream" | "entries">("stream");
  const [loaded, setLoaded] = useState<AgentEvent[]>([]);
  const [loading, setLoading] = useState(false);
  const [filter, setFilter] = useState("");
  const [selectedId, setSelectedId] = useState<number>();
  const seeded = useRef(false);

  useEffect(() => {
    if (seeded.current || !loadOlder) return;
    seeded.current = true;
    setLoading(true);
    void loadOlder({ tail: true })
      .then(page => setLoaded(current => mergeRaw(current, page)))
      .finally(() => setLoading(false));
  }, [loadOlder]);

  const stream = useMemo(() => mergeRaw(loaded, events), [loaded, events]);

  const oldest = stream[0]?.sequence;
  const canLoadEarlier = !!loadOlder && oldest !== undefined && oldest > 1;

  const visible = useMemo(() => {
    const needle = filter.trim().toLowerCase();
    if (!needle) return stream;
    return stream.filter(event =>
      event.kind.toLowerCase().includes(needle)
      || (event.text ?? "").toLowerCase().includes(needle)
      || (event.title ?? "").toLowerCase().includes(needle));
  }, [stream, filter]);

  const selected = visible.find(event => event.id === selectedId);

  // The active branch is the parent chain of the active entry; everything on
  // it is context the next turn can see, everything off it is history.
  const activePath = useMemo(() => {
    const byId = new Map(entries.map(entry => [entry.id, entry]));
    const path = new Set<string>();
    let cursor = head?.activeEntryId ?? null;
    while (cursor) {
      path.add(cursor);
      cursor = byId.get(cursor)?.parentEntryId ?? null;
    }
    return path;
  }, [entries, head?.activeEntryId]);

  return <div className="flex h-full flex-col">
    <div className="flex h-9 shrink-0 items-center gap-2 border-b border-border px-2.5">
      <div className="u-segmented h-6">
        {(["stream", "entries"] as const).map(option => <button
          key={option}
          type="button"
          className="u-segmented-item capitalize"
          data-active={view === option}
          onClick={() => setView(option)}
        >{option}</button>)}
      </div>
      {view === "stream" && <>
        <input
          type="search"
          value={filter}
          onChange={event => setFilter(event.target.value)}
          placeholder="Filter kind or text"
          aria-label="Filter events"
          className="h-6 min-w-0 flex-1 rounded-md border border-border bg-background px-2 text-[11px] text-foreground outline-none placeholder:text-muted-foreground/60 focus:border-ring"
        />
        <CopyJson value={visible} label="Copy the filtered stream as JSON" />
      </>}
      {view === "entries" && head && <span className="ml-auto truncate font-mono text-[10px] text-muted-foreground">
        head {head.activeEntryId ? shortId(head.activeEntryId) : "—"} · {leaves.length} {leaves.length === 1 ? "leaf" : "leaves"}
      </span>}
    </div>

    {view === "stream" && <div className="min-h-0 flex-1 overflow-y-auto font-mono text-[10.5px] leading-[1.9]">
      {canLoadEarlier && <button
        type="button"
        disabled={loading}
        onClick={() => {
          if (!loadOlder || oldest === undefined) return;
          setLoading(true);
          void loadOlder({ beforeSequence: oldest, tail: false })
            .then(page => setLoaded(current => mergeRaw(current, page)))
            .finally(() => setLoading(false));
        }}
        className="block w-full border-b border-border px-2.5 py-1.5 text-left text-[10.5px] text-muted-foreground transition-colors hover:bg-accent hover:text-foreground disabled:opacity-50"
      >{loading ? "Loading…" : "Load earlier events"}</button>}
      {visible.length === 0 && !loading && <p className="px-2.5 py-4 text-muted-foreground">
        {stream.length === 0 ? "No events recorded for this session yet." : "Nothing matches the filter."}
      </p>}
      {visible.map(event => <div key={event.id} className="border-b border-border/50">
        <button
          type="button"
          onClick={() => setSelectedId(current => current === event.id ? undefined : event.id)}
          aria-expanded={selectedId === event.id}
          className={cn("flex w-full items-baseline gap-2 px-2.5 text-left transition-colors hover:bg-accent", selectedId === event.id && "bg-code")}
        >
          <span className="w-10 shrink-0 text-right text-muted-foreground/50">{event.sequence}</span>
          <span className="w-[42%] min-w-0 shrink-0 truncate text-foreground">{event.kind}</span>
          <span className="min-w-0 flex-1 truncate text-muted-foreground/70">{event.status ?? event.title ?? event.text ?? ""}</span>
          <span className="shrink-0 text-muted-foreground/50">{eventTime(event.createdAt)}</span>
        </button>
        {selected?.id === event.id && <div className="border-t border-border bg-code px-2.5 py-1.5">
          <div className="flex items-center gap-2">
            <span className="text-[10px] uppercase tracking-[0.08em] text-muted-foreground/60">Raw event</span>
            <span className="ml-auto"><CopyJson value={event} label={`Copy event ${event.sequence} as JSON`} /></span>
          </div>
          <pre className="mt-1 max-h-56 overflow-auto whitespace-pre-wrap break-words text-[10px] leading-[1.6] text-foreground">{JSON.stringify(event, null, 2)}</pre>
        </div>}
      </div>)}
    </div>}

    {view === "entries" && <div className="min-h-0 flex-1 overflow-y-auto font-mono text-[10.5px] leading-[1.9]">
      {entries.length === 0 && <p className="px-2.5 py-4 text-muted-foreground">No forest entries for this session yet.</p>}
      {entries.map(entry => <div key={entry.id} className="group flex items-baseline gap-2 border-b border-border/50 px-2.5 transition-colors hover:bg-accent">
        <span className={cn("w-1.5 shrink-0 self-stretch", activePath.has(entry.id) ? "bg-success/60" : "bg-transparent")} aria-hidden="true" />
        <span className="w-16 shrink-0 truncate text-muted-foreground/60">{shortId(entry.id)}</span>
        <span className="min-w-0 flex-1 truncate text-foreground">{entry.kind}</span>
        <span className="shrink-0 text-muted-foreground/50">{entry.contextVisibility}</span>
        {activePath.has(entry.id) && <span className="shrink-0 text-[9px] uppercase tracking-[0.08em] text-success">active</span>}
        {onRevealEntry && <button
          type="button"
          onClick={() => onRevealEntry(entry.id)}
          aria-label={`Reveal entry ${entry.id} in the conversation`}
          title="Reveal in the conversation"
          className="hidden shrink-0 items-center gap-1 rounded px-1 text-[10px] text-muted-foreground hover:bg-card hover:text-foreground focus-visible:inline-flex group-hover:inline-flex"
        >
          <Eye size={10} aria-hidden="true" />
          reveal
        </button>}
        <ChevronRight size={10} className="shrink-0 text-muted-foreground/30" aria-hidden="true" />
      </div>)}
    </div>}

    <div className="flex h-7 shrink-0 items-center gap-2 border-t border-border px-2.5 font-mono text-[10px] text-muted-foreground">
      <span>{view === "stream" ? `${visible.length} of ${stream.length} events` : `${entries.length} entries`}</span>
      <span className="ml-auto truncate">{sessionId}</span>
    </div>
  </div>;
}
