import { SCREEN_CONTENT, ScreenHeading } from "./ui/screen";
import { pendingClass, SkeletonRows } from "@/components/ui/skeleton";
import { useEffect, useRef, useState } from "react";
import { Check, Pencil, Search, X } from "lucide-react";
import { cn } from "@/lib/utils";
import { bridgeApi, MEMORY_PAGE_SIZE } from "../api";
import { searchRecords } from "../memoryStats";
import { harnessLabel } from "../utils";
import { relative } from "./UsageInsights";
import { MemoryInsights } from "./MemoryInsights";
import type {
  AdapterDescriptor,
  MemoryCapabilities,
  MemoryActivityEntry,
  MemoryExtractionSettings,
  MemoryModelSettings,
  MemoryModelSource,
  MemoryRecallStats,
  MemoryRecord,
} from "../types";

const KINDS = ["preference", "fact", "decision", "constraint"] as const;
type Kind = typeof KINDS[number];
const KIND_LABEL: Record<Kind, string> = { preference: "Preference", fact: "Fact", decision: "Decision", constraint: "Constraint" };
const KIND_PLURAL: Record<Kind, string> = { preference: "Preferences", fact: "Facts", decision: "Decisions", constraint: "Constraints" };

/** Where the memory model came from: the tail of the sentence naming the model in use. */
const MEMORY_MODEL_SOURCE: Record<MemoryModelSource, string> = {
  user: "your pick",
  chat_default: "your new-chat model",
  standard: "Bridge's standard choice",
};

/** A model's label from the adapter list, else its id, else the harness default. */
function memoryModelLabel(adapters: AdapterDescriptor[], harness: string, model?: string | null): string {
  const adapter = adapters.find(item => item.id === harness);
  const harnessName = adapter?.label ?? harnessLabel(harness);
  if (!model) return `${harnessName}'s default model`;
  return adapter?.models.find(item => item.id === model)?.label ?? model;
}

/** What each extraction mode does after a chat turn, said once beside its choice. */
const MODES = [
  { id: "remember", label: "Manual only", description: "No extraction runs; existing active memory stays active, and new memory is saved only when you ask." },
  { id: "propose", label: "Review first", description: "Each validated extraction that fits the memory budget waits below for your decision." },
  { id: "auto_apply", label: "Automatic", description: "Promotes only 90%+ candidates that cite and match a stable user message from the same chat, with no likely conflict or forgotten match; other validated candidates that fit the budget stay here for review." },
] as const;
/** Mirrors the contract cap in bridge-protocol's memory messages. */
export const MAX_MEMORY_BODY_CHARS = 4000;
/** Mirrors `MAX_MEMORY_REVIEW_BATCH`: the most ids one review call may decide. */
const MEMORY_REVIEW_BATCH = 200;

/**
 * How long a body is *to the ledger*: trimmed, counted in code points. A
 * JavaScript `.length` counts UTF-16 units, so an emoji would score two and a
 * trailing newline would score at all — and the surface would refuse bodies
 * the server accepts. Mirrors `require_body` in memory_ledger.
 */
export function bodyLength(text: string): number {
  return [...text.trim()].length;
}

type MemoryStatus = "active" | "proposed";
/** What one memory list holds: the pages read so far, where the next starts, and the true total. */
type MemoryList = { records: MemoryRecord[]; total: number; cursor?: string };

/** Where a record sits in core's order: newest change first, then id. */
type ListPosition = Pick<MemoryRecord, "updatedAt" | "id">;

/** Whether `record` sorts at or after `position`, in core's descending order. */
function atOrPast(record: ListPosition, position: ListPosition): boolean {
  return record.updatedAt < position.updatedAt || (record.updatedAt === position.updatedAt && record.id <= position.id);
}

/**
 * Read a list's pages until at least `want` records are loaded and, when
 * `through` is given, the read has reached that record's position, or the
 * list ends. A reload after a change passes the record that was last on
 * screen: records that arrived at the top would otherwise push loaded ones
 * onto an unread page, out of view and out of the selection.
 */
async function readMemoryPages(status: MemoryStatus, want: number, through?: ListPosition): Promise<MemoryList> {
  const records: MemoryRecord[] = [];
  let cursor: string | undefined;
  let total = 0;
  const reached = () => {
    const last = records[records.length - 1];
    return !through || (last !== undefined && atOrPast(last, through));
  };
  do {
    const page = await bridgeApi.listMemoryRecords("account:local", status === "active" ? undefined : status, cursor ? { cursor } : {});
    records.push(...page.records);
    total = page.total ?? records.length;
    cursor = page.nextCursor ?? undefined;
  } while (cursor && (records.length < want || !reached()));
  return { records, total, cursor };
}

/**
 * Account memory on this machine (`account:local`) — the one and only memory
 * surface. Deliberately not this chat's history and not the helper picker —
 * the header says so, because the one-dialog-three-products confusion is the
 * bug this surface exists to fix. The Activity tab carries the read-only
 * recall analytics (injections, how much of the ledger is used, packet budget, recent runs) that used to
 * live on a separate full-screen surface; one product, one UI.
 */
export function MemoryDialog({
  open,
  initialBody,
  adapters = [],
  onClose,
  onError,
}: {
  open: boolean;
  /** A draft from "Remember this" on a reply, to cut down; never auto-saved. */
  initialBody?: string | null;
  adapters?: AdapterDescriptor[];
  onClose: () => void;
  onError: (message: string) => void;
}) {
  const [tab, setTab] = useState<"pins" | "queue" | "activity" | "insights">("pins");
  const [activeList, setActiveList] = useState<MemoryList>();
  const [proposedList, setProposedList] = useState<MemoryList>();
  // Proposals ticked for one bulk decision. Pruned whenever the queue reloads.
  const [selected, setSelected] = useState<ReadonlySet<string>>(new Set());
  const [loadingMore, setLoadingMore] = useState<MemoryStatus | null>(null);
  const [settings, setSettings] = useState<MemoryExtractionSettings>();
  const [memoryModel, setMemoryModel] = useState<MemoryModelSettings>();
  const [budget, setBudget] = useState<{ held: number; max: number }>();
  const [capabilities, setCapabilities] = useState<MemoryCapabilities>();
  const [injection, setInjection] = useState<boolean>();
  const [stats, setStats] = useState<MemoryRecallStats>();
  const [log, setLog] = useState<MemoryActivityEntry[]>();
  const [filter, setFilter] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [body, setBody] = useState("");
  const [kind, setKind] = useState<string>("preference");
  // Edit is supersession: the row's body moves here, and saving writes a
  // superseding record — never save-then-forget, never an in-place update.
  const [editingId, setEditingId] = useState<string | null>(null);
  // A harness picked with no model yet. The pickers otherwise always show the
  // stored pin, so a pin changed or cleared elsewhere shows up on the next read.
  const [modelDraft, setModelDraft] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  // Which read is the newest. The initial load and every memory-changed hint
  // read, so a slow earlier call must not land over a fresher one. Same shape
  // as RouterSettingsDialog's learningReadGeneration.
  const readGeneration = useRef(0);
  // Bumped by every memory-model write that succeeds. A read that began before
  // one must not put back the model that write replaced.
  const modelRevision = useRef(0);
  const editorRef = useRef<HTMLTextAreaElement>(null);
  // How many records each list shows, and the last one on screen, read by the
  // change-driven reload.
  const loadedCount = useRef<Record<MemoryStatus, number>>({ active: 0, proposed: 0 });
  const lastLoaded = useRef<Partial<Record<MemoryStatus, ListPosition>>>({});
  // A page that failed to load is not retried by the automatic read below.
  const failedCursor = useRef<string | undefined>(undefined);
  const records = activeList?.records;
  const proposed = proposedList?.records;

  useEffect(() => {
    loadedCount.current = { active: activeList?.records.length ?? 0, proposed: proposedList?.records.length ?? 0 };
    lastLoaded.current = { active: activeList?.records.at(-1), proposed: proposedList?.records.at(-1) };
  }, [activeList, proposedList]);

  /** Append the next page of one list. A reload that lands meanwhile wins. */
  const showMore = async (status: MemoryStatus) => {
    const list = status === "active" ? activeList : proposedList;
    const cursor = list?.cursor;
    if (!cursor) return;
    const generation = readGeneration.current;
    setLoadingMore(status);
    try {
      const page = await bridgeApi.listMemoryRecords("account:local", status === "active" ? undefined : status, { cursor });
      if (generation !== readGeneration.current) return;
      (status === "active" ? setActiveList : setProposedList)(current => current?.cursor !== cursor ? current : {
        records: [...current.records, ...page.records],
        total: page.total ?? current.total,
        cursor: page.nextCursor ?? undefined,
      });
    } catch (error) {
      failedCursor.current = cursor;
      if (generation === readGeneration.current) onError(error instanceof Error ? error.message : String(error));
    } finally {
      setLoadingMore(current => current === status ? null : current);
    }
  };

  const searching = query.trim() !== "" || filter !== null;
  const activeCursor = activeList?.cursor;
  useEffect(() => {
    // Search and the kind filter run over loaded records, so a narrowed view
    // reads the rest of the list first rather than miss what is unloaded.
    if (open && searching && activeCursor && activeCursor !== failedCursor.current && loadingMore === null) void showMore("active");
    // showMore is a fresh closure each render; the cursor is what drives this.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, searching, activeCursor, loadingMore]);

  useEffect(() => {
    if (open) return;
    // A closed dialog keeps no state, and an in-flight read must not land on it.
    readGeneration.current += 1;
    setTab("pins");
    setActiveList(undefined);
    setProposedList(undefined);
    setSelected(new Set());
    setLoadingMore(null);
    loadedCount.current = { active: 0, proposed: 0 };
    lastLoaded.current = {};
    setSettings(undefined);
    setBudget(undefined);
    setCapabilities(undefined);
    setInjection(undefined);
    setStats(undefined);
    setLog(undefined);
    setFilter(null);
    setQuery("");
    setBody("");
    setKind("preference");
    setEditingId(null);
    setModelDraft(null);
    setMemoryModel(undefined);
  }, [open]);

  useEffect(() => {
    if (open && initialBody) setBody(initialBody);
  }, [open, initialBody]);

  useEffect(() => {
    if (!open) return;
    let active = true;
    let off: (() => void) | undefined;
    const load = () => {
      const generation = ++readGeneration.current;
      const revision = modelRevision.current;
      Promise.all([
        readMemoryPages("active", loadedCount.current.active, lastLoaded.current.active),
        readMemoryPages("proposed", loadedCount.current.proposed, lastLoaded.current.proposed),
        bridgeApi.getExtractionSettings(),
        bridgeApi.getMemoryInjection(),
        bridgeApi.memoryRecallStats("account:local"),
        bridgeApi.memoryActivityLog("account:local"),
        bridgeApi.getConsolidationSettings(),
        bridgeApi.getMemoryModel(),
      ]).then(([activeRead, proposedRead, extraction, injectionSettings, recallStats, activity, consolidation, model]) => {
        if (!active || generation !== readGeneration.current) return;
        setActiveList(activeRead);
        setBudget({ held: consolidation.heldRecords, max: consolidation.maxRecords });
        setProposedList(proposedRead);
        setSelected(current => {
          const waiting = new Set(proposedRead.records.map(record => record.id));
          return new Set([...current].filter(id => waiting.has(id)));
        });
        setSettings(extraction);
        setInjection(injectionSettings.enabled);
        setStats(recallStats);
        setLog(activity);
        if (revision === modelRevision.current) setMemoryModel(model);
      }).catch(error => {
        // The generation gate covers the failure path too: a slow read that
        // errors after a newer one rendered must not toast over it.
        if (active && generation === readGeneration.current) onError(String(error));
      });
    };
    load();
    bridgeApi.getMemoryCapabilities().then(value => { if (active) setCapabilities(value); })
      .catch(error => { if (active) onError(String(error)); });
    void bridgeApi.onMemoryChanged(() => load()).then(fn => {
      if (!active) { fn(); return; }
      off = fn;
    }).catch(error => { if (active) onError(String(error)); });
    return () => { active = false; off?.(); };
  }, [onError, open]);

  useEffect(() => {
    if (!open) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [open, onClose]);

  if (!open) return null;

  const trimmed = body.trim();
  const bodyChars = bodyLength(body);
  const overLimit = bodyChars > MAX_MEMORY_BODY_CHARS;
  const visible = searchRecords(records ?? [], query).filter(record => !filter || record.kind === filter);
  const queueCount = proposedList?.total ?? 0;
  const full = budget !== undefined && budget.held >= budget.max;
  const fromReply = Boolean(initialBody) && editingId === null && body.length > 0;
  const statById = new Map((stats?.perRecord ?? []).map(stat => [stat.id, stat]));
  const injections = stats?.injectionsPerDay ?? [];
  const peakInjections = injections.length ? Math.max(...injections) : 0;
  const usedPct = stats && stats.activeRecords > 0 ? Math.round((stats.recalledRecords / stats.activeRecords) * 100) : null;
  const budgetPct = stats && stats.budgetCharsMax > 0 ? Math.round((stats.budgetCharsUsed / stats.budgetCharsMax) * 100) : 0;

  const act = async (work: () => Promise<unknown>) => {
    setBusy(true);
    try { await work(); }
    catch (error) { onError(error instanceof Error ? error.message : String(error)); }
    finally { setBusy(false); }
  };
  /** One decision for every ticked proposal, in batches core accepts. */
  const review = (approving: boolean) => act(async () => {
    const ids = [...selected];
    const refused: string[] = [];
    for (let start = 0; start < ids.length; start += MEMORY_REVIEW_BATCH) {
      const batch = ids.slice(start, start + MEMORY_REVIEW_BATCH);
      const result = await bridgeApi.reviewMemoryRecords(approving ? batch : [], approving ? [] : batch);
      refused.push(...result.refused.map(refusal => refusal.reason));
    }
    setSelected(new Set());
    if (refused.length) onError(`${refused.length} of ${ids.length} could not be ${approving ? "approved" : "rejected"}: ${refused[0]}`);
  });
  const toggleSelected = (id: string) => setSelected(current => {
    const next = new Set(current);
    if (next.has(id)) next.delete(id); else next.add(id);
    return next;
  });
  const save = () => act(async () => {
    if (editingId) {
      await bridgeApi.supersedeMemoryRecord(editingId, body, kind);
      setEditingId(null);
    } else {
      await bridgeApi.saveMemoryRecord(body, kind, undefined);
    }
    setBody("");
  });
  const beginEdit = (record: MemoryRecord) => {
    setEditingId(record.id);
    setBody(record.body);
    setKind(record.kind);
    editorRef.current?.focus();
    editorRef.current?.scrollIntoView?.({ block: "center", behavior: "instant" });
  };
  const cancelEdit = () => {
    setEditingId(null);
    setBody("");
    setKind("preference");
  };
  const setMode = (mode: string) => act(async () => {
    setSettings(await bridgeApi.updateExtractionSettings(mode));
  });
  /** Store a pin, or clear it with neither half, and show the stored answer. */
  const saveMemoryModel = (harness?: string, model?: string) => act(async () => {
    try {
      const next = harness && model ? await bridgeApi.setMemoryModel(harness, model) : await bridgeApi.setMemoryModel();
      modelRevision.current += 1;
      setMemoryModel(next);
    } finally {
      // Saved or refused, the pickers go back to what is stored.
      setModelDraft(null);
    }
  });
  // A pin names both halves, so picking a harness only drafts it until a model
  // is chosen; "Use my new-chat model" clears the pin at once.
  const pickMemoryHarness = (harness: string) => {
    if (harness) {
      setModelDraft(harness);
    } else if (memoryModel?.pinned) {
      void saveMemoryModel();
    } else {
      setModelDraft(null);
    }
  };
  const pickMemoryModel = (model: string) => {
    if (pickedHarness && model) void saveMemoryModel(pickedHarness, model);
  };

  const fieldClass = "rounded-lg border border-border-card bg-card text-foreground outline-none transition-colors placeholder:text-faint focus:border-ring/60 disabled:opacity-45";
  const quietSelect = "h-7 min-w-0 rounded-lg border border-transparent bg-transparent px-1.5 text-caption text-muted-foreground outline-none transition-colors hover:text-foreground focus:border-border-card disabled:opacity-45";
  const iconButton = "grid size-7 shrink-0 place-items-center rounded-lg text-muted-foreground transition-colors hover:bg-accent hover:text-foreground disabled:opacity-45";
  const pickedHarness = modelDraft ?? memoryModel?.pinned?.harness ?? "";
  const pickedModel = modelDraft === null ? memoryModel?.pinned?.model ?? "" : "";
  const models = adapters.find(adapter => adapter.id === pickedHarness)?.models ?? [];
  const composing = editingId !== null || body.length > 0;
  const memoryCount = activeList?.total ?? 0;
  const allShownSelected = (proposed?.length ?? 0) > 0 && (proposed ?? []).every(record => selected.has(record.id));
  const totalInjections = injections.reduce((sum, value) => sum + value, 0);

  return <div className="h-full overflow-y-auto" aria-labelledby="memory-title">
    <div className={SCREEN_CONTENT}>
      <ScreenHeading
        id="memory-title"
        title="Memory"
        action={<label className="flex cursor-pointer items-center gap-2.5 text-caption text-muted-foreground">
          Use in new chats
          <input
            type="checkbox"
            role="switch"
            className="peer sr-only"
            checked={injection ?? true}
            disabled={busy || injection === undefined}
            aria-label="Use active memory in new chats"
            onChange={event => {
              const next = event.target.checked;
              void act(async () => {
                const applied = await bridgeApi.setMemoryInjection(next);
                setInjection(applied.enabled);
              });
            }}
          />
          <span aria-hidden="true" className="relative h-[18px] w-8 shrink-0 rounded-full bg-foreground/15 transition-colors after:absolute after:left-[2px] after:top-[2px] after:size-[14px] after:rounded-full after:bg-background after:transition-[left] after:duration-150 peer-checked:bg-foreground peer-checked:after:left-4 peer-focus-visible:ring-2 peer-focus-visible:ring-ring peer-disabled:opacity-40" />
        </label>}
      />
      <div className="u-segmented mb-6 max-w-full" role="group" aria-label="Memory views">
        <button type="button" aria-pressed={tab === "pins"} data-active={tab === "pins"} className="u-segmented-item" onClick={() => setTab("pins")}>
          Memories{memoryCount > 0 && <span className="ml-1.5 tabular-nums text-faint">{memoryCount}</span>}
        </button>
        <button type="button" aria-pressed={tab === "queue"} data-active={tab === "queue"} className="u-segmented-item" onClick={() => setTab("queue")}>
          Review{queueCount > 0 && <span className="ml-1.5 tabular-nums text-faint">{queueCount}</span>}
        </button>
        <button type="button" aria-pressed={tab === "activity"} data-active={tab === "activity"} className="u-segmented-item" onClick={() => setTab("activity")}>Activity</button>
        <button type="button" aria-pressed={tab === "insights"} data-active={tab === "insights"} className="u-segmented-item" onClick={() => setTab("insights")}>Insights</button>
      </div>
      {full && budget && <div role="status" className="mb-6 flex flex-wrap items-center gap-3 rounded-xl border border-border-card bg-card px-4 py-3">
        <p className="min-w-0 flex-1 basis-64 text-caption text-muted-foreground">
          <span className="font-medium text-foreground">Memory is full.</span>{" "}
          It holds <span className="tabular-nums">{budget.held}</span> of <span className="tabular-nums">{budget.max}</span> records, counting proposals, so nothing new is saved or proposed and no extraction runs. Review proposals or forget memories you no longer need.
        </p>
        {queueCount > 0 && tab !== "queue" && <button
          type="button"
          className="h-7 shrink-0 rounded-lg px-2.5 text-caption font-medium text-foreground transition-colors hover:bg-accent"
          onClick={() => setTab("queue")}
        >Review {queueCount}</button>}
      </div>}
      {tab === "pins" && <div>
        <section aria-label="Memory editor" className="overflow-hidden rounded-xl border border-border-card bg-card transition-colors focus-within:border-ring/60">
          {editingId && <p className="px-4 pt-3 text-caption text-muted-foreground">Editing memory</p>}
          <textarea
            ref={editorRef}
            rows={composing ? 3 : 1}
            className="block w-full resize-none bg-transparent px-4 py-3 text-ui leading-relaxed text-foreground outline-none placeholder:text-faint disabled:opacity-45"
            placeholder="Add something Bridge should remember about you or how you work"
            value={body}
            disabled={busy}
            onChange={event => setBody(event.target.value)}
            onKeyDown={event => {
              if (event.key === "Enter" && (event.metaKey || event.ctrlKey) && trimmed && !overLimit && !busy) {
                event.preventDefault();
                void save();
              }
            }}
            aria-label="New memory"
          />
          {composing && <div className="flex flex-wrap items-center gap-2 border-t border-border px-2.5 py-2">
            <select className={quietSelect} value={kind} disabled={busy} onChange={event => setKind(event.target.value)} aria-label="Kind">
              {KINDS.map(item => <option key={item} value={item}>{KIND_LABEL[item]}</option>)}
            </select>
            <span className={`ml-auto shrink-0 text-caption tabular-nums ${overLimit ? "text-destructive" : "text-faint"}`}>{bodyChars} / {MAX_MEMORY_BODY_CHARS}</span>
            {editingId && <button
              type="button"
              className="h-7 shrink-0 rounded-lg px-2.5 text-caption font-medium text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
              disabled={busy}
              onClick={cancelEdit}
            >Cancel</button>}
            <button
              type="button"
              className="h-7 shrink-0 rounded-lg bg-primary px-3 text-caption font-medium text-primary-foreground transition-opacity disabled:opacity-40"
              disabled={busy || !trimmed || overLimit}
              onClick={() => void save()}
            >{editingId ? "Update" : "Save"}</button>
          </div>}
        </section>
        {fromReply && <p className="mt-2 px-0.5 text-caption text-muted-foreground">This is the whole reply. Cut it down to the one rule or fact worth keeping, then pick a kind.</p>}
        {overLimit && <p className="mt-2 px-0.5 text-caption text-destructive">Memories are capped at {MAX_MEMORY_BODY_CHARS} characters. Trim the text; nothing is clipped for you.</p>}

        <div className="mb-3 mt-8 flex flex-wrap items-center gap-2">
          <div className="relative min-w-48 flex-1">
            <Search size={13} aria-hidden="true" className="pointer-events-none absolute left-2.5 top-1/2 -translate-y-1/2 text-faint" />
            <input
              type="text"
              value={query}
              onChange={event => setQuery(event.target.value)}
              placeholder="Search"
              aria-label="Search memory"
              className={`${fieldClass} h-8 w-full pl-8 pr-3 text-caption`}
            />
          </div>
          <div className="u-segmented" role="group" aria-label="Filter by kind">
            <button type="button" aria-pressed={filter === null} data-active={filter === null} className="u-segmented-item" onClick={() => setFilter(null)}>All</button>
            {KINDS.map(item => (
              <button
                key={item}
                type="button"
                aria-pressed={filter === item}
                data-active={filter === item}
                className="u-segmented-item"
                onClick={() => setFilter(current => current === item ? null : item)}
              >{KIND_PLURAL[item]}</button>
            ))}
          </div>
        </div>
        {records === undefined
          ? <div data-testid="memory-loading"><SkeletonRows label="Loading memories" rows={3} rowClassName="px-4 py-3" /></div>
          : visible.length === 0
          ? <p className="rounded-xl border border-dashed border-border px-4 py-10 text-center text-caption text-muted-foreground">
            {query.trim() ? "No memories match your search." : filter ? `No ${KIND_PLURAL[filter as Kind].toLowerCase()} yet.` : "Nothing remembered yet. Add a memory above, or use /pin in any chat."}
          </p>
          : <ul className="divide-y divide-border overflow-hidden rounded-xl border border-border-card bg-card">
            {visible.map(record => {
              const stat = statById.get(record.id);
              return <li key={record.id} className={cn("group flex items-start gap-3 px-4 py-3 transition-colors", editingId === record.id && "bg-accent/50")}>
                <div className="min-w-0 flex-1">
                  <p className="whitespace-pre-wrap break-words text-ui leading-relaxed text-foreground">{record.body}</p>
                  <p className="mt-1 text-caption text-faint">
                    {KIND_LABEL[record.kind as Kind] ?? record.kind} · {new Date(record.createdAt).toLocaleDateString(undefined, { month: "short", day: "numeric" })}
                    {record.provenance === "model_proposal" && <> · extracted</>}
                    {record.provenance === "agent_proposal" && <> · from an agent</>}
                    {record.confidenceBps != null && <> · {Math.round(record.confidenceBps / 100)}% confident</>}
                    {record.supersedes && <> · replaced an earlier memory</>}
                    {(stat?.recalls ?? 0) > 0 && (
                      <span className="tabular-nums"> · recalled {stat!.recalls}× · last {dayAge(stat!.lastRecalledDay)}</span>
                    )}
                  </p>
                </div>
                <div className="-my-0.5 flex shrink-0 items-center gap-0.5 opacity-0 transition-opacity group-hover:opacity-100 group-focus-within:opacity-100">
                  <button type="button" className={iconButton} aria-label="Edit" title="Edit" disabled={busy} onClick={() => beginEdit(record)}>
                    <Pencil size={13} aria-hidden="true" />
                  </button>
                  <button type="button" className={iconButton} aria-label="Forget" title="Forget" disabled={busy} onClick={() => void act(() => bridgeApi.deleteMemoryRecord(record.id))}>
                    <X size={14} aria-hidden="true" />
                  </button>
                </div>
              </li>;
            })}
          </ul>}
        {records !== undefined && !searching && activeList?.cursor && <ShowMore
          remaining={memoryCount - records.length}
          loading={loadingMore === "active"}
          disabled={loadingMore !== null}
          onClick={() => void showMore("active")}
        />}

        {capabilities && capabilities.providerNative.length > 0 && (
          <section className="mt-10">
            <h2 className="mb-2 px-0.5 text-caption font-medium text-muted-foreground">Provider-owned memory</h2>
            <ul className="space-y-1.5 px-0.5">
              {capabilities.providerNative.map(item => (
                <li key={`${item.harness}:${item.command}`} className="flex flex-wrap items-baseline gap-x-2 text-caption text-faint">
                  <span className="font-mono text-muted-foreground">/{item.command}</span>
                  <span>{harnessLabel(item.harness)} · {item.description}. Stays on that provider.</span>
                </li>
              ))}
            </ul>
          </section>
        )}
      </div>}
      {tab === "queue" && <div className="space-y-8">
        <section>
          <h2 className="mb-2 px-0.5 text-ui font-medium text-foreground">After a chat turn</h2>
          <div role="group" aria-label="Extraction mode" className="divide-y divide-border overflow-hidden rounded-xl border border-border-card bg-card">
            {MODES.map(mode => {
              const selected = settings?.mode === mode.id;
              return <button
                key={mode.id}
                type="button"
                aria-pressed={selected}
                disabled={busy}
                onClick={() => void setMode(mode.id)}
                className="flex w-full items-start gap-3 px-4 py-3 text-left transition-colors hover:bg-accent/50 disabled:opacity-60"
              >
                <span aria-hidden="true" className={cn("mt-[3px] grid size-3.5 shrink-0 place-items-center rounded-full border transition-colors", selected ? "border-foreground" : "border-foreground/25")}>
                  {selected && <span className="size-1.5 rounded-full bg-foreground" />}
                </span>
                <span className="min-w-0 flex-1">
                  <span className="block text-ui text-foreground">{mode.label}</span>
                  <span className="mt-0.5 block text-caption leading-relaxed text-muted-foreground">{mode.description}</span>
                </span>
              </button>;
            })}
          </div>
          <p className="mt-2 px-0.5 text-caption text-faint">Invalid, unsafe, duplicate, or over-budget output is refused.</p>
        </section>
        <section>
          <div className="rounded-xl border border-border-card bg-card px-4 py-3">
            <div className="flex flex-wrap items-center gap-3">
              <span className="min-w-0 flex-1 basis-48">
                <span className="block text-ui text-foreground">Memory model</span>
                <span className="mt-0.5 block text-caption leading-relaxed text-muted-foreground">Runs extraction after chats, nightly cleanup and automatic approval.</span>
              </span>
              <select className={`${fieldClass} h-7 w-44 px-2 text-caption`} value={pickedHarness} disabled={busy} onChange={event => pickMemoryHarness(event.target.value)} aria-label="Memory harness">
                <option value="">Use my new-chat model</option>
                {adapters.map(adapter => <option key={adapter.id} value={adapter.id}>{adapter.label}</option>)}
              </select>
              <select className={`${fieldClass} h-7 w-44 px-2 text-caption`} value={pickedModel} disabled={busy || !pickedHarness} onChange={event => pickMemoryModel(event.target.value)} aria-label="Memory model">
                <option value="">Model…</option>
                {models.map(model => <option key={model.id} value={model.id}>{model.label ?? model.id}</option>)}
              </select>
            </div>
            {memoryModel && <div className="mt-2 space-y-1 text-caption leading-relaxed">
              {memoryModel.resolved
                ? <p className="text-muted-foreground" data-testid="memory-model-source">
                  Using {memoryModelLabel(adapters, memoryModel.resolved.harness, memoryModel.resolved.model)}, {MEMORY_MODEL_SOURCE[memoryModel.resolved.source]}.
                </p>
                : <p className="text-warning">No memory job can run: none of these can run a tool-free turn. Pick a Claude model.</p>}
              {memoryModel.skipped.map(skip => <p key={`${skip.source}:${skip.harness}`} className="text-faint">
                Skipped {memoryModelLabel(adapters, skip.harness, skip.model)}, {MEMORY_MODEL_SOURCE[skip.source]}: {skip.reason}.
              </p>)}
            </div>}
          </div>
          {settings?.lastRun && (
            <p className="mt-2 px-0.5 text-caption tabular-nums text-faint">
              Last run {settings.lastRun.status} · {settings.lastRun.proposalCount} extracted · {settings.lastRun.observedTokens} tokens · ${(settings.lastRun.spendMicrousd / 1_000_000).toFixed(4)}
            </p>
          )}
        </section>
        <section>
          <h2 className="mb-2 px-0.5 text-ui font-medium text-foreground">Waiting for review</h2>
          {proposed !== undefined && proposed.length === 0
            ? <p className="rounded-xl border border-dashed border-border px-4 py-10 text-center text-caption text-muted-foreground">
              Nothing to review. Candidates that need your approval land here.
            </p>
            : <>
              {proposed !== undefined && <p className="mb-2 px-0.5 text-caption text-faint">Proposals you don't review within 30 days expire, which frees their space.</p>}
              {proposed !== undefined && <div className="mb-2 flex min-h-7 flex-wrap items-center gap-2 px-0.5" role="toolbar" aria-label="Review selected proposals">
                <label className="flex cursor-pointer items-center gap-2 text-caption text-muted-foreground">
                  <input
                    type="checkbox"
                    className="size-3.5 rounded border-border accent-foreground"
                    checked={allShownSelected}
                    disabled={busy || proposed.length === 0}
                    onChange={() => setSelected(allShownSelected ? new Set() : new Set(proposed.map(record => record.id)))}
                    aria-label="Select all shown proposals"
                  />
                  {selected.size > 0
                    ? <span className="tabular-nums">{selected.size} selected</span>
                    : <span>Select all shown <span className="tabular-nums text-faint">{proposed.length} of {queueCount}</span></span>}
                </label>
                {selected.size > 0 && <div className="ml-auto flex items-center gap-1">
                  <button
                    type="button"
                    className="h-7 rounded-lg px-2.5 text-caption font-medium text-muted-foreground transition-colors hover:bg-accent hover:text-foreground disabled:opacity-45"
                    disabled={busy}
                    onClick={() => void review(false)}
                  >Reject selected</button>
                  <button
                    type="button"
                    className="inline-flex h-7 items-center gap-1.5 rounded-lg bg-primary px-2.5 text-caption font-medium text-primary-foreground transition-opacity disabled:opacity-40"
                    disabled={busy}
                    onClick={() => void review(true)}
                  ><Check size={12} aria-hidden="true" />Approve selected</button>
                </div>}
              </div>}
              <ul className="divide-y divide-border overflow-hidden rounded-xl border border-border-card bg-card">
                {(proposed ?? []).map(record => (
                  <li key={record.id} className={cn("flex flex-wrap items-start gap-3 px-4 py-3 transition-colors", selected.has(record.id) && "bg-accent/50")}>
                    <input
                      type="checkbox"
                      className="mt-1 size-3.5 shrink-0 rounded border-border accent-foreground"
                      checked={selected.has(record.id)}
                      disabled={busy}
                      onChange={() => toggleSelected(record.id)}
                      aria-label={`Select proposal: ${record.body.slice(0, 80)}`}
                    />
                    <div className="min-w-0 flex-1 basis-64">
                      <p className="whitespace-pre-wrap break-words text-ui leading-relaxed text-foreground">{record.body}</p>
                      <p className="mt-1 text-caption text-faint">
                        {KIND_LABEL[record.kind as Kind] ?? record.kind}
                        {record.provenance === "agent_proposal" && <> · proposed by an agent</>}
                        {record.confidenceBps != null && <> · {Math.round(record.confidenceBps / 100)}% confident</>}
                        {record.rationale && <> · {record.rationale}</>}
                      </p>
                    </div>
                    <div className="flex shrink-0 items-center gap-1">
                      <button
                        type="button"
                        className="h-7 rounded-lg px-2.5 text-caption font-medium text-muted-foreground transition-colors hover:bg-accent hover:text-foreground disabled:opacity-45"
                        disabled={busy}
                        onClick={() => void act(() => bridgeApi.rejectMemoryRecord(record.id))}
                      >Reject</button>
                      <button
                        type="button"
                        className="inline-flex h-7 items-center gap-1.5 rounded-lg bg-primary px-2.5 text-caption font-medium text-primary-foreground transition-opacity disabled:opacity-40"
                        disabled={busy}
                        onClick={() => void act(() => bridgeApi.approveMemoryRecord(record.id))}
                      ><Check size={12} aria-hidden="true" />Approve</button>
                    </div>
                  </li>
                ))}
              </ul>
              {proposedList?.cursor && <ShowMore
                remaining={queueCount - (proposed?.length ?? 0)}
                loading={loadingMore === "proposed"}
                disabled={loadingMore !== null}
                onClick={() => void showMore("proposed")}
              />}
            </>}
        </section>
      </div>}
      {tab === "activity" && <div className="space-y-3">
        <div className="grid gap-3 sm:grid-cols-2">
          <section className="rounded-xl border border-border-card bg-card px-4 py-3.5">
            <p className="text-caption text-muted-foreground">Recalls · 14 days</p>
            <p className="mt-1 font-display text-title font-semibold tabular-nums tracking-tight text-foreground">{totalInjections}</p>
            <Sparkbars values={injections} className="mt-3" />
            <p className="mt-2 text-caption tabular-nums text-faint">peak {peakInjections} injections / day</p>
          </section>
          <section className="rounded-xl border border-border-card bg-card px-4 py-3.5">
            <p className="text-caption text-muted-foreground">Packet budget</p>
            <p className="mt-1 font-display text-title font-semibold tabular-nums tracking-tight text-foreground">{budgetPct}%</p>
            <div className="mt-3 h-1 w-full overflow-hidden rounded-full bg-foreground/10" role="meter" aria-label="Packet budget" aria-valuenow={budgetPct} aria-valuemin={0} aria-valuemax={100}>
              <div className={cn("h-full rounded-full", budgetPct > 90 ? "bg-destructive" : "bg-foreground/70")} style={{ width: `${Math.min(100, budgetPct)}%` }} />
            </div>
            <p className="mt-2 text-caption tabular-nums text-faint">
              {stats?.budgetCharsUsed ?? 0} / {stats?.budgetCharsMax ?? 0} chars · refuses at capacity, never evicts
            </p>
          </section>
        </div>
        <section className="rounded-xl border border-border-card bg-card px-4 py-3.5" aria-label="Memory use">
          <div className="flex items-baseline justify-between gap-3">
            <p className="text-caption text-muted-foreground">Memory use · 14 days</p>
            <p className="text-caption tabular-nums text-faint">{stats?.packetsWithMemories ?? 0} of {stats?.packets ?? 0} packets carried a memory</p>
          </div>
          <p className="mt-1 font-display text-title font-semibold tabular-nums tracking-tight text-foreground">{usedPct === null ? "n/a" : `${usedPct}%`}</p>
          <p className="text-caption tabular-nums text-faint">
            {usedPct === null ? "No active memories yet." : `${stats?.recalledRecords ?? 0} of ${stats?.activeRecords ?? 0} active memories reached a prompt.`}
          </p>
          {(stats?.byKind.length ?? 0) > 0 && <ul className="mt-3 space-y-2" aria-label="Use by kind">
            {stats!.byKind.map(entry => (
              <li key={entry.kind}>
                <div className="mb-1 flex items-baseline justify-between gap-3 text-caption">
                  <span className="text-foreground">{KIND_PLURAL[entry.kind as Kind] ?? entry.kind}</span>
                  <span className="tabular-nums text-faint">{entry.recalled} of {entry.active} used · {entry.recalls} recalls</span>
                </div>
                <div className="h-1 w-full overflow-hidden rounded-full bg-foreground/10">
                  <div className="h-full rounded-full bg-foreground/70" style={{ width: `${entry.active === 0 ? 0 : Math.round((entry.recalled / entry.active) * 100)}%` }} />
                </div>
              </li>
            ))}
          </ul>}
          {(stats?.exclusions.length ?? 0) > 0 && <div className="mt-3 border-t border-border pt-3">
            <p className="text-caption text-muted-foreground">Held back at the packet gate</p>
            <ul className="mt-1.5 space-y-1">
              {stats!.exclusions.map(item => (
                <li key={item.code} className="flex items-baseline justify-between gap-3 text-caption">
                  <span className="text-foreground">{EXCLUSION_LABEL[item.code] ?? item.code}</span>
                  <span className="tabular-nums text-faint">{item.count} {item.count === 1 ? "packet" : "packets"}</span>
                </li>
              ))}
            </ul>
          </div>}
          <p className="mt-3 text-caption text-faint">Counts what reached a prompt. It does not measure whether a memory helped.</p>
        </section>
        <section className="rounded-xl border border-border-card bg-card px-4 py-3.5">
          <p className="text-caption text-muted-foreground">Recent runs</p>
          {log !== undefined && log.length === 0
            ? <p className="py-6 text-center text-caption text-faint">No extraction or consolidation runs yet.</p>
            : <ul className="mt-2 divide-y divide-border">
              {(log ?? []).map((entry, index) => (
                <li key={index} className="flex items-baseline gap-3 py-2 text-caption">
                  <span className="w-24 shrink-0 font-mono text-caption text-faint font-medium">{entry.source}</span>
                  <span className="min-w-0 flex-1 truncate text-foreground" title={runDetail(entry)}>{runDetail(entry)}</span>
                  <span className="shrink-0 tabular-nums text-faint">{relative(entry.at)}</span>
                </li>
              ))}
            </ul>}
        </section>
      </div>}
      {tab === "insights" && <MemoryInsights onError={onError} />}
    </div>
  </div>;
}

const EXCLUSION_LABEL: Record<string, string> = {
  unsafe_body: "Text the packet refuses to carry",
  over_budget: "Over the packet budget",
  conflict_group: "Conflicting with another memory",
  candidate_window_truncated: "Beyond the candidate window",
};

/** The next page of a memory list, sized to what is left. */
function ShowMore({ remaining, loading, disabled, onClick }: { remaining: number; loading: boolean; disabled: boolean; onClick: () => void }) {
  // While the page loads the label stays and shimmers: one loading state.
  return <button
    type="button"
    aria-busy={loading}
    className={cn("mt-2 h-8 w-full rounded-lg text-caption font-medium text-muted-foreground transition-colors hover:bg-accent hover:text-foreground disabled:opacity-45", pendingClass(loading))}
    disabled={disabled || loading}
    onClick={onClick}
  >Show <span className="tabular-nums">{Math.min(MEMORY_PAGE_SIZE, Math.max(remaining, 1))}</span> more</button>;
}

/** One line for a settled run: what it did, or why it did not. */
function runDetail(entry: MemoryActivityEntry): string {
  if (entry.status !== "completed") return entry.detail ? `${entry.status}: ${entry.detail}` : entry.status;
  if (entry.source === "extraction") return `${entry.applied} ${entry.applied === 1 ? "proposal" : "proposals"} written`;
  return `${entry.applied} applied, ${entry.refused} refused${entry.detail ? ` · ${entry.detail}` : ""}`;
}

/** Day buckets run 0 = 13 days ago … 13 = today. */
function dayAge(day: number): string {
  return day >= 13 ? "today" : `${13 - day}d ago`;
}

/** A tiny achromatic bar sparkline. `values` are non-negative counts; the
 *  tallest bar is full height. Pure presentation, no axis. */
function Sparkbars({ values, className }: { values: number[]; className?: string }) {
  const max = values.length ? Math.max(1, ...values) : 1;
  return (
    <div className={cn("flex h-8 items-end gap-[3px]", className)} aria-hidden="true">
      {values.map((value, index) => (
        <div
          key={index}
          className={cn("min-h-[2px] flex-1 rounded-sm", value > 0 ? "bg-foreground/60" : "bg-foreground/10")}
          style={{ height: `${Math.round((value / max) * 100)}%` }}
        />
      ))}
    </div>
  );
}
