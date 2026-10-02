import { useCallback, useEffect, useRef, useState } from "react";
import { ArrowLeft, ArrowRight, Bot, CheckCircle2, LoaderCircle, RefreshCw } from "lucide-react";
import { bridgeApi } from "../api";
import type { AdapterDescriptor, ModelSetupState } from "../types";
import type { UsageProvider } from "../usage";
import { canInstallManagedAgent, onboardingAgentReady, onboardingChoices, type OnboardingAgent } from "../onboarding";
import { recommendedProfileDrafts } from "../modelProfiles";
import { sourceLine, useManagedAgents } from "./ManagedAgentsPanel";
import { HarnessMark } from "./harnessMarks";
import { ProviderLoginPane } from "./ProviderLoginPane";

const NOOP = () => undefined;
const LOGIN_AGENTS = new Set(["claude", "codex", "opencode", "cursor", "grok"]);
const SESSION_SETUP_AGENTS = new Set(["cursor", "grok"]);
type Stage = "choose" | "connect";

function installationLabel(agent: OnboardingAgent) {
  return agent.installationSourceUnknown
    ? agent.backing === "none" ? "Installation not confirmed" : "Installation found; source could not be checked"
    : sourceLine(agent);
}

async function checkedHealth(agentIds: string[]) {
  const health = await bridgeApi.refreshModelCatalogs();
  for (const id of agentIds.filter(id => SESSION_SETUP_AGENTS.has(id))) {
    const prepared = await bridgeApi.prepareAgentSetup(id);
    health.adapters = health.adapters.map(adapter => adapter.id === id ? prepared : adapter);
  }
  return health;
}

function AgentConnection({ agent, adapter, busy, error, loginOpen, onInstall, onRepair, onLogin, onCloseLogin, onRefresh }: {
  agent: OnboardingAgent; adapter?: AdapterDescriptor; busy?: string; error?: string;
  loginOpen: boolean; onInstall: () => void; onRepair: () => void; onLogin: () => void;
  onCloseLogin: () => void; onRefresh: () => void;
}) {
  const missing = agent.backing === "none";
  const broken = agent.state === "repairable" || agent.state === "broken";
  const ready = onboardingAgentReady(agent, adapter);
  const supportedInstall = canInstallManagedAgent(agent);
  const supportsLogin = LOGIN_AGENTS.has(agent.agentId);
  return <article className="u-glass-soft rounded-2xl border border-border-card p-4" data-testid={`onboarding-agent-${agent.agentId}`}>
    <div className="flex items-start gap-3">
      <span className="grid size-10 shrink-0 place-items-center rounded-xl border border-border bg-background"><HarnessMark harness={agent.agentId} size={18} /></span>
      <div className="min-w-0 flex-1"><h2 className="text-sm font-medium">{agent.label}</h2><p className="mt-1 break-words text-xs text-muted-foreground">{installationLabel(agent)}</p></div>
      {ready && <CheckCircle2 size={18} className="shrink-0 text-success" aria-label="Sign-in found" />}
    </div>
    <div className="mt-4 space-y-2 border-t border-border pt-3">
      {busy ? <p role="status" className="flex items-center gap-2 text-xs text-muted-foreground"><LoaderCircle size={14} className="animate-spin" />{busy}…</p>
      : missing ? supportedInstall ? <><p className="text-xs text-muted-foreground">Bridge will download and manage a separate copy for you.</p><button type="button" onClick={onInstall} className="min-h-9 rounded-lg bg-primary px-3 text-xs font-medium text-primary-foreground">Install with Bridge</button></>
        : <p className="text-xs text-warning">Bridge cannot install this agent on this computer. Install it using the vendor's instructions, then check again, or go back and choose another agent.</p>
      : broken ? supportedInstall ? <button type="button" onClick={onRepair} className="min-h-9 rounded-lg border border-border px-3 text-xs">Repair installation</button>
        : <p className="text-xs text-warning">This installation needs attention. Repair it using the vendor's instructions, then check again.</p>
      : ready ? <p className="text-xs font-medium text-success">Sign-in found</p>
      : <>
        <p className="text-xs text-warning">{adapter?.authState === "signed_out" ? "Sign in to use this agent." : "Bridge cannot confirm sign-in yet. Sign in, then check again."}</p>
        {adapter?.unavailableReason && <p className="text-xs text-muted-foreground">{adapter.unavailableReason}</p>}
        {supportsLogin ? <button type="button" onClick={onLogin} disabled={loginOpen} className="min-h-9 rounded-lg bg-primary px-3 text-xs font-medium text-primary-foreground disabled:opacity-40">Sign in to {agent.label}</button>
          : <p className="text-xs text-muted-foreground">Bridge cannot open this agent's sign-in flow. Use its own app or CLI, then check again.</p>}
      </>}
      {!busy && !missing && <button type="button" onClick={onRefresh} className="min-h-8 rounded-lg px-2 text-xs text-muted-foreground hover:bg-accent">Check again</button>}
      {agent.vendorMessage && <p className="text-xs text-warning">{agent.vendorMessage}</p>}
      {error && <p role="alert" className="text-xs text-destructive">{error}</p>}
    </div>
    {loginOpen && <ProviderLoginPane provider={agent.agentId as UsageProvider | "grok"} label={agent.label} onClose={onCloseLogin} />}
  </article>;
}

export function ModelSetupWizard({ adapters, onComplete, onHealthChange = NOOP, onError }: {
  adapters: AdapterDescriptor[]; onComplete: (state: ModelSetupState) => void;
  onHealthChange?: () => void; onError: (message: string) => void;
}) {
  const [stage, setStage] = useState<Stage>("choose");
  const [selected, setSelected] = useState<string[]>([]);
  const [currentAdapters, setCurrentAdapters] = useState(adapters);
  const [refreshing, setRefreshing] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string>();
  const [login, setLogin] = useState<string | null>(null);
  const [useDetectedAgents, setUseDetectedAgents] = useState(false);
  const selection = useRef({ selected, stage });
  selection.current = { selected, stage };
  const refreshGeneration = useRef(0);
  useEffect(() => { setCurrentAdapters(adapters); }, [adapters]);
  useEffect(() => () => { refreshGeneration.current += 1; }, []);
  const refresh = useCallback((requested?: string[]) => {
    const generation = ++refreshGeneration.current;
    setRefreshing(true);
    const ids = requested ?? (selection.current.stage === "connect" ? selection.current.selected : []);
    void checkedHealth(ids).then(health => {
      if (generation === refreshGeneration.current) { setCurrentAdapters(health.adapters); setError(undefined); onHealthChange(); }
    }).catch(async cause => {
      // Failed ACP checks invalidate native auth/catalog facts. Read that result
      // so the card can offer sign-in instead of retaining an old ready badge.
      const health = await bridgeApi.health().catch(() => undefined);
      if (generation === refreshGeneration.current) {
        if (health) setCurrentAdapters(health.adapters);
        setError(cause instanceof Error ? cause.message : String(cause));
      }
    })
      .finally(() => { if (generation === refreshGeneration.current) setRefreshing(false); });
  }, [onHealthChange]);
  const managed = useManagedAgents(undefined, refresh);
  const choices = onboardingChoices(managed.listError ? [] : managed.agents ?? [], currentAdapters, !!managed.listError);
  const selectedAgents = choices.filter(agent => selected.includes(agent.agentId));
  const operating = Object.keys(managed.busy).length > 0;
  const ready = selected.length > 0 && selectedAgents.length === selected.length && selectedAgents.every(agent => onboardingAgentReady(agent, currentAdapters.find(adapter => adapter.id === agent.agentId)));
  const checkAgain = () => { managed.reload(); refresh(); };
  const closeLogin = () => { setLogin(null); checkAgain(); };
  const finish = async () => {
    if (!ready || saving || refreshing || operating || login) return;
    setSaving(true); setError(undefined);
    try {
      // Re-read both facts at the point of completion. A cancelled login, removed
      // executable, stale card, or expired availability probe cannot complete setup.
      const [list, health, config] = await Promise.all([
        bridgeApi.listManagedAgents().catch(cause => { if (useDetectedAgents) return null; throw cause; }),
        checkedHealth(selected), bridgeApi.configState(),
      ]);
      setCurrentAdapters(health.adapters);
      const freshChoices = onboardingChoices(list?.agents ?? [], health.adapters, !list);
      if (!selected.every(id => onboardingAgentReady(freshChoices.find(agent => agent.agentId === id), health.adapters.find(adapter => adapter.id === id)))) {
        managed.reload();
        throw new Error("One of your selected agents is not ready. Check its installation and sign-in, or choose another agent.");
      }
      const profiles = recommendedProfileDrafts(health.adapters.filter(adapter => selected.includes(adapter.id)), { allowAvailableFallback: true });
      // Preferences are written before model setup, whose successful save is
      // the durable completion marker. Failed setup remains retryable.
      for (const harness of config.harnesses) {
        if (harness.id === "bridge") continue;
        const enabled = selected.includes(harness.id);
        if (harness.enabled !== enabled) await bridgeApi.saveHarnessConfig({ ...harness, enabled });
      }
      const setup = await bridgeApi.saveModelProfiles(profiles);
      if (!setup.complete) throw new Error("Bridge could not finish model setup. Please try again.");
      onHealthChange();
      onComplete(setup);
    } catch (cause) {
      const message = cause instanceof Error ? cause.message : String(cause);
      managed.reload();
      const health = await bridgeApi.health().catch(() => undefined);
      if (health) setCurrentAdapters(health.adapters);
      setError(message); onError(message);
    } finally { setSaving(false); }
  };
  return <main className="relative z-50 flex h-[100dvh] w-full flex-col overflow-y-auto bg-background px-4 py-8 text-foreground">
    <div className="mx-auto my-auto w-full max-w-2xl">
      <header className="mb-6 flex items-center justify-between gap-3"><span className="inline-flex items-center gap-2 font-display text-base font-semibold"><Bot size={18} />Bridge</span><span className="text-xs text-muted-foreground">{stage === "choose" ? "1. Choose agents" : "2. Install & sign in"}</span></header>
      <section className="u-glass-popover rounded-3xl border border-border-card p-5 sm:p-8">
        <h1 className="font-display text-2xl font-semibold tracking-tight">{stage === "choose" ? "Which coding agents do you want to use?" : "Connect your coding agents"}</h1>
        <p className="mt-2 text-sm leading-relaxed text-muted-foreground">{stage === "choose" ? "Choose one or more. We'll help you install what's missing and sign in." : "Each selected agent needs an installation and sign-in before you enter Bridge."}</p>
        {stage === "choose" && <div className="mt-5 rounded-xl border border-border bg-muted/30 p-4 text-xs leading-relaxed text-muted-foreground"><p><strong className="text-foreground">Already on your computer:</strong> Bridge uses the copy you installed. You manage its updates.</p><p className="mt-2"><strong className="text-foreground">Installed by Bridge:</strong> a separate copy Bridge can update and remove. You still sign in with your agent account.</p></div>}
        {managed.listError && <div className="mt-4 space-y-2"><p role="alert" className="text-sm text-destructive">Could not check installation sources: {managed.listError}</p><p className="text-xs text-muted-foreground">You can retry, or use agents found by the sign-in and availability check. Bridge cannot manage those installations until detection recovers.</p>{!useDetectedAgents && <button type="button" onClick={() => setUseDetectedAgents(true)} className="min-h-9 rounded-lg border border-border px-3 text-xs">Use detected agents</button>}</div>}
        {!managed.agents && !managed.listError ? <p role="status" className="mt-6 flex items-center gap-2 text-sm text-muted-foreground"><LoaderCircle size={14} className="animate-spin" />Checking this computer…</p>
          : stage === "choose" ? <fieldset className="mt-5 space-y-2"><legend className="sr-only">Coding agents to use</legend>{choices.map(agent => <label key={agent.agentId} className="u-glass-soft flex cursor-pointer items-center gap-3 rounded-xl border border-border-card p-4">
            <input type="checkbox" checked={selected.includes(agent.agentId)} onChange={event => { setError(undefined); setSelected(ids => event.target.checked ? [...ids, agent.agentId] : ids.filter(id => id !== agent.agentId)); }} className="size-4 accent-primary" />
            <HarnessMark harness={agent.agentId} size={18} /><span className="min-w-0 flex-1"><span className="block text-sm font-medium">{agent.label}</span><span className="mt-1 block text-xs text-muted-foreground">{installationLabel(agent)}</span></span>
          </label>)}</fieldset>
          : <div className="mt-5 space-y-3">{selectedAgents.map(agent => <AgentConnection key={agent.agentId} agent={agent} adapter={currentAdapters.find(adapter => adapter.id === agent.agentId)} busy={managed.busy[agent.agentId] ?? (refreshing && SESSION_SETUP_AGENTS.has(agent.agentId) ? "Checking sign-in and models" : undefined)} error={managed.errors[agent.agentId]} loginOpen={login === agent.agentId} onInstall={() => managed.install(agent)} onRepair={() => managed.repair(agent)} onLogin={() => setLogin(agent.agentId)} onCloseLogin={closeLogin} onRefresh={checkAgain} />)}</div>}
        {error && <p role="alert" className="mt-4 text-sm text-destructive">{error}</p>}
        <footer className="mt-6 flex flex-wrap items-center justify-between gap-3 border-t border-border pt-4">
          <div className="flex gap-2">{stage === "connect" && <button type="button" disabled={saving || operating} onClick={() => { if (login) void bridgeApi.cancelProviderLogin(login); setLogin(null); setStage("choose"); }} className="inline-flex min-h-9 items-center gap-1 rounded-lg px-2 text-xs text-muted-foreground disabled:opacity-40"><ArrowLeft size={13} />Change agents</button>}
            <button type="button" disabled={refreshing || saving || operating} onClick={checkAgain} className="inline-flex min-h-9 items-center gap-2 rounded-lg px-2 text-xs text-muted-foreground disabled:opacity-40"><RefreshCw size={13} className={refreshing ? "animate-spin" : ""} />{refreshing ? "Checking…" : "Check again"}</button></div>
          {stage === "choose" ? <button type="button" disabled={((!managed.agents || !!managed.listError) && !useDetectedAgents) || selected.length === 0 || refreshing} onClick={() => { setStage("connect"); refresh(selected); }} className="inline-flex min-h-9 items-center gap-2 rounded-lg bg-primary px-4 text-sm font-medium text-primary-foreground disabled:opacity-40">Continue<ArrowRight size={14} /></button>
            : <button type="button" disabled={!ready || saving || refreshing || operating || !!login || (!!managed.listError && !useDetectedAgents)} onClick={() => void finish()} className="inline-flex min-h-9 items-center gap-2 rounded-lg bg-primary px-4 text-sm font-medium text-primary-foreground disabled:opacity-40">{saving && <LoaderCircle size={14} className="animate-spin" />}{saving ? "Finishing setup…" : "Start using Bridge"}</button>}
        </footer>
        {stage === "connect" && <p className="mt-3 text-xs leading-relaxed text-muted-foreground">Bridge checks local sign-in information, not your subscription or remaining credits. Model defaults are chosen automatically; you can change them later in Settings.</p>}
        {selected.some(id => SESSION_SETUP_AGENTS.has(id)) && <p className="mt-3 text-xs leading-relaxed text-muted-foreground">For Cursor and Grok, checking opens a temporary agent session to confirm sign-in and models, then closes it without sending a chat message. It may start tools already configured in that agent.</p>}
      </section>
    </div>
  </main>;
}
