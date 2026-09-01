import { useEffect, useRef, useState } from "react";
import { Check, Pencil, Search, X } from "lucide-react";
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
  };
  const cancelEdit = () => {
    setEditingId(null);
    setBody("");
    setKind("preference");
  };
  const setMode = (mode: string) => act(async () => {
    const next = mode === "propose"
      ? await bridgeApi.updateExtractionSettings("propose", profileHarness, profileModel)
      : await bridgeApi.updateExtractionSettings(mode);
    setSettings(next);
  });

  const fieldClass = "w-full min-w-0 rounded-xl border border-input bg-card px-3 text-sm text-foreground transition-colors disabled:opacity-45";
  const tabClass = (active: boolean) =>
    `rounded-full px-3 py-1 text-[12px] font-medium transition-colors ${active ? "bg-accent text-foreground" : "text-muted-foreground hover:text-foreground"}`;
  const models = adapters.find(adapter => adapter.id === profileHarness)?.models ?? [];

  return <div className="h-full overflow-y-auto" aria-labelledby="memory-title">
    <div className="mx-auto w-full max-w-5xl px-5 py-6 sm:px-8 sm:py-8">
      <header className="mb-5 flex items-end gap-3" data-tauri-drag-region="deep">
        <div className="min-w-0 flex-1">
          <h1 id="memory-title" className="m-0 font-display text-[19px] font-semibold tracking-[-0.02em] text-foreground">Memory</h1>
          <p className="mt-1 text-[13px] leading-relaxed text-muted-foreground">Account memory on this machine. Pins live under <span className="font-mono text-[11px]">account:local</span> and follow you across every chat here — not this chat's history, not the helper picker, and not a provider's <span className="font-mono text-[11px]">/memory</span>.</p>
        </div>
      </header>
      <div className="mb-5 flex flex-wrap items-center gap-1.5">
        <button type="button" aria-pressed={tab === "pins"} className={tabClass(tab === "pins")} onClick={() => setTab("pins")}>About me</button>
        <button type="button" aria-pressed={tab === "queue"} className={tabClass(tab === "queue")} onClick={() => setTab("queue")}>
          Review queue{queueCount > 0 && <span className="ml-1.5 rounded-full bg-accent px-1 font-mono text-[10px] leading-4 text-muted-foreground">{queueCount}</span>}
        </button>
        <button type="button" aria-pressed={tab === "activity"} className={tabClass(tab === "activity")} onClick={() => setTab("activity")}>Activity</button>
      </div>
      {tab === "pins" && <div className="space-y-4">
        <div className="space-y-2">
          <textarea
            className={`${fieldClass} min-h-24 py-2.5`}
            placeholder="Something Bridge should remember about you or how you work"
            value={body}
            disabled={busy}
            onChange={event => setBody(event.target.value)}
            aria-label="New pin"
          />
          <div className="flex items-center gap-3">
            <select className={`${fieldClass} h-9 w-36`} value={kind} disabled={busy} onChange={event => setKind(event.target.value)} aria-label="Kind">
              {KINDS.map(item => <option key={item} value={item}>{item}</option>)}
            </select>
            <span className={`shrink-0 text-[11px] tabular-nums ${overLimit ? "text-destructive" : "text-muted-foreground"}`}>{bodyChars} / {MAX_MEMORY_BODY_CHARS}</span>
            {editingId && (
              <button
                type="button"
                className="ml-auto h-9 shrink-0 rounded-xl border border-border px-3 text-sm font-medium text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
                disabled={busy}
                onClick={cancelEdit}
              >Cancel</button>
            )}
            <button
              type="button"
              className={`h-9 shrink-0 rounded-xl bg-primary px-4 text-sm font-medium text-primary-foreground transition-opacity disabled:opacity-45 ${editingId ? "" : "ml-auto"}`}
              disabled={busy || !trimmed || overLimit}
              onClick={() => void save()}
            >{editingId ? "Save edit" : "Save pin"}</button>
          </div>
          {overLimit && <p className="text-[12px] text-destructive">Pins are capped at {MAX_MEMORY_BODY_CHARS} characters. Trim the text — nothing is clipped for you.</p>}
        </div>
        <label className="flex items-center gap-2 text-[12px] text-muted-foreground">
          <input
            type="checkbox"
            checked={injection ?? true}
            disabled={busy || injection === undefined}
            aria-label="Use pins in new chats"
            onChange={event => {
              const next = event.target.checked;
              void act(async () => {
                const applied = await bridgeApi.setMemoryInjection(next);
                setInjection(applied.enabled);
              });
            }}
          />
          Use pins in new chats — sessions start with your pins in context, cited by id.
        </label>
        <div className="flex flex-wrap items-center gap-1.5">
          <div className="relative mr-1.5 min-w-40 flex-1">
            <Search size={13} aria-hidden="true" className="pointer-events-none absolute left-2.5 top-1/2 -translate-y-1/2 text-muted-foreground" />
            <input
              type="text"
              value={query}
              onChange={event => setQuery(event.target.value)}
              placeholder="Search memory"
              aria-label="Search memory"
              className={`${fieldClass} h-8 pl-8 text-[12px]`}
            />
          </div>
          {KINDS.map(item => (
            <button
              key={item}
              type="button"
              aria-pressed={filter === item}
              className={`rounded-full border px-2.5 py-0.5 text-[11px] font-medium transition-colors ${filter === item ? "border-transparent bg-accent text-foreground" : "border-border text-muted-foreground hover:text-foreground"}`}
              onClick={() => setFilter(current => current === item ? null : item)}
            >{item}</button>
          ))}
        </div>
        <ul className="space-y-2">
          {visible.map(record => (
            <li key={record.id} className="u-glass-soft flex items-start gap-3 rounded-2xl px-3.5 py-3">
              <div className="min-w-0 flex-1">
                <p className="whitespace-pre-wrap break-words text-sm text-foreground">{record.body}</p>
                <p className="mt-1 text-[11px] text-muted-foreground">
                  <span className="font-medium">{record.kind}</span> · {new Date(record.createdAt).toLocaleDateString()}
                  {record.provenance === "model_proposal" && <> · suggested</>}
                  {record.confidenceBps != null && <> · {Math.round(record.confidenceBps / 100)}% confident</>}
                  {record.supersedes && <> · replaced an earlier pin</>}
                  {(statById.get(record.id)?.recalls ?? 0) > 0 && (
                    <span className="tabular-nums"> · recalled {statById.get(record.id)!.recalls}× · last {dayAge(statById.get(record.id)!.lastRecalledDay)}</span>
                  )}
                </p>
              </div>
              <button
                type="button"
                className="shrink-0 rounded-lg p-1.5 text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
                aria-label="Edit"
                title="Edit"
                disabled={busy}
                onClick={() => beginEdit(record)}
              ><Pencil size={14} aria-hidden="true" /></button>
              <button
                type="button"
                className="shrink-0 rounded-lg p-1.5 text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
                aria-label="Forget"
                title="Forget"
                disabled={busy}
                onClick={() => void act(() => bridgeApi.deleteMemoryRecord(record.id))}
              ><X size={14} aria-hidden="true" /></button>
            </li>
          ))}
          {records !== undefined && visible.length === 0 && (
            <li className="rounded-2xl border border-dashed border-border px-3.5 py-6 text-center text-[13px] text-muted-foreground">
              {query.trim() ? "No pins match your search." : filter ? `No ${filter} pins yet.` : "Nothing pinned yet. Save something above, or use /pin in any chat."}
            </li>
          )}
        </ul>
        {capabilities && capabilities.providerNative.length > 0 && (
          <div className="border-t border-border pt-3">
            <p className="text-[11px] font-semibold uppercase tracking-wider text-muted-foreground">Provider-owned memory</p>
            <ul className="mt-1.5 space-y-1">
              {capabilities.providerNative.map(item => (
                <li key={`${item.harness}:${item.command}`} className="text-[12px] text-muted-foreground">
                  <span className="font-mono text-[11px] text-foreground">/{item.command}</span> · {harnessLabel(item.harness)} — {item.description}. Stays on that provider.
                </li>
              ))}
            </ul>
          </div>
        )}
      </div>}
      {tab === "queue" && <div className="space-y-4">
        <div className="space-y-2">
          <p className="text-[11px] font-semibold uppercase tracking-wider text-muted-foreground">After a chat turn</p>
          <div className="flex flex-wrap items-center gap-1.5">
            <button type="button" aria-pressed={settings?.mode === "remember"} className={tabClass(settings?.mode === "remember")} disabled={busy} onClick={() => void setMode("remember")}>Remember</button>
            <button type="button" aria-pressed={settings?.mode === "propose"} className={tabClass(settings?.mode === "propose")} disabled={busy} onClick={() => void setMode("propose")}>Propose</button>
            <button type="button" aria-pressed={false} className={`${tabClass(false)} opacity-45`} disabled title="Auto-apply needs the replay bench before it can exist.">Auto-apply</button>
          </div>
          <p className="text-[12px] leading-relaxed text-muted-foreground">Remember saves only what you ask. Propose replays finished turns on your pinned helper and queues suggestions here — nothing activates without you.</p>
          <div className="flex flex-wrap items-center gap-2">
            <select className={`${fieldClass} h-9 w-40`} value={profileHarness} disabled={busy} onChange={event => { setProfileHarness(event.target.value); setProfileModel(""); }} aria-label="Extraction harness">
              <option value="">Helper…</option>
              {adapters.map(adapter => <option key={adapter.id} value={adapter.id}>{adapter.label}</option>)}
            </select>
            <select className={`${fieldClass} h-9 w-48`} value={profileModel} disabled={busy || !profileHarness} onChange={event => setProfileModel(event.target.value)} aria-label="Extraction model">
              <option value="">Model…</option>
              {models.map(model => <option key={model.id} value={model.id}>{model.label ?? model.id}</option>)}
            </select>
          </div>
          {settings?.lastRun && (
            <p className="text-[11px] tabular-nums text-muted-foreground">
              Last run {settings.lastRun.status} · {settings.lastRun.proposalCount} proposed · {settings.lastRun.observedTokens} tokens · ${(settings.lastRun.spendMicrousd / 1_000_000).toFixed(4)}
            </p>
          )}
        </div>
        <ul className="space-y-2">
          {(proposed ?? []).map(record => (
            <li key={record.id} className="u-glass-soft rounded-2xl px-3.5 py-3">
              <p className="whitespace-pre-wrap break-words text-sm text-foreground">{record.body}</p>
              <p className="mt-1 text-[11px] text-muted-foreground">
                <span className="font-medium">{record.kind}</span>
                {record.confidenceBps != null && <> · {Math.round(record.confidenceBps / 100)}% confident</>}
                {record.rationale && <> · {record.rationale}</>}
              </p>
              <div className="mt-2 flex items-center gap-2">
                <button
                  type="button"
                  className="inline-flex h-8 items-center gap-1.5 rounded-lg bg-primary px-3 text-[12px] font-medium text-primary-foreground transition-opacity disabled:opacity-45"
                  disabled={busy}
                  onClick={() => void act(() => bridgeApi.approveMemoryRecord(record.id))}
                ><Check size={13} aria-hidden="true" />Approve</button>
                <button
                  type="button"
                  className="inline-flex h-8 items-center gap-1.5 rounded-lg border border-border px-3 text-[12px] font-medium text-muted-foreground transition-colors hover:bg-accent hover:text-foreground disabled:opacity-45"
                  disabled={busy}
                  onClick={() => void act(() => bridgeApi.rejectMemoryRecord(record.id))}
                >Reject</button>
              </div>
            </li>
          ))}
          {proposed !== undefined && proposed.length === 0 && (
            <li className="rounded-2xl border border-dashed border-border px-3.5 py-6 text-center text-[13px] text-muted-foreground">
              Nothing to review. Proposals from finished turns land here when Propose is on.
            </li>
          )}
        </ul>
      </div>}
      {tab === "activity" && <div className="space-y-3">
        <div className="grid grid-cols-2 gap-3">
          <section className="u-glass-soft rounded-2xl px-3.5 py-3">
            <p className="text-[11px] font-semibold uppercase tracking-wider text-muted-foreground">Recall · 14 days</p>
            <Sparkbars values={injections} className="mt-2.5" />
            <p className="mt-1.5 text-[11px] tabular-nums text-muted-foreground">peak {peakInjections} injections / day</p>
          </section>
          <section className="u-glass-soft rounded-2xl px-3.5 py-3">
            <p className="text-[11px] font-semibold uppercase tracking-wider text-muted-foreground">Packet budget</p>
            <div className="mt-3 h-1.5 w-full overflow-hidden rounded-full bg-accent" role="meter" aria-label="Packet budget" aria-valuenow={budgetPct} aria-valuemin={0} aria-valuemax={100}>
              <div className={`h-full rounded-full ${budgetPct > 90 ? "bg-destructive" : "bg-foreground/70"}`} style={{ width: `${Math.min(100, budgetPct)}%` }} />
            </div>
            <p className="mt-2 text-[11px] tabular-nums text-muted-foreground">
              {stats?.budgetCharsUsed ?? 0} / {stats?.budgetCharsMax ?? 0} chars · refuses at capacity, never evicts
            </p>
          </section>
        </div>
        <section className="u-glass-soft rounded-2xl px-3.5 py-3">
          <p className="text-[11px] font-semibold uppercase tracking-wider text-muted-foreground">Consolidation log</p>
          <ul className="mt-2 space-y-1.5">
            {(log ?? []).map((entry, index) => (
              <li key={index} className="flex items-baseline gap-2 text-[12px]">
                <span className="w-14 shrink-0 rounded-full border border-border px-1.5 text-center font-mono text-[10px] uppercase leading-4 text-muted-foreground">{entry.op}</span>
                <span className="min-w-0 flex-1 truncate text-foreground" title={entry.detail}>{entry.detail}</span>
                <span className="shrink-0 font-mono text-[10px] tabular-nums text-muted-foreground">{dayAge(entry.day)}</span>
              </li>
            ))}
            {log !== undefined && log.length === 0 && (
              <li className="rounded-2xl border border-dashed border-border px-3.5 py-6 text-center text-[13px] text-muted-foreground">
                No consolidation runs yet.
              </li>
            )}
          </ul>
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
    <div className={`flex h-8 items-end gap-0.5 ${className ?? ""}`} aria-hidden="true">
      {values.map((value, index) => (
        <div
          key={index}
          className="min-h-px flex-1 rounded-sm bg-foreground/55"
          style={{ height: `${Math.round((value / max) * 100)}%` }}
        />
      ))}
    </div>
  );
}
