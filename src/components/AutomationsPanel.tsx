import { SCREEN_CONTENT, ScreenHeading } from "./ui/screen";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { AlertTriangle, Check, ChevronDown, Clock, LoaderCircle, Pause, Pencil, Play, Plus, RefreshCw, Rocket, Trash2, X } from "lucide-react";
import { bridgeApi } from "../api";
import type { AutomationAction, AutomationActionResult, AutomationCapability, AutomationCatalog, AutomationProvider, SaveAutomationParams, UnifiedAutomation } from "../types";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogPanel, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";

const PROVIDER_LABELS: Record<AutomationProvider, string> = { claude: "Claude Code", codex: "Codex", cursor: "Cursor", opencode: "OpenCode" };
const providerLabel = (provider: AutomationProvider) => PROVIDER_LABELS[provider] ?? provider;
const CRON_HINT = "Five fields: minute hour day-of-month month day-of-week.";
const looksLikeCron = (expression: string) => expression.trim().split(/\s+/).length === 5;
const capabilityLabel = (capability: AutomationCapability) => capability === "runNow" ? "Run now" : capability[0].toUpperCase() + capability.slice(1);

function relativeTime(epochMs: number | null, now: number): string | null {
  if (!epochMs) return null;
  const delta = epochMs - now;
  const magnitude = Math.abs(delta);
  const unit = magnitude < 60_000 ? [1_000, "s"] as const : magnitude < 3_600_000 ? [60_000, "m"] as const : magnitude < 86_400_000 ? [3_600_000, "h"] as const : [86_400_000, "d"] as const;
  const amount = Math.max(1, Math.round(magnitude / unit[0]));
  return delta >= 0 ? `in ${amount}${unit[1]}` : `${amount}${unit[1]} ago`;
}

type DraftState = SaveAutomationParams & { mode: "create" | "edit" };

export function AutomationsPanel({ initialCatalog }: { initialCatalog?: AutomationCatalog } = {}) {
  const [catalog, setCatalog] = useState<AutomationCatalog | undefined>(initialCatalog);
  const [provider, setProvider] = useState<AutomationProvider | "all">("all");
  const [status, setStatus] = useState<UnifiedAutomation["status"] | "all">("all");
  const [expanded, setExpanded] = useState<string>();
  const [confirmDelete, setConfirmDelete] = useState<UnifiedAutomation>();
  const [draft, setDraft] = useState<DraftState>();
  const [results, setResults] = useState<Array<Pick<AutomationActionResult, "provider" | "message"> & { key: string }>>([]);
  const [failure, setFailure] = useState<string>();
  const [busy, setBusy] = useState(false);
  const [loading, setLoading] = useState(!initialCatalog);
  const refreshGeneration = useRef(0);
  const now = Date.now();

  const refresh = useCallback(async () => {
    const generation = ++refreshGeneration.current;
    setLoading(true);
    try {
      const next = await bridgeApi.automationCatalog();
      if (generation !== refreshGeneration.current) return;
      setCatalog(next);
      setFailure(undefined);
    } catch (error) {
      if (generation === refreshGeneration.current) setFailure(error instanceof Error ? error.message : String(error));
    } finally {
      if (generation === refreshGeneration.current) setLoading(false);
    }
  }, []);

  useEffect(() => { void refresh(); }, [refresh]);

  const automations = useMemo(
    () => (catalog?.automations ?? []).filter(automation =>
      (provider === "all" || automation.provider === provider)
      && (status === "all" || automation.status === status)),
    [catalog, provider, status],
  );
  const providerState = (owner: AutomationProvider) => catalog?.providers.find(state => state.provider === owner);
  const supports = (owner: AutomationProvider, capability: AutomationCapability) => {
    const state = providerState(owner);
    // An unreadable store advertises its shape, not a working control.
    return !!state?.available && state.capabilities.includes(capability);
  };
  const canCreateClaude = supports("claude", "create");

  const act = async (automation: UnifiedAutomation, action: AutomationAction) => {
    setBusy(true); setFailure(undefined);
    try {
      const result = await bridgeApi.executeAutomationAction(automation.provider, automation.id, action);
      setResults(current => [...current, { provider: result.provider, message: result.message, key: `${result.id}:${result.action}:${Date.now()}` }]);
      await refresh();
    } catch (error) {
      setFailure(error instanceof Error ? error.message : String(error));
    } finally {
      setBusy(false); setConfirmDelete(undefined);
    }
  };

  const openCreate = () => setDraft({ mode: "create", provider: "claude", prompt: "", scheduleExpression: "0 9 * * 1-5", recurring: true });
  const openEdit = (automation: UnifiedAutomation) => setDraft({
    mode: "edit", provider: automation.provider, id: automation.id, prompt: automation.prompt,
    scheduleExpression: automation.schedule.expression, recurring: automation.recurring,
  });
  const saveDraft = async () => {
    if (!draft) return;
    setBusy(true); setFailure(undefined);
    try {
      const { mode: _mode, ...payload } = draft;
      const result = await bridgeApi.saveAutomation(payload);
      setResults(current => [...current, { provider: result.provider, message: result.message, key: `${result.id}:save:${Date.now()}` }]);
      setDraft(undefined);
      await refresh();
    } catch (error) {
      setFailure(error instanceof Error ? error.message : String(error));
    } finally {
      setBusy(false);
    }
  };

  return <div className="h-full min-h-0 overflow-y-auto">
    <div className={SCREEN_CONTENT}>
      <ScreenHeading title="Automations" description="View and manage schedules saved by your coding agents." action={<>
        {canCreateClaude && <Button size="xs" disabled={busy} onClick={openCreate}><Plus size={13}/>New Claude automation</Button>}
        <Button size="xs" variant="secondary" disabled={busy} onClick={() => void refresh()}><RefreshCw size={13}/>Refresh</Button>
      </>} />

      <div className="mt-5 flex flex-wrap gap-2">
        <div className="u-segmented flex-wrap w-fit" aria-label="Filter automations by provider">{(["all", ...(catalog?.providers ?? []).map(state => state.provider)] as Array<AutomationProvider | "all">).map(value => <button type="button" key={value} data-active={provider === value} aria-pressed={provider === value} onClick={() => setProvider(value)} className="u-segmented-item">{value === "all" ? "All providers" : providerLabel(value)}</button>)}</div>
        <div className="u-segmented flex-wrap w-fit" aria-label="Filter automations by status">{(["all", "active", "paused"] as const).map(value => <button type="button" key={value} data-active={status === value} aria-pressed={status === value} onClick={() => setStatus(value)} className="u-segmented-item">{value === "all" ? "All statuses" : value[0].toUpperCase() + value.slice(1)}</button>)}</div>
      </div>

      {failure && <div className="mt-3 flex items-start gap-2 rounded-xl border border-destructive/30 bg-destructive/10 px-3 py-2 text-caption text-destructive"><AlertTriangle className="mt-0.5 shrink-0" size={12}/><span className="min-w-0 break-words">{failure}</span></div>}
      {loading && !catalog && <div className="flex min-h-56 items-center justify-center gap-2 text-xs text-muted-foreground"><LoaderCircle className="animate-spin" size={15}/>Reading native schedules…</div>}
      {!loading && !catalog && <div className="flex min-h-56 flex-col items-center justify-center gap-3 text-center text-xs text-muted-foreground"><p>Bridge could not read the native schedules.</p><Button size="xs" variant="secondary" onClick={() => void refresh()}><RefreshCw size={11}/>Try again</Button></div>}
      {catalog && automations.length === 0 && <p className="py-16 text-center text-xs text-muted-foreground">{catalog.automations.length === 0 ? "No native automations yet." : "No automations match these filters."}</p>}
      {catalog && automations.length > 0 && <div className="mt-4 divide-y divide-border overflow-hidden rounded-xl border border-border bg-card">{automations.map(automation => {
        const key = `${automation.provider}:${automation.id}`;
        const open = expanded === key;
        const nextRun = relativeTime(automation.nextRunAt, now);
        const lastRun = relativeTime(automation.lastRunAt, now);
        return <article key={key} className={`px-4 py-3.5 transition-colors ${open ? "bg-accent" : "hover:bg-accent"}`}>
          <button type="button" aria-expanded={open} onClick={() => setExpanded(open ? undefined : key)} className="flex w-full items-start gap-3.5 text-left">
            <div className="mt-0.5 flex h-9 w-9 shrink-0 items-center justify-center rounded-[12px] border border-border bg-accent text-muted-foreground"><Clock size={15}/></div>
            <div className="min-w-0 flex-1"><div className="flex items-center gap-2"><h3 className="truncate text-[13.5px] font-semibold tracking-[-0.008em] text-foreground">{automation.name}</h3><span className={`inline-flex shrink-0 rounded-full border px-1.5 py-0.5 text-caption font-medium uppercase tracking-[0.04em] ${automation.status === "active" ? "border-success/30 bg-success/10 text-success" : automation.status === "paused" ? "border-warning/30 bg-warning/10 text-warning" : "border-border bg-muted text-muted-foreground"}`}>{automation.status}</span></div><p className={`mt-1 text-[12px] leading-[1.55] text-muted-foreground ${open ? "" : "truncate"}`}>{automation.schedule.human}{!automation.recurring && " · one-shot"}</p><div className="mt-2 flex flex-wrap items-center gap-x-2 gap-y-1 text-caption text-muted-foreground"><span className="rounded-full border border-border bg-muted px-2 py-0.5 text-caption text-muted-foreground">{providerLabel(automation.provider)}</span>{nextRun && <span>next {nextRun}</span>}{lastRun && <span>last ran {lastRun}</span>}{automation.model && <span className="font-mono">{automation.model}</span>}</div></div>
            <ChevronDown size={13} className={`mt-1 shrink-0 text-muted-foreground transition-transform ${open ? "rotate-180" : ""}`}/>
          </button>
          {open && <div className="mt-3 rounded-xl border border-border bg-background p-3.5 text-caption text-muted-foreground sm:ml-[50px]">
            <p className="mb-1.5 text-[12px] font-medium text-muted-foreground">Prompt</p><p className="whitespace-pre-wrap break-words leading-relaxed">{automation.prompt}</p>
            <div className="mt-2 flex flex-wrap items-center gap-x-3 gap-y-1 font-mono text-caption"><span>{automation.schedule.kind}: {automation.schedule.expression}</span>{automation.effort && <span>effort {automation.effort}</span>}</div>
            {automation.cwds.length > 0 && <div className="mt-2.5 flex flex-wrap gap-1.5">{automation.cwds.map(cwd => <span key={cwd} className="rounded-full border border-border bg-card px-2 py-0.5 font-mono text-caption">{cwd}</span>)}</div>}
            {automation.runs.length > 0 && <ul className="mt-2.5 space-y-1">{automation.runs.map(run => <li key={run.id} className="flex items-start gap-2"><span className={`mt-1 h-1.5 w-1.5 shrink-0 rounded-full ${run.status.toLowerCase().includes("complete") ? "bg-success" : run.status.toLowerCase().includes("fail") ? "bg-destructive" : "bg-muted-foreground/40"}`}/><span>{run.title ?? run.id}{run.summary && ` — ${run.summary}`}</span></li>)}</ul>}
            <div className="mt-3 flex flex-wrap gap-1.5">
              {supports(automation.provider, "edit") && <Button size="xs" variant="secondary" disabled={busy} onClick={() => openEdit(automation)}><Pencil size={10}/>Edit</Button>}
              {supports(automation.provider, "runNow") && <Button size="xs" disabled={busy} onClick={() => void act(automation, "runNow")}><Rocket size={10}/>Run now</Button>}
              {automation.status === "active" && supports(automation.provider, "pause") && <Button size="xs" variant="secondary" disabled={busy} onClick={() => void act(automation, "pause")}><Pause size={10}/>Pause</Button>}
              {automation.status === "paused" && supports(automation.provider, "resume") && <Button size="xs" disabled={busy} onClick={() => void act(automation, "resume")}><Play size={10}/>Resume</Button>}
              {supports(automation.provider, "delete") && <Button size="xs" variant="ghost" disabled={busy} onClick={() => setConfirmDelete(automation)}><Trash2 size={10}/>Delete</Button>}
            </div>
          </div>}
        </article>;
      })}</div>}
      <details className="mt-5 text-caption text-muted-foreground">
        <summary className="min-h-8 w-fit cursor-pointer rounded py-1.5">Provider support</summary>
      <div className="divide-y divide-border overflow-hidden rounded-xl border border-border bg-card" aria-label="Automation provider capabilities">
        {(catalog?.providers ?? []).map(state => <section key={state.provider} className="px-4 py-3" aria-label={`${providerLabel(state.provider)} automation support`}>
          <div className="flex items-center justify-between gap-2 text-caption font-medium text-foreground"><span>{providerLabel(state.provider)}</span><span className={`h-1.5 w-1.5 rounded-full ${state.available ? "bg-success" : "bg-muted-foreground/40"}`}/></div>
          <p className="mt-1 text-caption leading-relaxed text-muted-foreground" title={state.detail}>{state.available ? `${state.count} native schedule${state.count === 1 ? "" : "s"}` : state.detail}</p>
          <div className="mt-2 flex flex-wrap gap-1">{state.available && state.capabilities.length ? state.capabilities.map(capability => <span key={capability} className="rounded-full border border-border bg-muted px-1.5 py-0.5 text-caption text-muted-foreground">{capabilityLabel(capability)}</span>) : <span className="text-caption text-muted-foreground">No native automation controls</span>}</div>
        </section>)}
      </div>

      </details>
      {!!results.length && <div className="u-glass-popover fixed bottom-5 right-5 z-40 w-80 max-w-[calc(100vw-2.5rem)] rounded-2xl p-3.5"><button type="button" aria-label="Dismiss automation results" onClick={() => setResults([])} className="absolute right-1 top-1 grid size-7 place-items-center rounded-md text-muted-foreground transition-colors hover:text-foreground"><X size={12}/></button>{results.map(result => <p key={result.key} className="flex gap-2 py-1 text-caption text-muted-foreground"><Check size={12} className="shrink-0 text-success"/><span><b className="text-foreground">{providerLabel(result.provider)}:</b> {result.message}</span></p>)}</div>}
    </div>

    <Dialog open={!!draft} onOpenChange={open => { if (!open && !busy) setDraft(undefined); }}>{draft && <DialogContent showCloseButton={!busy}><DialogHeader><DialogTitle>{draft.mode === "create" ? "New Claude automation" : "Edit Claude automation"}</DialogTitle><DialogDescription>Saved directly to Claude Code's native schedule file. Claude Code remains responsible for execution.</DialogDescription></DialogHeader><DialogPanel className="space-y-3"><label className="block text-caption font-medium text-muted-foreground">Prompt<Textarea aria-label="Automation prompt" className="mt-1" value={draft.prompt} onChange={event => setDraft(current => current ? { ...current, prompt: event.target.value } : current)}/></label><label className="block text-caption font-medium text-muted-foreground">Cron schedule<Input aria-label="Automation cron schedule" className="mt-1 font-mono" value={draft.scheduleExpression} onChange={event => setDraft(current => current ? { ...current, scheduleExpression: event.target.value } : current)}/>{!!draft.scheduleExpression.trim() && !looksLikeCron(draft.scheduleExpression) && <p className="mt-1 text-caption text-destructive">{CRON_HINT}</p>}</label><label className="flex items-center gap-2 text-caption text-muted-foreground"><input type="checkbox" checked={draft.recurring} onChange={event => setDraft(current => current ? { ...current, recurring: event.target.checked } : current)} className="size-3.5 rounded border-border accent-foreground"/>Recurring</label></DialogPanel><DialogFooter><Button variant="ghost" disabled={busy} onClick={() => setDraft(undefined)}>Cancel</Button><Button disabled={busy || !draft.prompt.trim() || !looksLikeCron(draft.scheduleExpression)} onClick={() => void saveDraft()}>{busy && <LoaderCircle className="animate-spin" size={12}/>}Save in Claude Code</Button></DialogFooter></DialogContent>}</Dialog>
    <Dialog open={!!confirmDelete} onOpenChange={open => { if (!open && !busy) setConfirmDelete(undefined); }}>{confirmDelete && <DialogContent showCloseButton={!busy}><DialogHeader><DialogTitle>Delete this automation?</DialogTitle><DialogDescription>This removes it from {providerLabel(confirmDelete.provider)}'s native schedule. There is no undo from Bridge.</DialogDescription></DialogHeader><DialogPanel><p className="rounded-xl border border-border bg-muted p-3 text-caption text-foreground">{confirmDelete.name}</p></DialogPanel><DialogFooter><Button variant="ghost" disabled={busy} onClick={() => setConfirmDelete(undefined)}>Cancel</Button><Button variant="destructive" disabled={busy} onClick={() => void act(confirmDelete, "delete")}>Delete</Button></DialogFooter></DialogContent>}</Dialog>
  </div>;
}
