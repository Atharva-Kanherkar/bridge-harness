import { useEffect, useMemo, useRef, useState } from "react";
import { AlertTriangle, Check, ChevronRight, Copy, Download, Eye } from "lucide-react";
import type { AgentEvent, ExportSessionTranscriptResult, SessionEntry } from "../types";
import type { SessionHead } from "../protocol/generated/protocol";
import { bridgeApi } from "../api";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { RowButton } from "@/components/ui/row";
import { SegmentedGroup, SegmentedItem } from "@/components/ui/segmented";
import { Tip } from "@/components/ui/tooltip";
import {
  buildTranscriptRows,
  countFacets,
  FACET_LABELS,
  filterTranscriptRows,
  TRANSCRIPT_FACETS,
  turnNumbersAreAbsolute,
  type TranscriptFacet,
} from "./transcriptFacets";
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
//
// A flat list of frames answers no question on its own, so every row carries
// the bucket it belongs to, the turn it fell in, and whether it failed (see
// `transcriptFacets.ts`). The failure count is always on screen: a failure
// you have to go looking for is the one you ship.

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
  return <Button
    variant="quiet"
    size="xs"
    aria-label={label}
    tooltip={label}
    onClick={() => {
      void navigator.clipboard?.writeText(JSON.stringify(value, null, 2));
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1400);
    }}
  >
    {copied ? <Check className="size-2.5" aria-hidden="true" /> : <Copy className="size-2.5" aria-hidden="true" />}
    {copied ? "Copied" : "Copy"}
  </Button>;
}

export function TranscriptPane({ sessionId, events, entries = [], head, leaves = [], loadOlder, onRevealEntry, exportTranscript = (id) => bridgeApi.exportSessionTranscript(id) }: {
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
  /** Writes the whole durable record to a JSONL file and says where. */
  exportTranscript?: (sessionId: string) => Promise<ExportSessionTranscriptResult>;
}) {
  const [view, setView] = useState<"stream" | "entries">("stream");
  const [loaded, setLoaded] = useState<AgentEvent[]>([]);
  const [loading, setLoading] = useState(false);
  const [filter, setFilter] = useState("");
  const [facet, setFacet] = useState<TranscriptFacet>("all");
  const [selectedId, setSelectedId] = useState<number>();
  const [exportState, setExportState] = useState<
    { status: "idle" } | { status: "running" } | { status: "done"; result: ExportSessionTranscriptResult } | { status: "error"; message: string }
  >({ status: "idle" });
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

  const rows = useMemo(() => buildTranscriptRows(stream), [stream]);
  const counts = useMemo(() => countFacets(rows), [rows]);
  const visible = useMemo(() => filterTranscriptRows(rows, facet, filter), [rows, facet, filter]);
  // A window onto the newest page of a long session can only count the turns
  // it loaded. Boundaries recorded since the ordinal was stamped carry the
  // session's own number; where none does, the pane says the number is
  // relative rather than quietly showing a different one from the export.
  const absoluteTurns = useMemo(() => turnNumbersAreAbsolute(rows, !canLoadEarlier), [rows, canLoadEarlier]);

  const selected = visible.find(row => row.event.id === selectedId);

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
    <div className="flex min-h-11 shrink-0 flex-wrap items-center gap-2 border-b border-border px-2">
      <SegmentedGroup label="Transcript view">
        {(["stream", "entries"] as const).map(option => <SegmentedItem
          key={option}
          active={view === option}
          onClick={() => setView(option)}
        ><span className="capitalize">{option}</span></SegmentedItem>)}
      </SegmentedGroup>
      {view === "stream" && <>
        <Input
          type="search"
          size="sm"
          value={filter}
          onChange={event => setFilter(event.target.value)}
          placeholder="Filter kind or text"
          aria-label="Filter events"
          className="min-w-24 flex-1"
        />
        <CopyJson value={visible.map(row => row.event)} label="Copy the filtered stream as JSON" />
      </>}
      {view === "entries" && head && <span className="ml-auto truncate font-mono text-caption text-muted-foreground">
        head {head.activeEntryId ? shortId(head.activeEntryId) : "—"} · {leaves.length} {leaves.length === 1 ? "leaf" : "leaves"}
      </span>}
      <Button
        variant="quiet"
        size="sm"
        className="shrink-0"
        loading={exportState.status === "running"}
        disabled={exportState.status === "running"}
        onClick={() => {
          setExportState({ status: "running" });
          void exportTranscript(sessionId)
            .then(result => setExportState({ status: "done", result }))
            .catch(cause => setExportState({ status: "error", message: cause instanceof Error ? cause.message : String(cause) }));
        }}
        tooltip="Write the whole durable record to a JSONL file"
      >
        <Download className="size-2.5" aria-hidden="true" />
        Export JSONL
      </Button>
    </div>

    {view === "stream" && <div className="flex shrink-0 flex-wrap items-center gap-1 border-b border-border px-2 py-1.5">
      {TRANSCRIPT_FACETS.map(option => <Button
        key={option}
        variant="chip"
        size="2xs"
        aria-pressed={facet === option}
        onClick={() => setFacet(option)}
      >
        {/* The one place this pane is not achromatic: a failure that reads
            like every other chip is a failure nobody clicks. */}
        <span className={option === "problems" && counts.problems > 0 ? "font-mono text-destructive" : "font-mono"}>{FACET_LABELS[option]}</span>
        <span className="font-mono tabular-nums opacity-70">{counts[option]}</span>
      </Button>)}
    </div>}

    {exportState.status === "done" && <div className="flex shrink-0 flex-wrap items-center gap-2 border-b border-border bg-code px-2 py-1.5 font-mono text-caption">
      <span className="text-muted-foreground">Wrote {exportState.result.lineCount} lines</span>
      <Tip label={exportState.result.path}><span className="min-w-0 flex-1 truncate text-foreground">{exportState.result.path}</span></Tip>
      <Button
        variant="quiet"
        size="xs"
        className="shrink-0"
        onClick={() => void navigator.clipboard?.writeText(exportState.result.path)}
      >
        <Copy className="size-2.5" aria-hidden="true" />
        Copy path
      </Button>
    </div>}
    {exportState.status === "error" && <p className="shrink-0 border-b border-border px-2 py-1.5 font-mono text-caption text-destructive">
      Export failed: {exportState.message}
    </p>}

    {view === "stream" && <div className="min-h-0 flex-1 overflow-y-auto font-mono text-caption leading-loose">
      {canLoadEarlier && <div className="border-b border-border"><RowButton
        size="sm"
        tone="muted"
        shape="flush"
        disabled={loading}
        onClick={() => {
          if (!loadOlder || oldest === undefined) return;
          setLoading(true);
          void loadOlder({ beforeSequence: oldest, tail: false })
            .then(page => setLoaded(current => mergeRaw(current, page)))
            .finally(() => setLoading(false));
        }}
        loading={loading}
      >Load earlier events</RowButton></div>}
      {visible.length === 0 && !loading && <p className="px-2 py-4 text-muted-foreground">
        {stream.length === 0 ? "No events recorded for this session yet." : "Nothing matches the filter."}
      </p>}
      {visible.map(row => <div key={row.event.id} className="border-b border-border/50">
        <RowButton
          size="bare"
          shape="flush"
          onClick={() => setSelectedId(current => current === row.event.id ? undefined : row.event.id)}
          aria-expanded={selectedId === row.event.id}
          active={selectedId === row.event.id}
        ><span className="flex w-full min-w-0 items-baseline gap-2 px-2 text-caption">
          <span className="w-7 shrink-0 tabular-nums text-muted-foreground">{row.event.sequence}</span>
          <Tip label={absoluteTurns
            ? `Turn ${row.turnIndex}`
            : `Turn ${row.turnIndex} of the loaded events — load earlier events for the session's own numbering`}
          ><span className="w-10 shrink-0 tabular-nums text-muted-foreground">{absoluteTurns ? "" : "~"}t{row.turnIndex}</span></Tip>
          <span className={cn("w-[34%] min-w-0 shrink-0 truncate", row.problem ? "text-destructive" : "text-foreground")}>{row.kind}</span>
          {row.problem && <AlertTriangle size={10} className="shrink-0 text-destructive" aria-label={row.problem} />}
          <span className="min-w-0 flex-1 truncate text-muted-foreground">{row.detail}</span>
          <span className="shrink-0 tabular-nums text-muted-foreground">{eventTime(row.event.createdAt)}</span>
        </span></RowButton>
        {selected?.event.id === row.event.id && <div className="border-t border-border bg-code px-2 py-1.5">
          <div className="flex items-center gap-2">
            <span className="text-caption text-muted-foreground font-medium">Raw event</span>
            {row.problem && <span className="text-caption text-destructive">{row.problem}</span>}
            <span className="ml-auto"><CopyJson value={row.event} label={`Copy event ${row.event.sequence} as JSON`} /></span>
          </div>
          <pre className="mt-1 max-h-56 overflow-auto whitespace-pre-wrap break-words text-caption leading-relaxed text-foreground">{JSON.stringify(row.event, null, 2)}</pre>
        </div>}
      </div>)}
    </div>}

    {view === "entries" && <div className="min-h-0 flex-1 overflow-y-auto font-mono text-caption leading-loose">
      {entries.length === 0 && <p className="px-2 py-4 text-muted-foreground">No forest entries for this session yet.</p>}
      {entries.map(entry => <div key={entry.id} className="group flex items-baseline gap-2 border-b border-border/50 px-2 transition-colors hover:bg-accent">
        <span className={cn("w-1.5 shrink-0 self-stretch", activePath.has(entry.id) ? "bg-success/60" : "bg-transparent")} aria-hidden="true" />
        <span className="w-16 shrink-0 truncate text-muted-foreground">{shortId(entry.id)}</span>
        <span className="min-w-0 flex-1 truncate text-foreground">{entry.kind}</span>
        <span className="shrink-0 text-muted-foreground">{entry.contextVisibility}</span>
        {activePath.has(entry.id) && <span className="shrink-0 text-caption text-success font-medium">active</span>}
        {onRevealEntry && <Button
          variant="quiet"
          size="sm"
          className="shrink-0"
          onClick={() => onRevealEntry(entry.id)}
          aria-label={`Reveal entry ${entry.id} in the conversation`}
          tooltip="Reveal in the conversation"
        >
          <Eye className="size-2.5" aria-hidden="true" />
          reveal
        </Button>}
        <ChevronRight size={10} className="shrink-0 text-muted-foreground" aria-hidden="true" />
      </div>)}
    </div>}

    <div className="flex min-h-8 shrink-0 flex-wrap items-center gap-2 border-t border-border px-2 font-mono text-caption text-muted-foreground">
      <span>{view === "stream" ? `${visible.length} of ${stream.length} events` : `${entries.length} entries`}</span>
      {/* Always on screen, whichever facet is selected: a reader who never
          clicks Problems still has to learn that there are some. */}
      {view === "stream" && <span className={cn(counts.problems > 0 && "text-destructive")}>
        {counts.problems} {counts.problems === 1 ? "problem" : "problems"}
      </span>}
      <span className="ml-auto truncate">{sessionId}</span>
    </div>
  </div>;
}
