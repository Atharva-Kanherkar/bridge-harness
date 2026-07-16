import { useEffect, useState } from "react";
import { Bot, ChevronRight, LoaderCircle, Settings2, ShieldCheck } from "lucide-react";
import { bridgeApi } from "../api";
import type { AdapterDescriptor, ModelProfileDraft, ModelSetupState } from "../types";
import { ModelProfileEditor } from "./ModelProfileEditor";

export function ModelSetupWizard({ adapters, onComplete, onError }: {
  adapters: AdapterDescriptor[];
  onComplete: (state: ModelSetupState) => void;
  onError: (message: string) => void;
}) {
  const [profiles, setProfiles] = useState<ModelProfileDraft[]>([]);
  const [advanced, setAdvanced] = useState(false);
  const [busy, setBusy] = useState(true);

  useEffect(() => {
    let active = true;
    bridgeApi.recommendedModelProfiles().then(value => { if (active) setProfiles(value); }).catch(error => { if (active) onError(String(error)); }).finally(() => { if (active) setBusy(false); });
    return () => { active = false; };
  }, [onError]);

  const save = async () => {
    setBusy(true);
    try { onComplete(await bridgeApi.saveModelProfiles(profiles)); }
    catch (error) { onError(error instanceof Error ? error.message : String(error)); }
    finally { setBusy(false); }
  };

  return <main className="relative z-50 flex h-[100dvh] w-full items-center justify-center overflow-hidden px-4 py-8 text-neutral-200">
    <div className={`w-full overflow-hidden rounded-3xl border border-white/[0.09] bg-[#121214]/95 shadow-2xl shadow-black/50 backdrop-blur-2xl ${advanced ? "max-w-5xl" : "max-w-xl"}`}>
      <header className="border-b border-white/[0.07] px-6 py-6 text-center">
        <span className="mx-auto mb-4 flex h-12 w-12 items-center justify-center rounded-2xl bg-violet-400/[0.1] text-violet-300"><Bot size={23} aria-hidden="true" /></span>
        <h1 className="font-display text-2xl font-semibold text-white">Set up Bridge models</h1>
        <p className="mx-auto mt-2 max-w-lg text-sm leading-relaxed text-neutral-500">Start with recommended role profiles from your installed adapters. You can tune them now or any time in learning settings.</p>
      </header>
      <div className={`overflow-y-auto p-6 ${advanced ? "max-h-[65vh]" : ""}`}>
        {!advanced ? <>
          <div className="rounded-2xl border border-emerald-300/[0.1] bg-emerald-300/[0.04] p-4">
            <div className="flex gap-3"><ShieldCheck className="mt-0.5 shrink-0 text-emerald-300" size={18} aria-hidden="true" /><div><h2 className="text-sm font-medium text-neutral-200">Recommended defaults</h2><p className="mt-1 text-[12px] leading-relaxed text-neutral-500">Bridge chooses fast, balanced, and high-capability defaults from each adapter’s live catalog. No provider knowledge required.</p></div></div>
          </div>
          <button type="button" disabled={busy || profiles.length === 0} onClick={() => void save()} className="mt-5 inline-flex h-11 w-full items-center justify-center gap-2 rounded-xl bg-white text-sm font-medium text-neutral-900 disabled:opacity-40">{busy ? <LoaderCircle className="animate-spin" size={15} aria-hidden="true" /> : null}Use recommended defaults<ChevronRight size={16} aria-hidden="true" /></button>
          <button type="button" disabled={busy || profiles.length === 0} onClick={() => setAdvanced(true)} className="mt-2 inline-flex h-10 w-full items-center justify-center gap-2 rounded-xl text-sm text-neutral-500 hover:bg-white/[0.05] hover:text-neutral-300 disabled:opacity-40"><Settings2 size={14} aria-hidden="true" />Customize role profiles</button>
        </> : <>
          <div className="mb-5 flex items-center justify-between gap-3"><div><h2 className="font-display text-lg font-semibold text-white">Advanced role profiles</h2><p className="mt-1 text-xs text-neutral-500">Only models advertised by available adapters can be selected.</p></div><button type="button" className="text-xs text-neutral-500 hover:text-neutral-300" onClick={() => setAdvanced(false)}>Back to defaults</button></div>
          <ModelProfileEditor profiles={profiles} adapters={adapters} disabled={busy} onChange={setProfiles} />
          <div className="sticky bottom-0 mt-5 flex justify-end border-t border-white/[0.07] bg-[#121214]/95 pt-4"><button type="button" disabled={busy || profiles.length === 0} onClick={() => void save()} className="inline-flex h-10 min-w-36 items-center justify-center gap-2 rounded-xl bg-white px-4 text-sm font-medium text-neutral-900 disabled:opacity-40">{busy && <LoaderCircle className="animate-spin" size={14} aria-hidden="true" />}Save model setup</button></div>
        </>}
      </div>
    </div>
  </main>;
}
