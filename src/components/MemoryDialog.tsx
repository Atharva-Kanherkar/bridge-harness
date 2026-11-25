import { SCREEN_CONTENT, ScreenHeading } from "./ui/screen";
import { SkeletonRows } from "@/components/ui/skeleton";
import { useEffect, useRef, useState } from "react";
import { Check, Pencil, Search, X } from "lucide-react";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { InputGroup, InputGroupAddon, InputGroupInput } from "@/components/ui/input-group";
import { RowButton } from "@/components/ui/row";
import { SegmentedGroup, SegmentedItem } from "@/components/ui/segmented";
import { Select, SelectItem, SelectPopup, SelectTrigger } from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import { Tip } from "@/components/ui/tooltip";
import { bridgeApi } from "../api";
import { searchRecords } from "../memoryStats";
import { harnessLabel } from "../utils";
import { relative } from "./UsageInsights";
import { MemoryInsights } from "./MemoryInsights";
import type {
  AdapterDescriptor,
  MemoryCapabilities,
  MemoryActivityEntry,
  MemoryExtractionSettings,
  MemoryRecallStats,
  MemoryRecord,
} from "../types";

const KINDS = ["preference", "fact", "decision", "constraint"] as const;
type Kind = typeof KINDS[number];
const KIND_LABEL: Record<Kind, string> = { preference: "Preference", fact: "Fact", decision: "Decision", constraint: "Constraint" };
const KIND_PLURAL: Record<Kind, string> = { preference: "Preferences", fact: "Facts", decision: "Decisions", constraint: "Constraints" };

/** What each extraction mode does after a chat turn, said once beside its choice. */
const MODES = [
  { id: "remember", label: "Manual only", description: "No extraction runs; existing active memory stays active, and new memory is saved only when you ask." },
  { id: "propose", label: "Review first", description: "Each validated extraction that fits the memory budget waits below for your decision." },
  { id: "auto_apply", label: "Automatic", description: "Promotes only 90%+ candidates that cite and match a stable user message from the same chat, with no likely conflict or forgotten match; other validated candidates that fit the budget stay here for review." },
] as const;
/** Mirrors the contract cap in bridge-protocol's memory messages. */
export const MAX_MEMORY_BODY_CHARS = 4000;

/**
 * How long a body is *to the ledger*: trimmed, counted in code points. A
 * JavaScript `.length` counts UTF-16 units, so an emoji would score two and a
 * trailing newline would score at all — and the surface would refuse bodies
 * the server accepts. Mirrors `require_body` in memory_ledger.
 */
export function bodyLength(text: string): number {
  return [...text.trim()].length;
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
  const [records, setRecords] = useState<MemoryRecord[]>();
  const [proposed, setProposed] = useState<MemoryRecord[]>();
  const [settings, setSettings] = useState<MemoryExtractionSettings>();
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
  const [profileHarness, setProfileHarness] = useState("");
  const [profileModel, setProfileModel] = useState("");
  const [busy, setBusy] = useState(false);
  // Which read is the newest. The initial load and every memory-changed hint
  // read, so a slow earlier call must not land over a fresher one. Same shape
  // as RouterSettingsDialog's learningReadGeneration.
  const readGeneration = useRef(0);
  const editorRef = useRef<HTMLTextAreaElement>(null);

  useEffect(() => {
    if (open) return;
    // A closed dialog keeps no state, and an in-flight read must not land on it.
    readGeneration.current += 1;
    setTab("pins");
    setRecords(undefined);
    setProposed(undefined);
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
    setProfileHarness("");
    setProfileModel("");
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
      Promise.all([
        bridgeApi.listMemoryRecords("account:local"),
        bridgeApi.listMemoryRecords("account:local", "proposed"),
        bridgeApi.getExtractionSettings(),
        bridgeApi.getMemoryInjection(),
        bridgeApi.memoryRecallStats("account:local"),
        bridgeApi.memoryActivityLog("account:local"),
        bridgeApi.getConsolidationSettings(),
      ]).then(([activeList, proposedList, extraction, injectionSettings, recallStats, activity, consolidation]) => {
        if (!active || generation !== readGeneration.current) return;
        setRecords(activeList.records);
        setBudget({ held: consolidation.heldRecords, max: consolidation.maxRecords });
        setProposed(proposedList.records);
        setSettings(extraction);
        setInjection(injectionSettings.enabled);
        setStats(recallStats);
        setLog(activity);
        setProfileHarness(current => current || extraction.harness || "");
        setProfileModel(current => current || extraction.model || "");
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
  const queueCount = proposed?.length ?? 0;
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
    const next = mode === "propose" || mode === "auto_apply"
      ? await bridgeApi.updateExtractionSettings(mode, profileHarness, profileModel)
      : await bridgeApi.updateExtractionSettings(mode);
    setSettings(next);
  });

  const models = adapters.find(adapter => adapter.id === profileHarness)?.models ?? [];
  const composing = editingId !== null || body.length > 0;
  const memoryCount = records?.length ?? 0;
  const totalInjections = injections.reduce((sum, value) => sum + value, 0);

  return <div className="h-full overflow-y-auto" aria-labelledby="memory-title">
    <div className={SCREEN_CONTENT}>
      <ScreenHeading
        id="memory-title"
        title="Memory"
        action={<label className="flex cursor-pointer items-center gap-2.5 text-caption text-muted-foreground">
          Use in new chats
          <Switch
            checked={injection ?? true}
            disabled={busy || injection === undefined}
            aria-label="Use active memory in new chats"
            onCheckedChange={next => {
              void act(async () => {
                const applied = await bridgeApi.setMemoryInjection(next);
                setInjection(applied.enabled);
              });
            }}
          />
        </label>}
      />
      <SegmentedGroup label="Memory views" className="mb-6 max-w-full">
        <SegmentedItem active={tab === "pins"} onClick={() => setTab("pins")}>
          Memories{memoryCount > 0 && <span className="ml-1.5 tabular-nums text-faint">{memoryCount}</span>}
        </SegmentedItem>
        <SegmentedItem active={tab === "queue"} onClick={() => setTab("queue")}>
          Review{queueCount > 0 && <span className="ml-1.5 tabular-nums text-faint">{queueCount}</span>}
        </SegmentedItem>
        <SegmentedItem active={tab === "activity"} onClick={() => setTab("activity")}>Activity</SegmentedItem>
        <SegmentedItem active={tab === "insights"} onClick={() => setTab("insights")}>Insights</SegmentedItem>
      </SegmentedGroup>
      {full && budget && <div role="status" className="mb-6 flex flex-wrap items-center gap-3 rounded-xl border border-border-card bg-card px-4 py-3">
        <p className="min-w-0 flex-1 basis-64 text-caption text-muted-foreground">
          <span className="font-medium text-foreground">Memory is full.</span>{" "}
          It holds <span className="tabular-nums">{budget.held}</span> of <span className="tabular-nums">{budget.max}</span> records, counting proposals, so nothing new is saved or proposed and no extraction runs. Review proposals or forget memories you no longer need.
        </p>
        {queueCount > 0 && tab !== "queue" && <Button variant="ghost" size="xs" onClick={() => setTab("queue")}>Review {queueCount}</Button>}
      </div>}
      {tab === "pins" && <div>
        <section aria-label="Memory editor" className="overflow-hidden rounded-xl border border-border-card bg-card transition-colors focus-within:border-ring/60">
          {editingId && <p className="px-4 pt-3 text-caption text-muted-foreground">Editing memory</p>}
          <div className="px-1 py-1.5 text-ui leading-relaxed">
          <Textarea
            unstyled
            ref={editorRef}
            rows={composing ? 3 : 1}
            // One line at rest, three while composing: the primitive's own floor is taller than one line.
            style={{ minHeight: composing ? "calc(3lh + 0.625rem)" : "calc(1lh + 0.625rem)", resize: "none" }}
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
          </div>
          {composing && <div className="flex flex-wrap items-center gap-2 border-t border-border px-2.5 py-2">
            <div className="w-32 shrink-0">
              <Choice label="Kind" value={kind} disabled={busy} onChange={setKind} options={KINDS.map(item => ({ value: item, label: KIND_LABEL[item] }))} />
            </div>
            <span className={`ml-auto shrink-0 text-caption tabular-nums ${overLimit ? "text-destructive" : "text-faint"}`}>{bodyChars} / {MAX_MEMORY_BODY_CHARS}</span>
            {editingId && <Button variant="quiet" size="xs" disabled={busy} onClick={cancelEdit}>Cancel</Button>}
            <Button size="xs" disabled={busy || !trimmed || overLimit} onClick={() => void save()}>{editingId ? "Update" : "Save"}</Button>
          </div>}
        </section>
        {fromReply && <p className="mt-2 px-0.5 text-caption text-muted-foreground">This is the whole reply. Cut it down to the one rule or fact worth keeping, then pick a kind.</p>}
        {overLimit && <p className="mt-2 px-0.5 text-caption text-destructive">Memories are capped at {MAX_MEMORY_BODY_CHARS} characters. Trim the text; nothing is clipped for you.</p>}

        <div className="mb-3 mt-8 flex flex-wrap items-center gap-2">
          <InputGroup className="min-w-48 flex-1">
            <InputGroupAddon><Search aria-hidden="true" /></InputGroupAddon>
            <InputGroupInput
              size="sm"
              type="text"
              value={query}
              onChange={event => setQuery(event.target.value)}
              placeholder="Search"
              aria-label="Search memory"
            />
          </InputGroup>
          <SegmentedGroup label="Filter by kind">
            <SegmentedItem active={filter === null} onClick={() => setFilter(null)}>All</SegmentedItem>
            {KINDS.map(item => (
              <SegmentedItem
                key={item}
                active={filter === item}
                onClick={() => setFilter(current => current === item ? null : item)}
              >{KIND_PLURAL[item]}</SegmentedItem>
            ))}
          </SegmentedGroup>
        </div>
        {records === undefined
          ? <div data-testid="memory-loading"><SkeletonRows label="Loading memories" rows={3} /></div>
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
                  <Button variant="quiet" size="icon-xs" aria-label="Edit" tooltip="Edit" disabled={busy} onClick={() => beginEdit(record)}>
                    <Pencil aria-hidden="true" />
                  </Button>
                  <Button variant="quiet" size="icon-xs" aria-label="Forget" tooltip="Forget" disabled={busy} onClick={() => void act(() => bridgeApi.deleteMemoryRecord(record.id))}>
                    <X aria-hidden="true" />
                  </Button>
                </div>
              </li>;
            })}
          </ul>}

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
              return <RowButton
                key={mode.id}
                size="bare"
                shape="flush"
                aria-pressed={selected}
                disabled={busy}
                onClick={() => void setMode(mode.id)}
              >
                <span className="flex w-full items-start gap-3 px-4 py-3">
                  <span aria-hidden="true" className={cn("mt-0.75 grid size-3.5 shrink-0 place-items-center rounded-full border transition-colors", selected ? "border-foreground" : "border-foreground/25")}>
                    {selected && <span className="size-1.5 rounded-full bg-foreground" />}
                  </span>
                  <span className="min-w-0 flex-1">
                    <span className="block text-ui text-foreground">{mode.label}</span>
                    <span className="mt-0.5 block text-caption leading-relaxed text-muted-foreground">{mode.description}</span>
                  </span>
                </span>
              </RowButton>;
            })}
          </div>
          <p className="mt-2 px-0.5 text-caption text-faint">Invalid, unsafe, duplicate, or over-budget output is refused.</p>
        </section>
        <section>
          <div className="flex flex-wrap items-center gap-3 rounded-xl border border-border-card bg-card px-4 py-3">
            <span className="min-w-0 flex-1 basis-48">
              <span className="block text-ui text-foreground">Extraction model</span>
              <span className="mt-0.5 block text-caption leading-relaxed text-muted-foreground">Runs on each chat&apos;s own model unless you pin a helper.</span>
            </span>
            <div className="w-40 shrink-0">
              <Choice
                label="Extraction harness"
                value={profileHarness}
                disabled={busy}
                onChange={next => { setProfileHarness(next); setProfileModel(""); }}
                options={[{ value: "", label: "Chat's own model" }, ...adapters.map(adapter => ({ value: adapter.id, label: adapter.label }))]}
              />
            </div>
            <div className="w-44 shrink-0">
              <Choice
                label="Extraction model"
                value={profileModel}
                disabled={busy || !profileHarness}
                onChange={setProfileModel}
                options={[{ value: "", label: "Model…" }, ...models.map(model => ({ value: model.id, label: model.label ?? model.id }))]}
              />
            </div>
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
              <ul className="divide-y divide-border overflow-hidden rounded-xl border border-border-card bg-card">
                {(proposed ?? []).map(record => (
                  <li key={record.id} className="flex flex-wrap items-start gap-3 px-4 py-3">
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
                      <Button variant="quiet" size="xs" disabled={busy} onClick={() => void act(() => bridgeApi.rejectMemoryRecord(record.id))}>Reject</Button>
                      <Button size="xs" disabled={busy} onClick={() => void act(() => bridgeApi.approveMemoryRecord(record.id))}><Check aria-hidden="true" />Approve</Button>
                    </div>
                  </li>
                ))}
              </ul>
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
                  <Tip label={runDetail(entry)}><span className="min-w-0 flex-1 truncate text-foreground">{runDetail(entry)}</span></Tip>
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

/** A compact choice from a short list. The caller's wrapper sets the width. */
function Choice({ label, value, options, onChange, disabled }: {
  label: string;
  value: string;
  options: { value: string; label: string }[];
  onChange: (next: string) => void;
  disabled?: boolean;
}) {
  const selected = options.find(option => option.value === value);
  return <Select value={value} disabled={disabled} onValueChange={next => { if (next !== null) onChange(next); }}>
    <SelectTrigger size="sm" aria-label={label} className="min-w-0">
      <span className="min-w-0 flex-1 truncate">{selected?.label ?? value}</span>
    </SelectTrigger>
    <SelectPopup alignItemWithTrigger={false}>
      {options.map(option => <SelectItem key={option.value} value={option.value}>{option.label}</SelectItem>)}
    </SelectPopup>
  </Select>;
}

/** A tiny achromatic bar sparkline. `values` are non-negative counts; the
 *  tallest bar is full height. Pure presentation, no axis. */
function Sparkbars({ values, className }: { values: number[]; className?: string }) {
  const max = values.length ? Math.max(1, ...values) : 1;
  return (
    <div className={cn("flex h-8 items-end gap-0.75", className)} aria-hidden="true">
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
