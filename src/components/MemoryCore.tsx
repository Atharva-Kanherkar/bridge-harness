import { useEffect, useMemo, useRef, useState } from "react";
import { Check, GitBranch, Pencil, Search, Sparkles, Trash2, X } from "lucide-react";
import { bridgeApi } from "../api";
import {
  buildEdges,
  layoutConstellation,
  nodeState,
  searchRecords,
  type EdgeKind,
  type GraphNode,
  type NodeState,
} from "../memoryCore";
import type {
  MemoryConsolidationEntry,
  MemoryCoRecallPair,
  MemoryRecallStats,
  MemoryRecord,
} from "../types";

const SCOPE = "account:local";
const GRAPH_W = 640;
const GRAPH_H = 440;

/** State → accent utility. The hue is never the only signal: pinned also wears
 *  a ring, proposed a dash, superseded a dim + shrink, and every node carries a
 *  `data-state`. See the legend and `nodeGlyph`. */
const STATE_FILL: Record<NodeState, string> = {
  pinned: "fill-mc-life",
  active: "fill-mc-info",
  proposed: "fill-mc-proposed",
  superseded: "fill-mc-text-weak",
  tombstoned: "fill-mc-text-weak",
};
const EDGE_STROKE: Record<EdgeKind, string> = {
  supersedes: "stroke-mc-text-weak",
  conflict: "stroke-mc-conflict",
  corecall: "stroke-mc-info-2",
};
const OP_TONE: Record<MemoryConsolidationEntry["op"], string> = {
  merge: "text-mc-info",
  correct: "text-mc-info-2",
  keep: "text-mc-life",
  group: "text-mc-text-weak",
  expire: "text-mc-text-weak",
  retire: "text-mc-conflict",
};

/**
 * Memory Core — the full-screen memory-graph surface (issue #416). A dark-only
 * surface on the Geist-derived `--color-mc-*` tokens; it does not inherit the
 * app's Graphite & Paper chrome. Read paths run through the mock/derived layer
 * in `api.ts`; every mutation round-trips the existing memory protocol methods.
 */
export function MemoryCore({
  open,
  onClose,
  onError,
}: {
  open: boolean;
  onClose: () => void;
  onError: (message: string) => void;
}) {
  const [records, setRecords] = useState<MemoryRecord[]>();
  const [proposed, setProposed] = useState<MemoryRecord[]>();
  const [stats, setStats] = useState<MemoryRecallStats>();
  const [coRecall, setCoRecall] = useState<MemoryCoRecallPair[]>();
  const [log, setLog] = useState<MemoryConsolidationEntry[]>();
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [editingBody, setEditingBody] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const readGeneration = useRef(0);

  useEffect(() => {
    if (open) return;
    readGeneration.current += 1;
    setRecords(undefined);
    setProposed(undefined);
    setStats(undefined);
    setCoRecall(undefined);
    setLog(undefined);
    setSelectedId(null);
    setQuery("");
    setEditingBody(null);
  }, [open]);

  useEffect(() => {
    if (!open) return;
    let active = true;
    let off: (() => void) | undefined;
    const load = () => {
      const generation = ++readGeneration.current;
      Promise.all([
        bridgeApi.memoryGraphRecords(SCOPE),
        bridgeApi.listMemoryRecords(SCOPE, "proposed"),
        bridgeApi.memoryRecallStats(SCOPE),
        bridgeApi.memoryCoRecallPairs(SCOPE),
        bridgeApi.memoryConsolidationLog(SCOPE),
      ]).then(([graph, proposedList, recallStats, pairs, consolidation]) => {
        if (!active || generation !== readGeneration.current) return;
        setRecords(graph);
        setProposed(proposedList.records);
        setStats(recallStats);
        setCoRecall(pairs);
        setLog(consolidation);
      }).catch(error => {
        if (active && generation === readGeneration.current) onError(String(error));
      });
    };
    load();
    void bridgeApi.onMemoryChanged(() => load()).then(fn => {
      if (!active) { fn(); return; }
      off = fn;
    }).catch(error => { if (active) onError(String(error)); });
    return () => { active = false; off?.(); };
  }, [onError, open]);

  useEffect(() => {
    if (!open) return;
    const onKeyDown = (event: KeyboardEvent) => { if (event.key === "Escape") onClose(); };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [open, onClose]);

  const nodes = useMemo(() => layoutConstellation(records ?? [], GRAPH_W, GRAPH_H), [records]);
  const nodeById = useMemo(() => new Map(nodes.map(node => [node.id, node])), [nodes]);
  const edges = useMemo(() => buildEdges(records ?? [], coRecall ?? []), [records, coRecall]);
  const matchIds = useMemo(() => {
    if (!query.trim()) return null;
    return new Set(searchRecords(records ?? [], query).map(record => record.id));
  }, [records, query]);

  const selected = selectedId ? (records ?? []).find(record => record.id === selectedId) ?? null : null;
  const selectedStat = selected ? stats?.perRecord.find(stat => stat.id === selected.id) : undefined;

  if (!open) return null;

  const act = async (work: () => Promise<unknown>) => {
    setBusy(true);
    try { await work(); }
    catch (error) { onError(error instanceof Error ? error.message : String(error)); }
    finally { setBusy(false); }
  };
  const saveEdit = () => {
    if (!selected || editingBody == null) return;
    void act(async () => {
      await bridgeApi.supersedeMemoryRecord(selected.id, editingBody, selected.kind);
      setEditingBody(null);
      setSelectedId(null);
    });
  };
  const forget = () => {
    if (!selected) return;
    void act(async () => {
      await bridgeApi.deleteMemoryRecord(selected.id);
      setSelectedId(null);
    });
  };

  const cardClass = "rounded-mc border border-mc-border bg-mc-surface";
  const injections = stats?.injectionsPerDay ?? [];
  const peakInjection = injections.length ? Math.max(...injections) : 0;
  const budgetPct = stats ? Math.round((stats.budgetCharsUsed / stats.budgetCharsMax) * 100) : 0;

  return (
    <div
      className="fixed inset-0 z-50 flex flex-col overflow-hidden font-sans text-mc-text mc-hero"
      role="dialog"
      aria-modal="true"
      aria-label="Memory Core"
    >
      {/* Header */}
      <header className="flex shrink-0 items-center gap-3 border-b border-mc-border px-5 py-3">
        <span className="flex h-8 w-8 items-center justify-center rounded-mc-sm border border-mc-border bg-mc-surface text-mc-life">
          <Sparkles size={16} aria-hidden="true" />
        </span>
        <div className="min-w-0">
          <h2 className="font-display text-sm font-semibold leading-tight text-mc-text">Memory Core</h2>
          <p className="font-mono text-[11px] text-mc-text-weak">{SCOPE}</p>
        </div>
        <div className="relative ml-4 w-64 max-w-[40vw]">
          <Search size={13} aria-hidden="true" className="pointer-events-none absolute left-2.5 top-2.5 text-mc-text-weak" />
          <input
            type="text"
            value={query}
            onChange={event => setQuery(event.target.value)}
            placeholder="Search memory…"
            aria-label="Search memory"
            className="w-full rounded-mc-sm border border-mc-border bg-mc-surface py-1.5 pl-8 pr-2 font-mono text-[12px] text-mc-text placeholder:text-mc-text-weak focus:border-mc-border-active focus:outline-none"
          />
        </div>
        <div className="ml-auto flex items-center gap-4 text-[11px] text-mc-text-weak">
          <Legend />
          <button
            type="button"
            onClick={onClose}
            aria-label="Close"
            className="rounded-mc-sm border border-mc-border p-1.5 text-mc-text-weak transition-colors hover:border-mc-border-active hover:text-mc-text"
          ><X size={15} aria-hidden="true" /></button>
        </div>
      </header>

      {/* Body */}
      <div className="grid min-h-0 flex-1 grid-cols-[minmax(0,1fr)_340px] overflow-hidden">
        {/* Constellation + analytics */}
        <div className="flex min-h-0 flex-col overflow-hidden">
          <div className="relative min-h-0 flex-1">
            <div className="absolute inset-0 mc-grid" aria-hidden="true" />
            <svg
              viewBox={`0 0 ${GRAPH_W} ${GRAPH_H}`}
              className="absolute inset-0 h-full w-full"
              preserveAspectRatio="xMidYMid meet"
              role="img"
              aria-label="Memory constellation"
            >
              <defs>
                <marker id="mc-arrow" viewBox="0 0 8 8" refX="7" refY="4" markerWidth="6" markerHeight="6" orient="auto-start-reverse">
                  <path d="M0 0 L8 4 L0 8 z" className="fill-mc-text-weak" />
                </marker>
              </defs>
              {edges.map((edge, index) => {
                const a = nodeById.get(edge.from);
                const b = nodeById.get(edge.to);
                if (!a || !b) return null;
                const dimmed = matchIds ? !(matchIds.has(edge.from) && matchIds.has(edge.to)) : false;
                return (
                  <line
                    key={`${edge.kind}-${index}`}
                    x1={a.x} y1={a.y} x2={b.x} y2={b.y}
                    className={EDGE_STROKE[edge.kind]}
                    strokeWidth={edge.kind === "corecall" ? Math.min(3, 0.6 + (edge.weight ?? 1) * 0.25) : 1.4}
                    strokeOpacity={dimmed ? 0.08 : edge.kind === "corecall" ? 0.32 : 0.55}
                    strokeDasharray={edge.kind === "conflict" ? "2 4" : edge.kind === "corecall" ? "1 5" : undefined}
                    markerEnd={edge.kind === "supersedes" ? "url(#mc-arrow)" : undefined}
                  />
                );
              })}
              {nodes.map(node => (
                <MemoryNode
                  key={node.id}
                  node={node}
                  selected={node.id === selectedId}
                  dimmed={matchIds ? !matchIds.has(node.id) : false}
                  onSelect={() => { setSelectedId(node.id); setEditingBody(null); }}
                />
              ))}
            </svg>
            <p className="pointer-events-none absolute bottom-3 left-4 font-mono text-[10px] text-mc-text-weak">
              {nodes.length} nodes · {edges.length} edges · radius = confidence
            </p>
          </div>
          {/* Analytics strip */}
          <div className="grid shrink-0 grid-cols-2 gap-3 border-t border-mc-border p-4">
            <div className={`${cardClass} p-3`}>
              <p className="text-[11px] font-medium text-mc-text-weak">Recall activity · 14d</p>
              <Sparkbars values={injections} className="mt-2" tone="bg-mc-life" />
              <p className="mt-1 font-mono text-[10px] text-mc-text-weak">peak {peakInjection} injections / day</p>
            </div>
            <div className={`${cardClass} p-3`}>
              <p className="text-[11px] font-medium text-mc-text-weak">Packet budget</p>
              <div className="mt-2 h-2 w-full overflow-hidden rounded-full bg-mc-surface-active">
                <div
                  className={`h-full rounded-full ${budgetPct > 90 ? "bg-mc-conflict" : "bg-mc-info"}`}
                  style={{ width: `${budgetPct}%` }}
                />
              </div>
              <p className="mt-1 font-mono text-[10px] text-mc-text-weak">
                {stats?.budgetCharsUsed ?? 0} / {stats?.budgetCharsMax ?? 0} chars · refuses at capacity, never evicts
              </p>
            </div>
          </div>
        </div>

        {/* Right rail: inspector + queue + consolidation */}
        <aside className="flex min-h-0 flex-col gap-3 overflow-y-auto border-l border-mc-border bg-mc-bg/60 p-4">
          <Inspector
            record={selected}
            records={records ?? []}
            stat={selectedStat}
            editingBody={editingBody}
            busy={busy}
            onEdit={() => selected && setEditingBody(selected.body)}
            onEditChange={setEditingBody}
            onSaveEdit={saveEdit}
            onCancelEdit={() => setEditingBody(null)}
            onForget={forget}
          />

          <section className={`${cardClass} p-3`}>
            <p className="text-[11px] font-medium text-mc-text-weak">Review queue · proposed</p>
            <ul className="mt-2 space-y-2">
              {(proposed ?? []).map(record => (
                <li key={record.id} className="rounded-mc-sm border border-mc-border bg-mc-surface-hover p-2.5">
                  <p className="text-[12px] leading-snug text-mc-text">{record.body}</p>
                  <p className="mt-1 font-mono text-[10px] text-mc-proposed">
                    {record.kind}{record.confidenceBps != null && <> · {Math.round(record.confidenceBps / 100)}%</>}{record.rationale && <> · {record.rationale}</>}
                  </p>
                  <div className="mt-2 flex gap-1.5">
                    <button
                      type="button"
                      disabled={busy}
                      onClick={() => void act(() => bridgeApi.approveMemoryRecord(record.id))}
                      className="inline-flex items-center gap-1 rounded-mc-sm bg-mc-life px-2 py-1 text-[11px] font-medium text-mc-bg transition-opacity hover:opacity-90 disabled:opacity-45"
                    ><Check size={12} aria-hidden="true" />Accept</button>
                    <button
                      type="button"
                      disabled={busy}
                      onClick={() => void act(() => bridgeApi.rejectMemoryRecord(record.id))}
                      className="rounded-mc-sm border border-mc-border px-2 py-1 text-[11px] font-medium text-mc-text-weak transition-colors hover:border-mc-border-active hover:text-mc-text disabled:opacity-45"
                    >Reject</button>
                  </div>
                </li>
              ))}
              {proposed !== undefined && proposed.length === 0 && (
                <li className="rounded-mc-sm border border-dashed border-mc-border px-3 py-4 text-center text-[11px] text-mc-text-weak">
                  Nothing to review.
                </li>
              )}
            </ul>
          </section>

          <section className={`${cardClass} p-3`}>
            <p className="text-[11px] font-medium text-mc-text-weak">Consolidation log</p>
            <ul className="mt-2 space-y-1.5">
              {(log ?? []).map((entry, index) => (
                <li key={index} className="flex items-baseline gap-2 text-[11px]">
                  <span className={`font-mono uppercase ${OP_TONE[entry.op]}`}>{entry.op}</span>
                  <span className="min-w-0 flex-1 truncate text-mc-text-weak" title={entry.detail}>{entry.detail}</span>
                  <span className="font-mono text-[10px] text-mc-text-weak">{13 - entry.day}d</span>
                </li>
              ))}
            </ul>
          </section>
        </aside>
      </div>
    </div>
  );
}

/** One constellation node: circle sized by confidence, plus the state's own
 *  non-color glyph — a ring for pinned, a dashed stroke for proposed, a dim
 *  shrink for superseded. */
function MemoryNode({ node, selected, dimmed, onSelect }: {
  node: GraphNode;
  selected: boolean;
  dimmed: boolean;
  onSelect: () => void;
}) {
  const state = node.state;
  const baseOpacity = state === "superseded" ? 0.4 : 1;
  return (
    <g
      className="cursor-pointer"
      opacity={dimmed ? 0.15 : baseOpacity}
      onClick={onSelect}
      role="button"
      aria-label={`memory ${node.id}`}
      data-node={node.id}
      data-state={state}
    >
      {selected && (
        <circle cx={node.x} cy={node.y} r={node.r + 6} className="fill-none stroke-mc-text" strokeWidth={1} strokeDasharray="2 3" />
      )}
      {state === "pinned" && (
        <circle cx={node.x} cy={node.y} r={node.r + 3} className="fill-none stroke-mc-life" strokeWidth={1.4} strokeOpacity={0.7} />
      )}
      <circle
        cx={node.x}
        cy={node.y}
        r={node.r}
        className={`${STATE_FILL[state]} ${state === "proposed" ? "stroke-mc-proposed" : "stroke-none"}`}
        fillOpacity={state === "superseded" ? 0.5 : state === "proposed" ? 0.35 : 0.9}
        strokeWidth={state === "proposed" ? 1.6 : 0}
        strokeDasharray={state === "proposed" ? "3 3" : undefined}
      />
    </g>
  );
}

/** The selected record in full: every `memory_records` field, lineage, and a
 *  recall sparkline. Edit supersedes; Forget tombstones — both through the
 *  existing protocol methods. */
function Inspector({ record, records, stat, editingBody, busy, onEdit, onEditChange, onSaveEdit, onCancelEdit, onForget }: {
  record: MemoryRecord | null;
  records: MemoryRecord[];
  stat: MemoryRecallStats["perRecord"][number] | undefined;
  editingBody: string | null;
  busy: boolean;
  onEdit: () => void;
  onEditChange: (value: string) => void;
  onSaveEdit: () => void;
  onCancelEdit: () => void;
  onForget: () => void;
}) {
  const cardClass = "rounded-mc border border-mc-border bg-mc-surface";
  if (!record) {
    return (
      <section className={`${cardClass} flex items-center justify-center p-6 text-center text-[12px] text-mc-text-weak`}>
        Select a node to inspect its memory — body, provenance, lineage, and recall history.
      </section>
    );
  }
  const state = nodeState(record);
  const lineage: MemoryRecord[] = [];
  let cursor: MemoryRecord | undefined = record;
  const seen = new Set<string>();
  while (cursor && !seen.has(cursor.id)) {
    lineage.unshift(cursor);
    seen.add(cursor.id);
    cursor = cursor.supersedes ? records.find(item => item.id === cursor!.supersedes) : undefined;
  }
  const conf = record.confidenceBps != null ? Math.round(record.confidenceBps / 100) : null;
  return (
    <section className={`${cardClass} p-3`}>
      <div className="flex items-center gap-2">
        <span className="font-mono text-[11px] text-mc-text">{record.id}</span>
        <span className="rounded-full border border-mc-border px-1.5 py-0.5 font-mono text-[9px] uppercase text-mc-text-weak" data-state={state}>{state}</span>
      </div>
      {editingBody == null ? (
        <p className="mt-2 whitespace-pre-wrap text-[13px] leading-snug text-mc-text">{record.body}</p>
      ) : (
        <textarea
          value={editingBody}
          onChange={event => onEditChange(event.target.value)}
          aria-label="Edit memory body"
          className="mt-2 min-h-20 w-full rounded-mc-sm border border-mc-border bg-mc-surface-hover p-2 text-[13px] text-mc-text focus:border-mc-border-active focus:outline-none"
        />
      )}
      <dl className="mt-3 grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 font-mono text-[10px] text-mc-text-weak">
        <dt>kind</dt><dd className="text-mc-text">{record.kind}</dd>
        <dt>provenance</dt><dd className="text-mc-text">{record.provenance}</dd>
        {conf != null && <><dt>confidence</dt><dd className="text-mc-text">{conf}% · {record.confidenceBps} bps</dd></>}
        {record.sourceSessionId && <><dt>source</dt><dd className="truncate text-mc-text" title={record.sourceSessionId}>{record.sourceSessionId}</dd></>}
        {record.conflictGroup && <><dt>conflict</dt><dd className="text-mc-conflict">{record.conflictGroup}</dd></>}
        <dt>created</dt><dd className="text-mc-text">{new Date(record.createdAt).toLocaleDateString()}</dd>
      </dl>

      {lineage.length > 1 && (
        <div className="mt-3">
          <p className="flex items-center gap-1 text-[10px] font-medium text-mc-text-weak"><GitBranch size={11} aria-hidden="true" /> lineage · as-of history</p>
          <ol className="mt-1 space-y-0.5">
            {lineage.map((item, index) => (
              <li key={item.id} className="flex items-center gap-1.5 font-mono text-[10px]">
                <span className={index === lineage.length - 1 ? "text-mc-life" : "text-mc-text-weak line-through"}>{item.id}</span>
                {index < lineage.length - 1 && <span className="text-mc-text-weak">→</span>}
              </li>
            ))}
          </ol>
        </div>
      )}

      {stat && (
        <div className="mt-3">
          <p className="text-[10px] font-medium text-mc-text-weak">recalls · 14d</p>
          <Sparkbars values={stat.daily} className="mt-1" tone="bg-mc-life" />
          <p className="mt-1 font-mono text-[10px] text-mc-text-weak">
            {stat.recalls} recalls · {Math.round(stat.inPacketRatio * 100)}% in-packet · last {stat.lastRecalledDay < 0 ? "never" : `${13 - stat.lastRecalledDay}d ago`}
          </p>
        </div>
      )}

      {state !== "superseded" && state !== "tombstoned" && (
        <div className="mt-3 flex gap-1.5">
          {editingBody == null ? (
            <>
              <button
                type="button"
                disabled={busy}
                onClick={onEdit}
                className="inline-flex items-center gap-1 rounded-mc-sm border border-mc-border px-2 py-1 text-[11px] text-mc-text-weak transition-colors hover:border-mc-border-active hover:text-mc-text disabled:opacity-45"
              ><Pencil size={12} aria-hidden="true" />Edit</button>
              <button
                type="button"
                disabled={busy}
                onClick={onForget}
                className="inline-flex items-center gap-1 rounded-mc-sm border border-mc-border px-2 py-1 text-[11px] text-mc-conflict transition-colors hover:border-mc-conflict disabled:opacity-45"
              ><Trash2 size={12} aria-hidden="true" />Forget</button>
            </>
          ) : (
            <>
              <button
                type="button"
                disabled={busy}
                onClick={onSaveEdit}
                className="rounded-mc-sm bg-mc-life px-2.5 py-1 text-[11px] font-medium text-mc-bg transition-opacity hover:opacity-90 disabled:opacity-45"
              >Save edit</button>
              <button
                type="button"
                disabled={busy}
                onClick={onCancelEdit}
                className="rounded-mc-sm border border-mc-border px-2.5 py-1 text-[11px] text-mc-text-weak transition-colors hover:text-mc-text disabled:opacity-45"
              >Cancel</button>
            </>
          )}
        </div>
      )}
    </section>
  );
}

/** A tiny bar sparkline. `values` are non-negative counts; the tallest bar is
 *  full height. Pure presentation, no axis. */
function Sparkbars({ values, tone, className }: { values: number[]; tone: string; className?: string }) {
  const max = values.length ? Math.max(1, ...values) : 1;
  return (
    <div className={`flex h-8 items-end gap-0.5 ${className ?? ""}`}>
      {values.map((value, index) => (
        <div
          key={index}
          className={`min-h-px flex-1 rounded-sm ${tone}`}
          style={{ height: `${Math.round((value / max) * 100)}%` }}
          aria-hidden="true"
        />
      ))}
    </div>
  );
}

function Legend() {
  const items: { label: string; className: string; glyph: "solid" | "ring" | "dash" | "dim" }[] = [
    { label: "pinned", className: "bg-mc-life", glyph: "ring" },
    { label: "active", className: "bg-mc-info", glyph: "solid" },
    { label: "proposed", className: "bg-mc-proposed", glyph: "dash" },
    { label: "superseded", className: "bg-mc-text-weak", glyph: "dim" },
  ];
  return (
    <div className="hidden items-center gap-3 lg:flex">
      {items.map(item => (
        <span key={item.label} className="flex items-center gap-1 font-mono text-[10px] text-mc-text-weak">
          <span
            className={`h-2.5 w-2.5 rounded-full ${item.className} ${item.glyph === "ring" ? "ring-1 ring-mc-life ring-offset-1 ring-offset-mc-bg" : ""} ${item.glyph === "dash" ? "border border-dashed border-mc-proposed bg-transparent" : ""} ${item.glyph === "dim" ? "opacity-40" : ""}`}
            aria-hidden="true"
          />
          {item.label}
        </span>
      ))}
    </div>
  );
}
