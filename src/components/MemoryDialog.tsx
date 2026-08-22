import { useEffect, useRef, useState } from "react";
import { Check, Pencil, Pin, X } from "lucide-react";
import { bridgeApi } from "../api";
import { harnessLabel } from "../utils";
import type { AdapterDescriptor, MemoryCapabilities, MemoryExtractionSettings, MemoryRecord } from "../types";

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
 * Account memory on this machine (`account:local`). Deliberately not this
 * chat's history and not the helper picker — the header says so, because the
 * one-dialog-three-products confusion is the bug this surface exists to fix.
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
  const [tab, setTab] = useState<"pins" | "queue">("pins");
  const [records, setRecords] = useState<MemoryRecord[]>();
  const [proposed, setProposed] = useState<MemoryRecord[]>();
  const [settings, setSettings] = useState<MemoryExtractionSettings>();
  const [capabilities, setCapabilities] = useState<MemoryCapabilities>();
  const [injection, setInjection] = useState<boolean>();
  const [filter, setFilter] = useState<string | null>(null);
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
    setFilter(null);
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
      ]).then(([activeList, proposedList, extraction, injectionSettings]) => {
        if (!active || generation !== readGeneration.current) return;
        setRecords(activeList.records);
        setProposed(proposedList.records);
        setSettings(extraction);
        setInjection(injectionSettings.enabled);
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
  const visible = (records ?? []).filter(record => !filter || record.kind === filter);
  const queueCount = proposed?.length ?? 0;

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

  return <div className="fixed inset-0 z-50 flex items-start justify-center overflow-y-auto bg-scrim p-4 pt-[6vh] backdrop-blur-md" role="dialog" aria-modal="true" aria-labelledby="memory-title" onMouseDown={event => { if (event.target === event.currentTarget) onClose(); }}>
    <div className="u-overlay-strong animate-page-enter flex max-h-[90dvh] w-full max-w-2xl flex-col overflow-hidden rounded-3xl">
      <header className="flex shrink-0 items-start gap-3 border-b border-border px-5 py-4">
        <span className="mt-0.5 flex h-9 w-9 shrink-0 items-center justify-center rounded-xl bg-muted text-foreground"><Pin size={18} aria-hidden="true" /></span>
        <div className="min-w-0 flex-1">
          <h2 id="memory-title" className="font-display text-base font-semibold text-foreground">Memory</h2>
          <p className="mt-1 text-[13px] leading-relaxed text-muted-foreground">Account memory on this machine. Pins live under <span className="font-mono text-[11px]">account:local</span> and follow you across every chat here.</p>
          <p className="mt-2 text-[12px] leading-relaxed text-muted-foreground">Not this chat's history, not the helper picker, and not a provider's <span className="font-mono text-[11px]">/memory</span> — those stay where they live.</p>
        </div>
        <button type="button" className="shrink-0 rounded-xl p-2 text-muted-foreground transition-colors hover:bg-accent hover:text-foreground" onClick={onClose} aria-label="Close"><X size={16} aria-hidden="true" /></button>
      </header>
      <div className="flex shrink-0 items-center gap-1.5 border-b border-border px-5 py-2">
        <button type="button" aria-pressed={tab === "pins"} className={tabClass(tab === "pins")} onClick={() => setTab("pins")}>About me</button>
        <button type="button" aria-pressed={tab === "queue"} className={tabClass(tab === "queue")} onClick={() => setTab("queue")}>
          Review queue{queueCount > 0 && <span className="ml-1.5 rounded-full bg-accent px-1 font-mono text-[10px] leading-4 text-muted-foreground">{queueCount}</span>}
        </button>
      </div>
      {tab === "pins" && <div className="min-h-0 flex-1 space-y-4 overflow-y-auto p-5">
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
            <span className={`text-[11px] tabular-nums ${overLimit ? "text-destructive" : "text-muted-foreground"}`}>{bodyChars} / {MAX_MEMORY_BODY_CHARS}</span>
            {editingId && (
              <button
                type="button"
                className="ml-auto h-9 rounded-xl border border-border px-3 text-sm font-medium text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
                disabled={busy}
                onClick={cancelEdit}
              >Cancel</button>
            )}
            <button
              type="button"
              className={`h-9 rounded-xl bg-primary px-4 text-sm font-medium text-primary-foreground transition-opacity disabled:opacity-45 ${editingId ? "" : "ml-auto"}`}
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
              {filter ? `No ${filter} pins yet.` : "Nothing pinned yet. Save something above, or use /pin in any chat."}
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
      {tab === "queue" && <div className="min-h-0 flex-1 space-y-4 overflow-y-auto p-5">
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
    </div>
  </div>;
}
