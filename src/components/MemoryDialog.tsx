import { SCREEN_CONTENT, ScreenHeading } from "./ui/screen";
import { useEffect, useRef, useState } from "react";
import { Check, Pencil, Search, X } from "lucide-react";
import { cn } from "@/lib/utils";
import { bridgeApi } from "../api";
import { searchRecords } from "../memoryStats";
import { harnessLabel } from "../utils";
import type {
  AdapterDescriptor,
  MemoryCapabilities,
  MemoryConsolidationEntry,
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
 * What "Remember this" does with a message: at or under the cap it saves
 * directly; over it the dialog opens pre-filled for the user to trim. Never a
 * clip, never a truncated write.
 */
export function rememberAction(text: string): "save" | "open-dialog" {
  return bodyLength(text) > MAX_MEMORY_BODY_CHARS ? "open-dialog" : "save";
}

/**
 * Account memory on this machine (`account:local`) — the one and only memory
 * surface. Deliberately not this chat's history and not the helper picker —
 * the header says so, because the one-dialog-three-products confusion is the
 * bug this surface exists to fix. The Activity tab carries the read-only
 * recall analytics (injections, packet budget, consolidation log) that used to
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
  /** Pre-filled composer text (a too-long "Remember this"); never auto-saved. */
  initialBody?: string | null;
  adapters?: AdapterDescriptor[];
  onClose: () => void;
  onError: (message: string) => void;
}) {
  const [tab, setTab] = useState<"pins" | "queue" | "activity">("pins");
  const [records, setRecords] = useState<MemoryRecord[]>();
  const [proposed, setProposed] = useState<MemoryRecord[]>();
  const [settings, setSettings] = useState<MemoryExtractionSettings>();
  const [capabilities, setCapabilities] = useState<MemoryCapabilities>();
  const [injection, setInjection] = useState<boolean>();
  const [stats, setStats] = useState<MemoryRecallStats>();
  const [log, setLog] = useState<MemoryConsolidationEntry[]>();
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
        bridgeApi.memoryConsolidationLog("account:local"),
      ]).then(([activeList, proposedList, extraction, injectionSettings, recallStats, consolidation]) => {
        if (!active || generation !== readGeneration.current) return;
        setRecords(activeList.records);
        setProposed(proposedList.records);
        setSettings(extraction);
        setInjection(injectionSettings.enabled);
        setStats(recallStats);
        setLog(consolidation);
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
  const statById = new Map((stats?.perRecord ?? []).map(stat => [stat.id, stat]));
  const injections = stats?.injectionsPerDay ?? [];
  const peakInjections = injections.length ? Math.max(...injections) : 0;
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

  const fieldClass = "rounded-lg border border-border-card bg-card text-foreground outline-none transition-colors placeholder:text-faint focus:border-ring/60 disabled:opacity-45";
  const quietSelect = "h-7 min-w-0 rounded-lg border border-transparent bg-transparent px-1.5 text-caption text-muted-foreground outline-none transition-colors hover:text-foreground focus:border-border-card disabled:opacity-45";
  const iconButton = "grid size-7 shrink-0 place-items-center rounded-lg text-muted-foreground transition-colors hover:bg-accent hover:text-foreground disabled:opacity-45";
  const models = adapters.find(adapter => adapter.id === profileHarness)?.models ?? [];
  const composing = editingId !== null || body.length > 0;
  const memoryCount = records?.length ?? 0;
  const totalInjections = injections.reduce((sum, value) => sum + value, 0);

  return <div className="h-full overflow-y-auto" aria-labelledby="memory-title">
    <div className={SCREEN_CONTENT}>
      <ScreenHeading
        id="memory-title"
        title="Memory"
        description="What Bridge remembers across your conversations."
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
      </div>
      {tab === "pins" && <div>
        <section aria-label="Memory editor" className="overflow-hidden rounded-xl border border-border-card bg-card transition-colors focus-within:border-ring/60">
          {editingId && <p className="px-4 pt-3 text-caption text-muted-foreground">Editing memory</p>}
          <textarea
            ref={editorRef}
            rows={composing ? 3 : 1}
            className="block w-full resize-none bg-transparent px-4 py-3 text-[13px] leading-relaxed text-foreground outline-none placeholder:text-faint disabled:opacity-45"
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
        {records !== undefined && visible.length === 0
          ? <p className="rounded-xl border border-dashed border-border px-4 py-10 text-center text-caption text-muted-foreground">
            {query.trim() ? "No memories match your search." : filter ? `No ${KIND_PLURAL[filter as Kind].toLowerCase()} yet.` : "Nothing remembered yet. Add a memory above, or use /pin in any chat."}
          </p>
          : <ul className="divide-y divide-border overflow-hidden rounded-xl border border-border-card bg-card">
            {visible.map(record => {
              const stat = statById.get(record.id);
              return <li key={record.id} className={cn("group flex items-start gap-3 px-4 py-3 transition-colors", editingId === record.id && "bg-accent/50")}>
                <div className="min-w-0 flex-1">
                  <p className="whitespace-pre-wrap break-words text-[13px] leading-relaxed text-foreground">{record.body}</p>
                  <p className="mt-1 text-caption text-faint">
                    {KIND_LABEL[record.kind as Kind] ?? record.kind} · {new Date(record.createdAt).toLocaleDateString(undefined, { month: "short", day: "numeric" })}
                    {record.provenance === "model_proposal" && <> · extracted</>}
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
                  <span className="block text-[13px] text-foreground">{mode.label}</span>
                  <span className="mt-0.5 block text-caption leading-relaxed text-muted-foreground">{mode.description}</span>
                </span>
              </button>;
            })}
          </div>
          <p className="mt-2 px-0.5 text-caption text-faint">Invalid, unsafe, duplicate, or over-budget output is refused.</p>
        </section>
        <section>
          <div className="flex flex-wrap items-center gap-3 rounded-xl border border-border-card bg-card px-4 py-3">
            <span className="min-w-0 flex-1 basis-48">
              <span className="block text-[13px] text-foreground">Extraction model</span>
              <span className="mt-0.5 block text-caption leading-relaxed text-muted-foreground">Runs on each chat&apos;s own model unless you pin a helper.</span>
            </span>
            <select className={`${fieldClass} h-7 w-40 px-2 text-caption`} value={profileHarness} disabled={busy} onChange={event => { setProfileHarness(event.target.value); setProfileModel(""); }} aria-label="Extraction harness">
              <option value="">Chat&apos;s own model</option>
              {adapters.map(adapter => <option key={adapter.id} value={adapter.id}>{adapter.label}</option>)}
            </select>
            <select className={`${fieldClass} h-7 w-44 px-2 text-caption`} value={profileModel} disabled={busy || !profileHarness} onChange={event => setProfileModel(event.target.value)} aria-label="Extraction model">
              <option value="">Model…</option>
              {models.map(model => <option key={model.id} value={model.id}>{model.label ?? model.id}</option>)}
            </select>
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
            : <ul className="divide-y divide-border overflow-hidden rounded-xl border border-border-card bg-card">
              {(proposed ?? []).map(record => (
                <li key={record.id} className="flex flex-wrap items-start gap-3 px-4 py-3">
                  <div className="min-w-0 flex-1 basis-64">
                    <p className="whitespace-pre-wrap break-words text-[13px] leading-relaxed text-foreground">{record.body}</p>
                    <p className="mt-1 text-caption text-faint">
                      {KIND_LABEL[record.kind as Kind] ?? record.kind}
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
            </ul>}
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
        <section className="rounded-xl border border-border-card bg-card px-4 py-3.5">
          <p className="text-caption text-muted-foreground">Consolidation log</p>
          {log !== undefined && log.length === 0
            ? <p className="py-6 text-center text-caption text-faint">No consolidation runs yet.</p>
            : <ul className="mt-2 divide-y divide-border">
              {(log ?? []).map((entry, index) => (
                <li key={index} className="flex items-baseline gap-3 py-2 text-caption">
                  <span className="w-14 shrink-0 font-mono text-[11px] uppercase tracking-wide text-faint">{entry.op}</span>
                  <span className="min-w-0 flex-1 truncate text-foreground" title={entry.detail}>{entry.detail}</span>
                  <span className="shrink-0 tabular-nums text-faint">{dayAge(entry.day)}</span>
                </li>
              ))}
            </ul>}
        </section>
      </div>}
    </div>
  </div>;
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
          className={cn("min-h-[2px] flex-1 rounded-[2px]", value > 0 ? "bg-foreground/60" : "bg-foreground/10")}
          style={{ height: `${Math.round((value / max) * 100)}%` }}
        />
      ))}
    </div>
  );
}
