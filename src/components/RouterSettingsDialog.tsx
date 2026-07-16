import { useEffect, useMemo, useState } from "react";
import { BrainCircuit, LoaderCircle, ShieldCheck, X } from "lucide-react";
import { bridgeApi } from "../api";
import type { AdapterDescriptor, RouterMode, RouterPreferences } from "../types";

const defaults: RouterPreferences = {
  mode: "shadow",
  minimumPassBps: 6500,
  pinnedHarness: null,
  pinnedModel: null,
  excludedHarnesses: [],
  excludedModels: [],
};

function parseList(value: string): string[] {
  return [...new Set(value.split(",").map(item => item.trim().toLowerCase()).filter(Boolean))];
}

export function RouterSettingsDialog({
  open,
  workspaceId,
  adapters,
  onClose,
  onError,
}: {
  open: boolean;
  workspaceId?: string;
  adapters: AdapterDescriptor[];
  onClose: () => void;
  onError: (message: string) => void;
}) {
  const [preferences, setPreferences] = useState<RouterPreferences>(defaults);
  const [excludedHarnesses, setExcludedHarnesses] = useState("");
  const [excludedModels, setExcludedModels] = useState("");
  const [busy, setBusy] = useState(false);
  const models = useMemo(() => adapters.flatMap(adapter => adapter.models.map(model => ({ ...model, harness: adapter.id, harnessLabel: adapter.label }))), [adapters]);

  useEffect(() => {
    if (!open || !workspaceId) return;
    let active = true;
    setBusy(true);
    bridgeApi.routerPreferences(workspaceId).then(value => {
      if (!active) return;
      setPreferences(value);
      setExcludedHarnesses(value.excludedHarnesses.join(", "));
      setExcludedModels(value.excludedModels.join(", "));
    }).catch(error => { if (active) onError(String(error)); }).finally(() => { if (active) setBusy(false); });
    return () => { active = false; };
  }, [onError, open, workspaceId]);

  if (!open || !workspaceId) return null;
  const fieldClass = "h-10 w-full rounded-xl border border-white/[0.09] bg-white/[0.04] px-3 text-sm text-neutral-200 outline-none transition-colors focus:border-white/[0.18] disabled:opacity-45";
  const save = async () => {
    setBusy(true);
    try {
      const saved = await bridgeApi.updateRouterPreferences(workspaceId, {
        ...preferences,
        excludedHarnesses: parseList(excludedHarnesses),
        excludedModels: parseList(excludedModels),
      });
      setPreferences(saved);
      onClose();
    } catch (error) {
      onError(error instanceof Error ? error.message : String(error));
    } finally {
      setBusy(false);
    }
  };

  return <div className="fixed inset-0 z-50 flex items-start justify-center bg-black/70 p-4 pt-[7vh] backdrop-blur-md" role="dialog" aria-modal="true" aria-labelledby="router-settings-title" onMouseDown={event => { if (event.target === event.currentTarget) onClose(); }}>
    <div className="animate-page-enter w-full max-w-xl overflow-hidden rounded-3xl border border-white/[0.09] bg-[#121214]/95 shadow-2xl shadow-black/50">
      <header className="flex items-start gap-3 border-b border-white/[0.08] px-5 py-4">
        <span className="mt-0.5 flex h-9 w-9 shrink-0 items-center justify-center rounded-xl bg-violet-400/[0.09] text-violet-300"><BrainCircuit size={18} aria-hidden="true" /></span>
        <div className="min-w-0 flex-1"><h2 id="router-settings-title" className="font-display text-base font-semibold text-white">Learning router</h2><p className="mt-1 text-[13px] leading-relaxed text-neutral-500">Choose the least expensive route that preserves your measured quality floor.</p></div>
        <button type="button" className="rounded-xl p-2 text-neutral-500 hover:bg-white/[0.08] hover:text-neutral-200" onClick={onClose} aria-label="Close"><X size={16} aria-hidden="true" /></button>
      </header>
      <div className="max-h-[68vh] space-y-5 overflow-y-auto p-5">
        <div className="grid gap-4 sm:grid-cols-2">
          <label className="space-y-2 text-[11px] font-semibold uppercase tracking-wider text-neutral-500">Mode
            <select className={fieldClass} value={preferences.mode} disabled={busy} onChange={event => setPreferences(current => ({ ...current, mode: event.target.value as RouterMode }))}>
              <option value="disabled">Disabled</option><option value="shadow">Shadow</option><option value="autonomous">Autonomous</option>
            </select>
          </label>
          <label className="space-y-2 text-[11px] font-semibold uppercase tracking-wider text-neutral-500">Minimum pass probability
            <div className="relative"><input className={fieldClass} type="number" min={0} max={100} step={1} value={Math.round(preferences.minimumPassBps / 100)} disabled={busy} onChange={event => setPreferences(current => ({ ...current, minimumPassBps: Math.max(0, Math.min(10000, Number(event.target.value) * 100)) }))} /><span className="pointer-events-none absolute right-3 top-2.5 text-sm text-neutral-500">%</span></div>
          </label>
          <label className="space-y-2 text-[11px] font-semibold uppercase tracking-wider text-neutral-500">Pin harness
            <select className={fieldClass} value={preferences.pinnedHarness ?? ""} disabled={busy} onChange={event => setPreferences(current => ({ ...current, pinnedHarness: event.target.value || null, pinnedModel: null }))}>
              <option value="">Automatic</option>{adapters.map(adapter => <option key={adapter.id} value={adapter.id}>{adapter.label}{adapter.available ? "" : " (unavailable)"}</option>)}
            </select>
          </label>
          <label className="space-y-2 text-[11px] font-semibold uppercase tracking-wider text-neutral-500">Pin model
            <select className={fieldClass} value={preferences.pinnedModel ?? ""} disabled={busy} onChange={event => setPreferences(current => ({ ...current, pinnedModel: event.target.value || null }))}>
              <option value="">Automatic</option>{models.filter(model => !preferences.pinnedHarness || model.harness === preferences.pinnedHarness).map(model => <option key={`${model.harness}:${model.id}`} value={model.id}>{model.harnessLabel} · {model.label}</option>)}
            </select>
          </label>
        </div>
        <label className="block space-y-2 text-[11px] font-semibold uppercase tracking-wider text-neutral-500">Exclude harnesses <span className="normal-case tracking-normal text-neutral-600">comma-separated IDs</span><input className={fieldClass} value={excludedHarnesses} disabled={busy} placeholder="e.g. claude" onChange={event => setExcludedHarnesses(event.target.value)} /></label>
        <label className="block space-y-2 text-[11px] font-semibold uppercase tracking-wider text-neutral-500">Exclude models <span className="normal-case tracking-normal text-neutral-600">comma-separated IDs</span><input className={fieldClass} value={excludedModels} disabled={busy} placeholder="e.g. opus, gpt-5.3-codex" onChange={event => setExcludedModels(event.target.value)} /></label>
        <div className="flex gap-3 rounded-2xl border border-emerald-300/[0.09] bg-emerald-300/[0.04] p-4"><ShieldCheck className="mt-0.5 shrink-0 text-emerald-300" size={17} aria-hidden="true" /><p className="text-[12px] leading-relaxed text-neutral-400">Shadow mode measures recommendations without changing execution. Autonomous mode unlocks only after 20 completed shadow outcomes with fewer than 5% manual or no-route decisions. Pins never bypass permissions or budgets.</p></div>
      </div>
      <footer className="flex justify-end gap-2 border-t border-white/[0.08] px-5 py-4"><button type="button" className="rounded-xl px-4 py-2 text-sm text-neutral-500 hover:text-neutral-200" disabled={busy} onClick={onClose}>Cancel</button><button type="button" className="inline-flex min-w-24 items-center justify-center gap-2 rounded-xl bg-white px-4 py-2 text-sm font-medium text-neutral-900 disabled:opacity-40" disabled={busy} onClick={() => void save()}>{busy && <LoaderCircle className="animate-spin" size={14} aria-hidden="true" />}Save</button></footer>
    </div>
  </div>;
}
