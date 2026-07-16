import { useCallback, useEffect, useMemo, useState } from "react";
import { AlertTriangle, Check, ChevronDown, Download, ExternalLink, LoaderCircle, RotateCcw, Search, ShieldCheck, Trash2, X } from "lucide-react";
import { bridgeApi } from "../api";
import type { CommunitySkill, SkillAction, SkillActionResult, SkillCatalog, SkillPreview, SkillProvider } from "../types";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogPanel, DialogTitle } from "@/components/ui/dialog";

const providerLabel = (provider: SkillProvider) => provider === "codex" ? "Codex" : "Claude";
const compactNumber = new Intl.NumberFormat("en", { notation: "compact", maximumFractionDigits: 1 });

export function SkillMarketplace() {
  const [catalog, setCatalog] = useState<SkillCatalog>();
  const [scope, setScope] = useState<"community" | "personal">("community");
  const [provider, setProvider] = useState<SkillProvider | "all">("all");
  const [query, setQuery] = useState("");
  const [expanded, setExpanded] = useState<string>();
  const [preview, setPreview] = useState<SkillPreview>();
  const [results, setResults] = useState<SkillActionResult[]>([]);
  const [failure, setFailure] = useState<string>();
  const [busy, setBusy] = useState(false);
  const refresh = useCallback(async () => { try { setCatalog(await bridgeApi.skillCatalog()); setFailure(undefined); } catch (error) { setFailure(error instanceof Error ? error.message : String(error)); } }, []);
  useEffect(() => { void refresh(); }, [refresh]);
  const community = useMemo(() => {
    const needle = query.trim().toLowerCase();
    return (catalog?.community ?? []).filter(skill => (provider === "all" || skill.compatibility.includes(provider)) && (!needle || `${skill.name} ${skill.description} ${skill.source} ${skill.categories.join(" ")}`.toLowerCase().includes(needle)));
  }, [catalog, provider, query]);
  const personal = useMemo(() => {
    const needle = query.trim().toLowerCase();
    return (catalog?.personal ?? []).filter(skill => (provider === "all" || skill.providers.includes(provider)) && (!needle || `${skill.name} ${skill.description}`.toLowerCase().includes(needle)));
  }, [catalog, provider, query]);
  const request = async (skill: CommunitySkill, action: SkillAction, targets: SkillProvider[]) => {
    setBusy(true); setResults([]); setFailure(undefined);
    try { setPreview(await bridgeApi.previewSkillChange(skill.id, action, targets)); }
    catch (error) { setFailure(error instanceof Error ? error.message : String(error)); }
    finally { setBusy(false); }
  };
  const execute = async () => {
    if (!preview) return;
    setBusy(true);
    try { setResults(await bridgeApi.executeSkillChange(preview.confirmationId)); setPreview(undefined); await refresh(); }
    catch (error) { setFailure(error instanceof Error ? error.message : String(error)); setPreview(undefined); }
    finally { setBusy(false); }
  };
  return <div className="h-full min-h-0 overflow-y-auto scrollbar-thin scrollbar-thumb-white/10">
    <main className="mx-auto w-full max-w-5xl px-5 pb-12 pt-7 sm:px-8 sm:pt-10">
      <div className="flex items-start justify-between gap-4"><div><h1 className="font-display text-[26px] font-semibold tracking-tight text-white">Skills</h1><p className="mt-1 text-[13px] text-neutral-500">Curated capabilities for Codex and Claude, with explicit trust and rollback</p></div><span className="rounded-full border border-white/[0.08] px-2.5 py-1 text-[9.5px] text-neutral-500">{catalog?.community.length ?? 0} curated</span></div>
      <div className="relative mt-5"><Search className="pointer-events-none absolute left-3 top-1/2 -translate-y-1/2 text-neutral-500" size={14}/><Input value={query} onChange={event => setQuery(event.target.value)} placeholder="Search skills, publishers, or capabilities" className="h-8 rounded-xl border-white/[0.12] bg-white/[0.055] pl-9 text-[11.5px]"/></div>
      <div className="mt-4 flex items-center justify-between border-b border-white/[0.06] pb-2"><div className="flex gap-1">{(["community", "personal"] as const).map(value => <button key={value} onClick={() => setScope(value)} className={`rounded-lg px-2.5 py-1 text-[10.5px] capitalize ${scope === value ? "bg-white/[0.07] text-neutral-100" : "text-neutral-500 hover:text-neutral-300"}`}>{value}</button>)}</div><div className="flex gap-1">{(["all", "codex", "claude"] as const).map(value => <button key={value} onClick={() => setProvider(value)} className={`rounded-md px-1.5 py-1 text-[9.5px] ${provider === value ? "text-neutral-100" : "text-neutral-500 hover:text-neutral-300"}`}>{value === "all" ? "All" : providerLabel(value)}</button>)}</div></div>
      {failure && <div className="mt-3 flex items-start gap-2 rounded-xl border border-red-400/15 bg-red-400/[0.04] px-3 py-2 text-[10.5px] text-red-200/80"><AlertTriangle className="mt-0.5 shrink-0" size={12}/><span>{failure}</span></div>}
      {!catalog && <div className="flex min-h-56 items-center justify-center gap-2 text-xs text-neutral-500"><LoaderCircle className="animate-spin" size={15}/>Reading local capabilities…</div>}
      {catalog && scope === "community" && <div className="mt-3 grid grid-cols-1 gap-x-7 lg:grid-cols-2">{community.map(skill => {
        const open = expanded === skill.id;
        const installed = skill.providerStates.filter(state => state.installed);
        const installable = skill.providerStates.filter(state => !state.installed).map(state => state.provider);
        return <article key={skill.id} className="border-b border-white/[0.055] py-3">
          <button type="button" onClick={() => setExpanded(open ? undefined : skill.id)} className="flex w-full items-start gap-3 text-left"><div className="mt-0.5 flex h-8 w-8 shrink-0 items-center justify-center rounded-lg border border-white/[0.08] bg-white/[0.04] font-display text-xs text-neutral-300">{skill.name.slice(0, 2).toUpperCase()}</div><div className="min-w-0 flex-1"><div className="flex items-center gap-2"><h3 className="truncate text-[12px] font-medium text-neutral-100">{skill.name}</h3>{skill.official && <ShieldCheck size={11} className="text-sky-400" aria-label="Official publisher"/>}<span className={`rounded px-1 py-0.5 text-[8px] uppercase ${skill.risk === "low" ? "bg-emerald-400/10 text-emerald-300" : "bg-amber-400/10 text-amber-300"}`}>{skill.risk} risk</span></div><p className="mt-1 line-clamp-2 text-[10.5px] leading-4 text-neutral-500">{skill.description}</p><div className="mt-1.5 flex gap-2 text-[9px] text-neutral-600"><span>{compactNumber.format(skill.installs)} installs</span><span>·</span><span>{skill.source}</span>{installed.length > 0 && <span className="text-emerald-400/80">Installed: {installed.map(state => providerLabel(state.provider)).join(" + ")}</span>}</div></div><ChevronDown size={13} className={`mt-1 shrink-0 text-neutral-600 transition-transform ${open ? "rotate-180" : ""}`}/></button>
          {open && <div className="ml-11 mt-3 rounded-xl border border-white/[0.07] bg-white/[0.025] p-3 text-[10px] text-neutral-500"><div className="flex flex-wrap gap-1.5">{skill.permissions.map(permission => <span key={permission} className="rounded-md border border-white/[0.07] px-1.5 py-0.5">{permission}</span>)}</div><p className="mt-2 leading-4">{skill.riskSummary}</p><div className="mt-2 flex flex-wrap items-center gap-x-3 gap-y-1 font-mono text-[9px]"><a href={skill.sourceUrl} target="_blank" rel="noreferrer" className="inline-flex items-center gap-1 text-sky-400 hover:text-sky-300">Source <ExternalLink size={9}/></a><span>pin {skill.pinnedRef.slice(0, 10)}</span><span>{skill.fileCount} files</span><span>{catalog.installer}</span></div><div className="mt-3 flex flex-wrap gap-1.5">{installable.map(target => <Button key={target} size="xs" disabled={busy} onClick={() => void request(skill, "install", [target])}><Download size={10}/>Install for {providerLabel(target)}</Button>)}{installable.length === 2 && <Button size="xs" variant="secondary" disabled={busy} onClick={() => void request(skill, "install", installable)}>Install for both</Button>}{installed.filter(state => state.managed && state.updateAvailable).map(state => <Button key={`update-${state.provider}`} size="xs" disabled={busy} onClick={() => void request(skill, "install", [state.provider])}><Download size={10}/>Update {providerLabel(state.provider)}</Button>)}{installed.filter(state => state.managed && state.rollbackAvailable).map(state => <Button key={`rollback-${state.provider}`} size="xs" variant="secondary" disabled={busy} onClick={() => void request(skill, "rollback", [state.provider])}><RotateCcw size={10}/>Rollback {providerLabel(state.provider)}</Button>)}{installed.filter(state => state.managed).map(state => <Button key={`remove-${state.provider}`} size="xs" variant="ghost" disabled={busy} onClick={() => void request(skill, "uninstall", [state.provider])}><Trash2 size={10}/>Remove {providerLabel(state.provider)}</Button>)}</div></div>}
        </article>;
      })}</div>}
      {catalog && scope === "personal" && <section className="mt-5"><div className="mb-4 rounded-xl border border-sky-400/10 bg-sky-400/[0.035] px-3 py-2 text-[10.5px] leading-4 text-sky-100/70"><b className="text-sky-200">Yours, untouched.</b> Personal Skills are discovered locally and shown separately. Bridge never updates, replaces, or removes them.</div><div className="grid grid-cols-1 gap-x-7 lg:grid-cols-2">{personal.map(skill => <article key={skill.id} className="border-b border-white/[0.055] py-3"><div className="flex items-start justify-between gap-3"><div><h3 className="text-[12px] font-medium text-neutral-100">{skill.name}</h3><p className="mt-1 text-[10.5px] leading-4 text-neutral-500">{skill.description || "Local skill"}</p></div><div className="flex gap-1">{skill.providers.map(item => <span key={item} className="rounded border border-white/[0.07] px-1.5 py-0.5 text-[8.5px] text-neutral-500">{providerLabel(item)}</span>)}</div></div></article>)}</div>{personal.length === 0 && <p className="py-16 text-center text-xs text-neutral-600">No matching Personal Skills.</p>}</section>}
      {!!results.length && <div className="fixed bottom-5 right-5 z-40 w-80 rounded-xl border border-white/[0.1] bg-[#111116]/95 p-3 shadow-2xl backdrop-blur-xl"><button onClick={() => setResults([])} className="absolute right-2 top-2 text-neutral-600 hover:text-neutral-300"><X size={12}/></button>{results.map(result => <p key={`${result.provider}:${result.action}`} className="flex gap-2 py-1 text-[10.5px] text-neutral-400">{result.success ? <Check size={12} className="text-emerald-400"/> : <AlertTriangle size={12} className="text-red-400"/>}<span><b className="text-neutral-200">{providerLabel(result.provider)}:</b> {result.error ?? result.message}</span></p>)}</div>}
    </main>
    <Dialog open={!!preview} onOpenChange={open => { if (!open && !busy) setPreview(undefined); }}>{preview && <DialogContent className="border-white/[0.1] bg-[#111116]" showCloseButton={!busy}><DialogHeader><div className="flex items-start gap-3"><AlertTriangle className="mt-0.5 shrink-0 text-amber-300" size={16}/><div><DialogTitle className="font-display text-base font-medium text-white">Confirm {preview.action}</DialogTitle><DialogDescription className="mt-1 text-[11px] leading-5">Review the exact source and requested access before Bridge changes either provider.</DialogDescription></div></div></DialogHeader><DialogPanel className="py-0"><dl className="grid grid-cols-[88px_1fr] gap-x-3 gap-y-2 rounded-xl border border-white/[0.07] bg-white/[0.025] p-3 text-[10px]"><dt className="text-neutral-600">Skill</dt><dd className="text-neutral-200">{preview.skill.name}</dd><dt className="text-neutral-600">Source</dt><dd className="break-all text-neutral-400">{preview.skill.sourceUrl}</dd><dt className="text-neutral-600">Pinned ref</dt><dd className="break-all font-mono text-neutral-400">{preview.skill.pinnedRef}</dd><dt className="text-neutral-600">Targets</dt><dd className="text-neutral-400">{preview.targets.map(providerLabel).join(" + ")}</dd><dt className="text-neutral-600">Permissions</dt><dd className="text-neutral-400">{preview.skill.permissions.join(", ")}</dd><dt className="text-neutral-600">Risk</dt><dd className="text-neutral-400">{preview.skill.risk}: {preview.skill.riskSummary}</dd><dt className="text-neutral-600">Files</dt><dd className="text-neutral-400">{preview.skill.fileCount} via {preview.installer}</dd></dl><ul className="mt-3 space-y-1 text-[10px] text-neutral-500">{preview.changes.map(change => <li key={change}>• {change}</li>)}</ul></DialogPanel><DialogFooter variant="bare"><Button variant="ghost" disabled={busy} onClick={() => setPreview(undefined)}>Cancel</Button><Button disabled={busy} onClick={() => void execute()}>{busy && <LoaderCircle className="animate-spin" size={12}/>}Confirm once</Button></DialogFooter></DialogContent>}</Dialog>
  </div>;
}
