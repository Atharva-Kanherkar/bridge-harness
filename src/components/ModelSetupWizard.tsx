import { useEffect, useMemo, useRef, useState } from "react";
import { ArrowLeft, ArrowRight, Bot, Check, CheckCircle2, LoaderCircle, Search, Settings2, Sparkles, Wrench } from "lucide-react";
import { bridgeApi } from "../api";
import type { AdapterDescriptor, ModelProfileDraft, ModelSetupState } from "../types";
import type { ManagedAgentStatus } from "../protocol/generated/protocol";
import type { UsageProvider } from "../usage";
import { isAbsent, sourceLine, stateLabel, useManagedAgents } from "./ManagedAgentsPanel";
import { HarnessMark } from "./harnessMarks";
import { ModelProfileEditor } from "./ModelProfileEditor";
import { ProviderLoginPane } from "./UsageWidget";

type Stage = "agents" | "models";

function needsRepair(agent: ManagedAgentStatus): boolean {
  return agent.state === "repairable" || agent.state === "broken";
}

function AgentChoice({ agent, adapter, busy, error, loginOpen, onInstall, onRepair, onLogin, onCloseLogin }: {
  agent: ManagedAgentStatus;
  adapter?: AdapterDescriptor;
  busy?: string;
  error?: string;
  loginOpen: boolean;
  onInstall: () => void;
  onRepair: () => void;
  onLogin: () => void;
  onCloseLogin: () => void;
}) {
  const authState = adapter?.authState ?? null;
  const absent = isAbsent(agent);
  const detected = agent.backing === "external" || agent.backing === "explicit";
  const settled = !absent && !needsRepair(agent) && adapter?.available === true && authState !== "signed_out";

  return <article className="u-glass-soft flex min-w-0 flex-col rounded-2xl border border-border-card p-4" data-testid={`onboarding-agent-${agent.agentId}`}>
    <div className="flex min-w-0 items-start gap-3">
      <span className="grid size-10 shrink-0 place-items-center rounded-xl border border-border-card bg-background text-foreground">
        <HarnessMark harness={agent.agentId} size={18} />
      </span>
      <div className="min-w-0 flex-1">
        <div className="flex min-w-0 items-center gap-2">
          <h3 className="truncate text-[14px] font-medium text-foreground">{agent.label}</h3>
          {detected && <span className="rounded-full bg-success/10 px-2 py-0.5 text-[10px] font-medium text-success">Detected</span>}
        </div>
        <p className="mt-1 truncate text-[11px] text-muted-foreground" title={agent.executable ?? sourceLine(agent)}>{sourceLine(agent)}</p>
      </div>
    </div>

    <div className="mt-4 flex min-h-8 items-center gap-2 border-t border-border-card pt-3">
      {busy ? <span role="status" className="inline-flex items-center gap-2 text-[12px] text-muted-foreground"><LoaderCircle className="animate-spin" size={13} />{busy}…</span>
      : absent ? <button type="button" onClick={onInstall} className="inline-flex min-h-8 items-center gap-2 rounded-lg bg-primary px-3 text-[12px] font-medium text-primary-foreground transition-colors hover:bg-primary/90"><Sparkles size={13} />Install</button>
      : needsRepair(agent) ? <button type="button" onClick={onRepair} className="inline-flex min-h-8 items-center gap-2 rounded-lg border border-border bg-card px-3 text-[12px] font-medium text-foreground transition-colors hover:bg-accent"><Wrench size={13} />Repair</button>
      : authState === "signed_out" ? <button type="button" onClick={onLogin} className="inline-flex min-h-8 items-center gap-2 rounded-lg bg-primary px-3 text-[12px] font-medium text-primary-foreground transition-colors hover:bg-primary/90">Sign in <ArrowRight size={13} /></button>
      : adapter?.available ? <span className="inline-flex items-center gap-1.5 text-[12px] font-medium text-success"><CheckCircle2 size={14} />{authState === "signed_in" ? "Signed in" : "Ready to try"}</span>
      : <span className="text-[12px] font-medium text-warning">Needs attention</span>}
      {!busy && !settled && !absent && authState !== "signed_out" && <span className="truncate text-[11px] text-muted-foreground" title={adapter?.unavailableReason ?? stateLabel(agent)}>{adapter?.unavailableReason ?? stateLabel(agent)}</span>}
    </div>
    {error && <p role="alert" className="mt-2 text-[11px] leading-relaxed text-destructive">{error}</p>}
    {loginOpen && <ProviderLoginPane provider={agent.agentId as UsageProvider} label={agent.label} onClose={onCloseLogin} />}
  </article>;
}

export function ModelSetupWizard({ adapters, onComplete, onSkip = () => undefined, onHealthChange = () => undefined, onError }: {
  adapters: AdapterDescriptor[];
  onComplete: (state: ModelSetupState) => void;
  onSkip?: () => void;
  onHealthChange?: () => void;
  onError: (message: string) => void;
}) {
  const profilesEdited = useRef(false);
  const [stage, setStage] = useState<Stage>("agents");
  const [profiles, setProfiles] = useState<ModelProfileDraft[]>([]);
  const [advanced, setAdvanced] = useState(false);
  const [busy, setBusy] = useState(false);
  const [login, setLogin] = useState<string | null>(null);
  const managed = useManagedAgents(undefined, onHealthChange);
  const readyAdapters = useMemo(
    () => adapters.filter(adapter => adapter.id !== "bridge" && adapter.available && adapter.authState !== "signed_out"),
    [adapters],
  );
  const readyAdapterKey = readyAdapters.map(adapter => adapter.id).join(":");
  const detectedCount = managed.agents?.filter(agent => !isAbsent(agent)).length ?? 0;

  useEffect(() => {
    if (stage !== "models" || !readyAdapterKey) return;
    let active = true;
    setBusy(true);
    bridgeApi.recommendedModelProfiles()
      .then(value => { if (active && !profilesEdited.current) setProfiles(value); })
      .catch(error => { if (active) onError(String(error)); })
      .finally(() => { if (active) setBusy(false); });
    return () => { active = false; };
  }, [stage, readyAdapterKey, onError]);

  const save = async () => {
    setBusy(true);
    try { onComplete(await bridgeApi.saveModelProfiles(profiles)); }
    catch (error) { onError(error instanceof Error ? error.message : String(error)); }
    finally { setBusy(false); }
  };

  const advance = () => {
    if (readyAdapters.length === 0) { onSkip(); return; }
    setStage("models");
  };

  const closeLogin = () => {
    setLogin(null);
    managed.reload();
    onHealthChange();
  };

  return <main className="relative z-50 flex min-h-[100dvh] w-full items-center justify-center overflow-y-auto bg-background px-4 py-8 text-foreground">
    <div className="pointer-events-none absolute inset-0 overflow-hidden" aria-hidden="true">
      <div className="absolute left-1/2 top-[-18rem] size-[40rem] -translate-x-1/2 rounded-full bg-primary/[0.055] blur-3xl" />
      <div className="absolute bottom-[-16rem] right-[-8rem] size-[32rem] rounded-full bg-muted/50 blur-3xl" />
    </div>
    <div className="relative w-full max-w-5xl">
      <header className="mb-6 flex items-center justify-between gap-4 px-1">
        <div className="flex items-center gap-2.5">
          <span className="grid size-8 place-items-center rounded-xl border border-border-card bg-card shadow-sm"><Bot size={16} /></span>
          <span className="font-display text-[15px] font-semibold tracking-tight">Bridge</span>
        </div>
        <ol className="flex items-center gap-1.5 text-[11px] text-muted-foreground" aria-label="Onboarding progress">
          <li className={`inline-flex items-center gap-1.5 rounded-full px-2.5 py-1 ${stage === "agents" ? "bg-foreground text-background" : "bg-success/10 text-success"}`}><span>{stage === "models" ? <Check size={11} /> : "1"}</span> Agents</li>
          <li aria-hidden="true">—</li>
          <li className={`rounded-full px-2.5 py-1 ${stage === "models" ? "bg-foreground text-background" : "bg-muted text-muted-foreground"}`}>2&nbsp; Models</li>
        </ol>
      </header>

      <section className="u-glass-popover overflow-hidden rounded-3xl border border-border-card shadow-2xl shadow-foreground/[0.04]">
        {stage === "agents" ? <>
          <div className="border-b border-border-card px-6 py-6 sm:px-8 sm:py-7">
            <div className="flex items-start gap-4">
              <span className="mt-0.5 grid size-11 shrink-0 place-items-center rounded-2xl bg-primary text-primary-foreground"><Search size={20} /></span>
              <div>
                <p className="text-[11px] font-medium uppercase tracking-[0.16em] text-muted-foreground">Welcome to Bridge</p>
                <h1 className="mt-1.5 font-display text-2xl font-semibold tracking-tight sm:text-[28px]">Bring your agents with you.</h1>
                <p className="mt-2 max-w-2xl text-[13px] leading-relaxed text-muted-foreground">Bridge checks your computer first, so nothing gets installed twice. Use any agents you already have, add the ones you want, and leave the rest alone.</p>
              </div>
            </div>
          </div>
          <div className="max-h-[58dvh] overflow-y-auto px-6 py-5 sm:px-8">
            <div className="mb-4 flex items-center justify-between gap-3">
              <div>
                <h2 className="text-[13px] font-medium">Choose your agents</h2>
                <p className="mt-0.5 text-[11px] text-muted-foreground">{managed.agents ? `${detectedCount} detected on this computer` : "Checking this computer…"}</p>
              </div>
              <button type="button" onClick={() => { managed.reload(); onHealthChange(); }} className="min-h-8 rounded-lg border border-border bg-card px-3 text-[11px] font-medium text-foreground transition-colors hover:bg-accent">Scan again</button>
            </div>
            {managed.listError && <div className="mb-4 flex items-center justify-between gap-3 rounded-xl border border-destructive/25 bg-destructive/5 px-3 py-2 text-[12px] text-destructive"><span>{managed.listError}</span><button type="button" onClick={managed.reload} className="font-medium underline underline-offset-2">Retry</button></div>}
            {!managed.agents ? <div role="status" className="grid min-h-48 place-items-center rounded-2xl border border-border-card"><span className="inline-flex items-center gap-2 text-[12px] text-muted-foreground"><LoaderCircle className="animate-spin" size={14} />Detecting installed agents…</span></div>
            : <div className="grid gap-3 md:grid-cols-2">
              {managed.agents.map(agent => <AgentChoice
                key={agent.agentId}
                agent={agent}
                adapter={adapters.find(adapter => adapter.id === agent.agentId)}
                busy={managed.busy[agent.agentId]}
                error={managed.errors[agent.agentId]}
                loginOpen={login === agent.agentId}
                onInstall={() => managed.install(agent)}
                onRepair={() => managed.repair(agent)}
                onLogin={() => setLogin(agent.agentId)}
                onCloseLogin={closeLogin}
              />)}
            </div>}
          </div>
          <footer className="flex flex-wrap items-center justify-between gap-3 border-t border-border-card bg-muted/20 px-6 py-4 sm:px-8">
            <button type="button" onClick={() => onSkip()} className="min-h-9 rounded-lg px-3 text-[12px] text-muted-foreground transition-colors hover:bg-accent hover:text-foreground">Skip for now</button>
            <button type="button" onClick={advance} className="inline-flex min-h-9 items-center gap-2 rounded-lg bg-primary px-4 text-[12px] font-medium text-primary-foreground transition-colors hover:bg-primary/90">{readyAdapters.length > 0 ? "Continue" : "Continue without an agent"}<ArrowRight size={14} /></button>
          </footer>
        </> : <>
          <div className="border-b border-border-card px-6 py-6 sm:px-8 sm:py-7">
            <p className="text-[11px] font-medium uppercase tracking-[0.16em] text-muted-foreground">Almost there</p>
            <h1 className="mt-1.5 font-display text-2xl font-semibold tracking-tight sm:text-[28px]">Choose how Bridge uses your models.</h1>
            <p className="mt-2 max-w-2xl text-[13px] leading-relaxed text-muted-foreground">Start with thoughtful defaults, or tune every role now. Nothing here changes your provider account.</p>
          </div>
          <div className="max-h-[58dvh] min-h-64 overflow-y-auto px-6 py-5 sm:px-8">
            {busy && profiles.length === 0 ? <div role="status" className="grid min-h-52 place-items-center"><span className="inline-flex items-center gap-2 text-[12px] text-muted-foreground"><LoaderCircle className="animate-spin" size={14} />Reading available models…</span></div>
            : advanced ? <>
              <div className="mb-5 flex flex-wrap items-center justify-between gap-3"><div><h2 className="text-[14px] font-medium">Advanced role profiles</h2><p className="mt-1 text-[12px] text-muted-foreground">Only models advertised by ready agents can be selected.</p></div><button type="button" className="min-h-8 rounded-lg border border-border bg-card px-3 text-[12px] text-foreground transition-colors hover:bg-accent" onClick={() => setAdvanced(false)}>Use simple setup</button></div>
              <ModelProfileEditor profiles={profiles} adapters={adapters} disabled={busy} onChange={value => { profilesEdited.current = true; setProfiles(value); }} />
            </> : <div className="grid gap-4 md:grid-cols-[1.3fr_0.7fr]">
              <div className="rounded-2xl border border-border-card bg-card p-5">
                <span className="grid size-9 place-items-center rounded-xl bg-success/10 text-success"><CheckCircle2 size={17} /></span>
                <h2 className="mt-4 text-[15px] font-medium">Recommended defaults</h2>
                <p className="mt-2 text-[12px] leading-relaxed text-muted-foreground">Bridge maps fast, balanced, and high-capability work to the best available models from the agents you connected.</p>
                <div className="mt-4 flex flex-wrap gap-2">{readyAdapters.map(adapter => <span key={adapter.id} className="inline-flex items-center gap-1.5 rounded-full border border-border-card bg-background px-2.5 py-1 text-[11px]"><HarnessMark harness={adapter.id} size={11} />{adapter.label}</span>)}</div>
              </div>
              <button type="button" onClick={() => setAdvanced(true)} className="group rounded-2xl border border-border-card bg-card p-5 text-left transition-colors hover:bg-accent/50">
                <Settings2 className="text-muted-foreground transition-colors group-hover:text-foreground" size={18} />
                <span className="mt-4 block text-[14px] font-medium">Customize role profiles</span>
                <span className="mt-2 block text-[12px] leading-relaxed text-muted-foreground">Pick the exact model and effort for planning, coding, review, and research.</span>
              </button>
            </div>}
          </div>
          <footer className="flex flex-wrap items-center justify-between gap-3 border-t border-border-card bg-muted/20 px-6 py-4 sm:px-8">
            <button type="button" onClick={() => setStage("agents")} className="inline-flex min-h-9 items-center gap-2 rounded-lg px-3 text-[12px] text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"><ArrowLeft size={14} />Back</button>
            <button type="button" disabled={busy || profiles.length === 0} onClick={() => void save()} className="inline-flex min-h-9 items-center gap-2 rounded-lg bg-primary px-4 text-[12px] font-medium text-primary-foreground transition-colors hover:bg-primary/90 disabled:opacity-40">{busy && <LoaderCircle className="animate-spin" size={14} />}{advanced ? "Save model setup" : "Use recommended defaults"}</button>
          </footer>
        </>}
      </section>
      <p className="mt-4 text-center text-[11px] text-muted-foreground">You can change agents, authentication, and model roles later in Settings.</p>
    </div>
  </main>;
}
