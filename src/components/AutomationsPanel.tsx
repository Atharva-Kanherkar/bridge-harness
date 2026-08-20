import { useCallback, useEffect, useMemo, useState } from "react";
import { AlertTriangle, Check, ChevronDown, Clock, LoaderCircle, Pause, Play, RefreshCw, Trash2, X } from "lucide-react";
import { bridgeApi } from "../api";
import type { AutomationAction, AutomationActionResult, AutomationCatalog, AutomationProvider, UnifiedAutomation } from "../types";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogPanel, DialogTitle } from "@/components/ui/dialog";

const providerLabel = (provider: AutomationProvider) => provider === "codex" ? "Codex" : provider === "claude" ? "Claude Code" : "OpenCode";

function relativeTime(epochMs: number | null, now: number): string | null {
  if (!epochMs) return null;
  const delta = epochMs - now;
  const magnitude = Math.abs(delta);
  const unit = magnitude < 60_000 ? [1_000, "s"] as const : magnitude < 3_600_000 ? [60_000, "m"] as const : magnitude < 86_400_000 ? [3_600_000, "h"] as const : [86_400_000, "d"] as const;
  const amount = Math.max(1, Math.round(magnitude / unit[0]));
  return delta >= 0 ? `in ${amount}${unit[1]}` : `${amount}${unit[1]} ago`;
}

export function AutomationsPanel({ initialCatalog }: { initialCatalog?: AutomationCatalog } = {}) {
  const [catalog, setCatalog] = useState<AutomationCatalog | undefined>(initialCatalog);
  const [provider, setProvider] = useState<AutomationProvider | "all">("all");
  const [expanded, setExpanded] = useState<string>();
  const [confirmDelete, setConfirmDelete] = useState<UnifiedAutomation>();
  const [results, setResults] = useState<AutomationActionResult[]>([]);
  const [failure, setFailure] = useState<string>();
  const [busy, setBusy] = useState(false);
  const now = Date.now();
  const refresh = useCallback(async () => { try { setCatalog(await bridgeApi.automationCatalog()); setFailure(undefined); } catch (error) { setFailure(error instanceof Error ? error.message : String(error)); } }, []);
  useEffect(() => { void refresh(); }, [refresh]);
  const automations = useMemo(() => (catalog?.automations ?? []).filter(automation => provider === "all" || automation.provider === provider), [catalog, provider]);
  const act = async (automation: UnifiedAutomation, action: AutomationAction) => {
    setBusy(true); setFailure(undefined);
    try { const result = await bridgeApi.executeAutomationAction(automation.provider, automation.id, action); setResults(current => [...current, result]); await refresh(); }
    catch (error) { setFailure(error instanceof Error ? error.message : String(error)); }
    finally { setBusy(false); setConfirmDelete(undefined); }
  };
  return <div className="h-full min-h-0 overflow-y-auto">
    <main className="mx-auto w-full max-w-5xl px-3 pb-16 pt-8 sm:px-6 sm:pt-12">
      <div className="flex flex-wrap items-start justify-between gap-4"><div className="min-w-0"><h1 className="font-display text-[26px] font-semibold tracking-[-0.025em] text-foreground sm:text-[32px]">Automations</h1><p className="mt-1.5 text-[13.5px] leading-relaxed text-muted-foreground">Every scheduled job your agents already run — Claude Code and Codex schedules, one view. Each app stays the scheduler.</p></div><Button size="xs" variant="secondary" disabled={busy} onClick={() => void refresh()}><RefreshCw size={11}/>Refresh</Button></div>
      <div className="mt-5 flex flex-wrap items-center gap-2">{(catalog?.providers ?? []).map(state => <span key={state.provider} title={state.detail} className={`inline-flex items-center gap-1.5 rounded-full border px-2.5 py-1 text-[10px] font-medium ${state.available ? "border-border bg-muted text-muted-foreground" : "border-border bg-muted text-muted-foreground/50"}`}><span className={`h-1.5 w-1.5 rounded-full ${state.available ? "bg-success" : "bg-muted-foreground/40"}`}/>{providerLabel(state.provider)}{state.available ? ` · ${state.count}` : " · unavailable"}</span>)}</div>
      <div className="mt-5 u-segmented w-fit">{(["all", "claude", "codex"] as const).map(value => <button key={value} data-active={provider === value} onClick={() => setProvider(value)} className="u-segmented-item">{value === "all" ? "All" : providerLabel(value)}</button>)}</div>
      {failure && <div className="mt-3 flex items-start gap-2 rounded-xl border border-destructive/30 bg-destructive/10 px-3 py-2 text-[10.5px] text-destructive"><AlertTriangle className="mt-0.5 shrink-0" size={12}/><span className="min-w-0 break-words">{failure}</span></div>}
      {!catalog && <div className="flex min-h-56 items-center justify-center gap-2 text-xs text-muted-foreground"><LoaderCircle className="animate-spin" size={15}/>Reading local schedules…</div>}
      {catalog && automations.length === 0 && <p className="py-16 text-center text-xs text-muted-foreground/70">No automations yet. Schedule one in Claude Code or Codex and it appears here.</p>}
      {catalog && automations.length > 0 && <div className="mt-4 grid grid-cols-1 gap-3 sm:grid-cols-2">{automations.map(automation => {
        const key = `${automation.provider}:${automation.id}`;
        const open = expanded === key;
        const nextRun = relativeTime(automation.nextRunAt, now);
        const lastRun = relativeTime(automation.lastRunAt, now);
        return <article key={key} className={`rounded-2xl border bg-card px-4 py-3.5 transition-colors duration-200 ${open ? "border-input bg-accent" : "border-border hover:border-input hover:bg-accent"}`}>
          <button type="button" onClick={() => setExpanded(open ? undefined : key)} className="flex w-full items-start gap-3.5 text-left">
            <div className="mt-0.5 flex h-9 w-9 shrink-0 items-center justify-center rounded-[12px] border border-border bg-accent text-muted-foreground"><Clock size={15}/></div>
            <div className="min-w-0 flex-1">
              <div className="flex items-center gap-2">
                <h3 className="truncate text-[13.5px] font-semibold tracking-[-0.008em] text-foreground">{automation.name}</h3>
                <span className={`inline-flex shrink-0 items-center gap-1 rounded-full border px-1.5 py-0.5 text-[8.5px] font-medium uppercase tracking-[0.04em] ${automation.status === "active" ? "border-success/30 bg-success/10 text-success" : automation.status === "paused" ? "border-warning/30 bg-warning/10 text-warning" : "border-border bg-muted text-muted-foreground"}`}>{automation.status}</span>
              </div>
              <p className={`mt-1 text-[11.5px] leading-[1.55] text-muted-foreground ${open ? "" : "truncate"}`}>{automation.schedule.human}{!automation.recurring && " · one-shot"}</p>
              <div className="mt-2 flex flex-wrap items-center gap-x-2 gap-y-1 text-[10px] text-muted-foreground/70">
                <span className="rounded-full border border-border bg-muted px-2 py-0.5 text-[9px] text-muted-foreground">{providerLabel(automation.provider)}</span>
                {nextRun && <span>next {nextRun}</span>}
                {lastRun && <><span className="text-muted-foreground/40">·</span><span>last ran {lastRun}</span></>}
                {automation.model && <><span className="text-muted-foreground/40">·</span><span className="font-mono">{automation.model}</span></>}
              </div>
            </div>
            <ChevronDown size={13} className={`mt-1 shrink-0 text-muted-foreground/70 transition-transform ${open ? "rotate-180" : ""}`}/>
          </button>
          {open && <div className="mt-3 rounded-xl border border-border bg-background p-3.5 text-[10.5px] text-muted-foreground sm:ml-[50px]">
            <p className="mb-1.5 text-[9px] font-semibold uppercase tracking-[0.12em] text-muted-foreground/70">Prompt</p>
            <p className="whitespace-pre-wrap break-words leading-relaxed">{automation.prompt}</p>
            <div className="mt-2 flex flex-wrap items-center gap-x-3 gap-y-1 font-mono text-[9px]"><span>{automation.schedule.kind}: {automation.schedule.expression}</span>{automation.effort && <span>effort {automation.effort}</span>}</div>
            {automation.cwds.length > 0 && <><p className="mb-1 mt-2.5 text-[9px] font-semibold uppercase tracking-[0.12em] text-muted-foreground/70">Runs in</p><div className="flex flex-wrap gap-1.5">{automation.cwds.map(cwd => <span key={cwd} className="rounded-full border border-border bg-card px-2 py-0.5 font-mono text-[9px]">{cwd}</span>)}</div></>}
            {automation.runs.length > 0 && <><p className="mb-1 mt-2.5 text-[9px] font-semibold uppercase tracking-[0.12em] text-muted-foreground/70">Recent runs</p><ul className="space-y-1">{automation.runs.map(run => <li key={run.id} className="flex items-start gap-2"><span className={`mt-1 h-1.5 w-1.5 shrink-0 rounded-full ${run.status.toLowerCase().includes("complete") ? "bg-success" : run.status.toLowerCase().includes("fail") ? "bg-destructive" : "bg-muted-foreground/40"}`}/><span className="min-w-0 break-words">{run.title ?? run.id}{run.summary && <span className="text-muted-foreground/70"> — {run.summary}</span>}{run.createdAt && <span className="text-muted-foreground/50"> · {relativeTime(run.createdAt, now)}</span>}</span></li>)}</ul></>}
            <div className="mt-3 flex flex-wrap gap-1.5">
              {automation.canPause && automation.status === "active" && <Button size="xs" variant="secondary" disabled={busy} onClick={() => void act(automation, "pause")}><Pause size={10}/>Pause</Button>}
              {automation.canPause && automation.status === "paused" && <Button size="xs" disabled={busy} onClick={() => void act(automation, "resume")}><Play size={10}/>Resume</Button>}
              {!automation.canPause && <span className="self-center text-[9px] text-muted-foreground/60">{providerLabel(automation.provider)}'s format has no paused state</span>}
              <Button size="xs" variant="ghost" disabled={busy} onClick={() => setConfirmDelete(automation)}><Trash2 size={10}/>Delete</Button>
            </div>
          </div>}
        </article>;
      })}</div>}
      {!!results.length && <div className="u-overlay-strong fixed bottom-5 right-5 z-40 w-80 max-w-[calc(100vw-2.5rem)] rounded-2xl p-3.5"><button onClick={() => setResults([])} className="absolute right-2.5 top-2.5 text-muted-foreground transition-colors hover:text-foreground"><X size={12}/></button>{results.map((result, index) => <p key={`${result.provider}:${result.id}:${result.action}:${index}`} className="flex gap-2 py-1 text-[10.5px] text-muted-foreground">{result.success ? <Check size={12} className="shrink-0 text-success"/> : <AlertTriangle size={12} className="shrink-0 text-destructive"/>}<span className="min-w-0 break-words"><b className="text-foreground">{providerLabel(result.provider)}:</b> {result.message}</span></p>)}</div>}
    </main>
    <Dialog open={!!confirmDelete} onOpenChange={open => { if (!open && !busy) setConfirmDelete(undefined); }}>{confirmDelete && <DialogContent showCloseButton={!busy}><DialogHeader><div className="flex items-start gap-3"><AlertTriangle className="mt-0.5 shrink-0 text-warning" size={16}/><div><DialogTitle className="font-display text-base font-medium text-foreground">Delete this automation?</DialogTitle><DialogDescription className="mt-1 text-[11px] leading-5">This removes it from {providerLabel(confirmDelete.provider)}'s own schedule — the app will no longer run it. There is no undo from Bridge.</DialogDescription></div></div></DialogHeader><DialogPanel className="py-0"><dl className="grid grid-cols-[72px_1fr] gap-x-3 gap-y-2 rounded-xl border border-border bg-muted p-3 text-[10px] sm:grid-cols-[88px_1fr]"><dt className="text-muted-foreground">Name</dt><dd className="break-words text-foreground">{confirmDelete.name}</dd><dt className="text-muted-foreground">Schedule</dt><dd className="text-muted-foreground">{confirmDelete.schedule.human}</dd><dt className="text-muted-foreground">App</dt><dd className="text-muted-foreground">{providerLabel(confirmDelete.provider)}</dd></dl></DialogPanel><DialogFooter variant="bare"><Button variant="ghost" disabled={busy} onClick={() => setConfirmDelete(undefined)}>Cancel</Button><Button variant="destructive" disabled={busy} onClick={() => void act(confirmDelete, "delete")}>{busy && <LoaderCircle className="animate-spin" size={12}/>}Delete</Button></DialogFooter></DialogContent>}</Dialog>
  </div>;
}
