import { ManagedAgentsPanel } from "./ManagedAgentsPanel";
import { useEffect, useRef, useState } from "react";
import { Bot, LoaderCircle, Settings2, ShieldCheck } from "lucide-react";
import { bridgeApi } from "../api";
import type { AdapterDescriptor, ModelProfileDraft, ModelSetupState } from "../types";
import { ModelProfileEditor } from "./ModelProfileEditor";

export function ModelSetupWizard({ adapters, onComplete, onError }: {
  adapters: AdapterDescriptor[];
  onComplete: (state: ModelSetupState) => void;
  onError: (message: string) => void;
}) {
  const profilesEdited = useRef(false);
  const [profiles, setProfiles] = useState<ModelProfileDraft[]>([]);
  const [advanced, setAdvanced] = useState(false);
  const [busy, setBusy] = useState(true);

  useEffect(() => {
    let active = true;
    bridgeApi.recommendedModelProfiles().then(value => { if (active && !profilesEdited.current) setProfiles(value); }).catch(error => { if (active) onError(String(error)); }).finally(() => { if (active) setBusy(false); });
    return () => { active = false; };
  }, [onError, adapters]);

  const save = async () => {
    setBusy(true);
    try { onComplete(await bridgeApi.saveModelProfiles(profiles)); }
    catch (error) { onError(error instanceof Error ? error.message : String(error)); }
    finally { setBusy(false); }
  };

  return <main className="relative z-50 flex min-h-[100dvh] w-full items-center justify-center overflow-y-auto px-4 py-8 text-foreground">
    <div className={`u-glass-popover flex max-h-[88dvh] w-full flex-col overflow-hidden rounded-2xl ${advanced ? "max-w-5xl" : "max-w-xl"}`}>
      <header className="shrink-0 border-b border-border bg-muted/20 px-6 py-5">
        <span className="mb-3 flex h-10 w-10 items-center justify-center rounded-xl bg-muted text-foreground"><Bot size={23} aria-hidden="true" /></span>
        <h1 className="font-display text-2xl font-semibold text-foreground">Set up Bridge models</h1>
        <p className="mt-2 max-w-2xl text-sm leading-relaxed text-muted-foreground">Choose the models Bridge uses for conversations and delegated work. You can change these later in Settings → Models.</p>
      </header>
      <div className="min-h-0 flex-1 overflow-y-auto p-6">
        <details className="mb-4 rounded-xl border border-border p-3">
          <summary className="cursor-pointer text-sm font-medium">Install or sign in to your agents</summary>
          <p className="mt-2 text-[13px] text-muted-foreground">Choose an agent, then sign in. Bridge starts the provider’s login for you.</p>
          <ManagedAgentsPanel />
        </details>
        {!advanced ? <>
          <div className="rounded-xl border border-border bg-card p-4">
            <div className="flex gap-3"><ShieldCheck className="mt-0.5 shrink-0 text-muted-foreground" size={18} aria-hidden="true" /><div><h2 className="text-sm font-medium text-foreground">Recommended defaults</h2><p className="mt-1 text-[13px] leading-relaxed text-muted-foreground">Bridge chooses fast, balanced, and high-capability defaults from each adapter’s live catalog. No provider knowledge required.</p></div></div>
          </div>
        </> : <>
          <div className="mb-5 flex flex-wrap items-center justify-between gap-3"><div className="min-w-0"><h2 className="font-display text-lg font-semibold text-foreground">Advanced role profiles</h2><p className="mt-1 text-[13px] text-muted-foreground">Only models advertised by available adapters can be selected.</p></div><button type="button" className="min-h-8 rounded-lg border border-border bg-card px-3 text-[13px] text-foreground transition-colors hover:bg-accent" onClick={() => setAdvanced(false)}>Back to defaults</button></div>
          <ModelProfileEditor profiles={profiles} adapters={adapters} disabled={busy} onChange={value => { profilesEdited.current = true; setProfiles(value); }} />

        </>}
      </div>
      <footer className="flex shrink-0 flex-wrap items-center justify-end gap-2 border-t border-border bg-muted/20 px-6 py-4">
        {!advanced && <button type="button" disabled={busy || profiles.length === 0} onClick={() => setAdvanced(true)} className="inline-flex min-h-8 items-center justify-center gap-2 rounded-lg border border-border bg-card px-3 text-[13px] text-foreground transition-colors hover:bg-accent disabled:opacity-40"><Settings2 size={14} aria-hidden="true" />Customize role profiles</button>}
        <button type="button" disabled={busy || profiles.length === 0} onClick={() => void save()} className="inline-flex min-h-8 items-center justify-center gap-2 rounded-lg bg-primary px-4 text-[13px] font-medium text-primary-foreground transition-colors hover:bg-primary/90 disabled:opacity-40">{busy && <LoaderCircle className="animate-spin" size={14} aria-hidden="true" />}{advanced ? "Save model setup" : "Use recommended defaults"}</button>
      </footer>
    </div>
  </main>;
}
