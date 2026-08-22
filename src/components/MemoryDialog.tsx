import { useEffect, useRef, useState } from "react";
import { Pin, X } from "lucide-react";
import { bridgeApi } from "../api";
import { harnessLabel } from "../utils";
import type { MemoryCapabilities, MemoryRecord } from "../types";

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
  onClose,
  onError,
}: {
  open: boolean;
  /** Pre-filled composer text (a too-long "Remember this"); never auto-saved. */
  initialBody?: string | null;
  onClose: () => void;
  onError: (message: string) => void;
}) {
  const [records, setRecords] = useState<MemoryRecord[]>();
  const [capabilities, setCapabilities] = useState<MemoryCapabilities>();
  const [filter, setFilter] = useState<string | null>(null);
  const [body, setBody] = useState("");
  const [kind, setKind] = useState<string>("preference");
  const [busy, setBusy] = useState(false);
  // Which list read is the newest. The initial load and every memory-changed
  // hint read, so a slow earlier call must not land over a fresher one. Same
  // shape as RouterSettingsDialog's learningReadGeneration.
  const readGeneration = useRef(0);

  useEffect(() => {
    if (open) return;
    // A closed dialog keeps no state, and an in-flight read must not land on it.
    readGeneration.current += 1;
    setRecords(undefined);
    setCapabilities(undefined);
    setFilter(null);
    setBody("");
    setKind("preference");
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
      bridgeApi.listMemoryRecords("account:local").then(result => {
        if (!active || generation !== readGeneration.current) return;
        setRecords(result.records);
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

  const save = async () => {
    setBusy(true);
    try {
      await bridgeApi.saveMemoryRecord(body, kind, undefined);
      setBody("");
    } catch (error) { onError(error instanceof Error ? error.message : String(error)); }
    finally { setBusy(false); }
  };
  const forget = async (recordId: string) => {
    setBusy(true);
    try { await bridgeApi.deleteMemoryRecord(recordId); }
    catch (error) { onError(error instanceof Error ? error.message : String(error)); }
    finally { setBusy(false); }
  };

  const fieldClass = "w-full min-w-0 rounded-xl border border-input bg-card px-3 text-sm text-foreground transition-colors disabled:opacity-45";
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
      <div className="min-h-0 flex-1 space-y-4 overflow-y-auto p-5">
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
            <button
              type="button"
              className="ml-auto h-9 rounded-xl bg-primary px-4 text-sm font-medium text-primary-foreground transition-opacity disabled:opacity-45"
              disabled={busy || !trimmed || overLimit}
              onClick={() => void save()}
            >Save pin</button>
          </div>
          {overLimit && <p className="text-[12px] text-destructive">Pins are capped at {MAX_MEMORY_BODY_CHARS} characters. Trim the text — nothing is clipped for you.</p>}
        </div>
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
                <p className="mt-1 text-[11px] text-muted-foreground"><span className="font-medium">{record.kind}</span> · {new Date(record.createdAt).toLocaleDateString()}</p>
              </div>
              <button
                type="button"
                className="shrink-0 rounded-lg p-1.5 text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
                aria-label="Forget"
                title="Forget"
                disabled={busy}
                onClick={() => void forget(record.id)}
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
      </div>
    </div>
  </div>;
}
