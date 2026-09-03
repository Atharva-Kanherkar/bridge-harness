import { useMemo, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { AlertTriangle, CheckCircle2, FileJson2, FolderOpen, History, LoaderCircle, LockKeyhole, ShieldCheck } from "lucide-react";
import { bridgeApi } from "../api";
import type {
  ExternalImportArtifact,
  ExternalImportCandidate,
  ExternalImportCommit,
  ExternalImportConflictPolicy,
  ExternalImportDiscovery,
} from "../protocol/generated/protocol";
import { cn } from "@/lib/utils";

const CLAUDE_FORMAT_VERSIONS = {
  "claude-auto-memory-v1": "claude-auto-memory-v1",
  "claude-jsonl-v1": "claude-jsonl-v1",
  "claude-settings-v1": "claude-settings-v1",
};

const SETUP_KINDS = new Set(["instruction", "rule", "prompt", "command", "skill", "agent", "hook", "mcp_server", "plugin"]);
const control = "h-9 rounded-xl border border-border bg-background/60 px-3 text-xs text-foreground outline-none focus:border-foreground/25 disabled:opacity-45";
type Stage = "source" | "discovery" | "preview" | "confirm" | "result";

function kindLabel(kind: ExternalImportArtifact["kind"]) {
  return kind.replaceAll("_", " ");
}

function formatBytes(bytes: number) {
  if (bytes < 1_024) return `${bytes} B`;
  return `${(bytes / 1_024).toFixed(bytes < 10_240 ? 1 : 0)} KB`;
}

function countStatuses(commit: ExternalImportCommit) {
  return commit.candidateResults.reduce<Record<string, number>>((counts, result) => {
    counts[result.status] = (counts[result.status] ?? 0) + 1;
    return counts;
  }, {});
}

export function ImportHarnessSection({ onError }: { onError: (message: string) => void }) {
  const [stage, setStage] = useState<Stage>("source");
  const [busy, setBusy] = useState(false);
  const [discovery, setDiscovery] = useState<ExternalImportDiscovery>();
  const [artifactIds, setArtifactIds] = useState<string[]>([]);
  const [candidates, setCandidates] = useState<ExternalImportCandidate[]>([]);
  const [candidateIds, setCandidateIds] = useState<string[]>([]);
  const [conflictPolicy, setConflictPolicy] = useState<ExternalImportConflictPolicy>("skip");
  const [memoryScope, setMemoryScope] = useState("");
  const [dryRun, setDryRun] = useState(false);
  const [commit, setCommit] = useState<ExternalImportCommit>();

  const selectedCandidates = useMemo(
    () => candidates.filter(candidate => candidateIds.includes(candidate.candidateId)),
    [candidateIds, candidates],
  );
  const memorySelected = selectedCandidates.some(candidate => candidate.kind === "memory");
  const canConfirm = selectedCandidates.length > 0 && (!memorySelected || memoryScope.length > 0);

  const report = (error: unknown) => onError(error instanceof Error ? error.message : String(error));

  async function discover(kind: "directory" | "export") {
    try {
      const chosen = await open(kind === "directory"
        ? { directory: true, multiple: false, title: "Choose a Claude Code project or configuration folder" }
        : { directory: false, multiple: false, title: "Choose a Claude Code JSONL export", filters: [{ name: "Claude Code history", extensions: ["jsonl"] }] });
      const path = Array.isArray(chosen) ? chosen[0] : chosen;
      if (!path) return;
      setBusy(true);
      const found = await bridgeApi.discoverExternalImport({
        provider: "claude_code",
        approvedRoots: kind === "directory" ? [path] : [],
        selectedExport: kind === "export" ? path : null,
        sourceVersion: null,
        schemaVersion: null,
        formatVersions: CLAUDE_FORMAT_VERSIONS,
      });
      setDiscovery(found);
      setArtifactIds([]);
      setCandidates([]);
      setCandidateIds([]);
      setCommit(undefined);
      setStage("discovery");
    } catch (error) { report(error); }
    finally { setBusy(false); }
  }

  async function preview() {
    if (!discovery || artifactIds.length === 0) return;
    setBusy(true);
    try {
      const result = await bridgeApi.previewExternalImport(discovery, artifactIds);
      setCandidates(result.candidates);
      setCandidateIds(result.candidates.filter(candidate => candidate.selectedByDefault).map(candidate => candidate.candidateId));
      setStage("preview");
    } catch (error) { report(error); }
    finally { setBusy(false); }
  }

  async function commitSelection() {
    if (!canConfirm) return;
    setBusy(true);
    try {
      const result = await bridgeApi.commitExternalImport(candidates, {
        selectedCandidateIds: candidateIds,
        conflictPolicy,
        setupActivationPolicy: "disabled",
        memoryScope: memorySelected ? memoryScope : null,
        dryRun,
        createdAt: new Date().toISOString(),
      });
      setCommit(result);
      setStage("result");
    } catch (error) { report(error); }
    finally { setBusy(false); }
  }

  const toggle = (id: string, selected: boolean, setIds: (ids: string[]) => void, ids: string[]) =>
    setIds(selected ? [...ids, id] : ids.filter(item => item !== id));

  return <div className="mx-auto max-w-4xl">
    <div className="mb-5">
      <div className="flex items-center gap-2"><History size={17} className="text-muted-foreground"/><h2 className="font-display text-lg font-semibold">Import from another harness</h2></div>
      <p className="mt-1 max-w-2xl text-xs leading-relaxed text-muted-foreground">Bring selected Claude Code history, memory, and reusable setup into Bridge. Discovery is read-only and nothing is stored until the final confirmation.</p>
    </div>

    <div className="mb-5 grid gap-2 sm:grid-cols-3">
      <div className="u-glass-soft rounded-2xl border border-border p-3"><ShieldCheck size={14} className="text-success"/><b className="mt-2 block text-[11px] font-medium">Local only</b><p className="mt-1 text-[10.5px] leading-relaxed text-muted-foreground">Source content stays on this Mac. There is no background scan, sync, or cloud upload.</p></div>
      <div className="u-glass-soft rounded-2xl border border-border p-3"><LockKeyhole size={14} className="text-muted-foreground"/><b className="mt-2 block text-[11px] font-medium">Secrets excluded</b><p className="mt-1 text-[10.5px] leading-relaxed text-muted-foreground">Credentials and authorization material are removed before preview or persistence.</p></div>
      <div className="u-glass-soft rounded-2xl border border-border p-3"><AlertTriangle size={14} className="text-warning"/><b className="mt-2 block text-[11px] font-medium">Not an encrypted backup</b><p className="mt-1 text-[10.5px] leading-relaxed text-muted-foreground">Bridge’s local database follows the app’s existing local-state posture and is not an encrypted backup.</p></div>
    </div>

    {stage === "source" && <section className="rounded-3xl border border-border bg-card/45 p-5">
      <h3 className="font-display text-sm font-semibold">Choose an explicit source</h3>
      <p className="mt-1 text-[11px] text-muted-foreground">Bridge reads only the folder or export you choose and never changes the source files or starts Claude Code.</p>
      <div className="mt-4 grid gap-3 sm:grid-cols-2">
        <button type="button" disabled={busy} onClick={() => void discover("directory")} className="flex items-start gap-3 rounded-2xl border border-border p-4 text-left transition-colors hover:bg-accent disabled:opacity-45"><FolderOpen size={17} className="mt-0.5 text-muted-foreground"/><span><b className="block text-[12px] font-medium">Choose Claude folder</b><span className="mt-1 block text-[10.5px] leading-relaxed text-muted-foreground">A project, <span className="font-mono">.claude</span> folder, or Claude home you approve.</span></span></button>
        <button type="button" disabled={busy} onClick={() => void discover("export")} className="flex items-start gap-3 rounded-2xl border border-border p-4 text-left transition-colors hover:bg-accent disabled:opacity-45"><FileJson2 size={17} className="mt-0.5 text-muted-foreground"/><span><b className="block text-[12px] font-medium">Choose JSONL export</b><span className="mt-1 block text-[10.5px] leading-relaxed text-muted-foreground">A version-gated Claude Code transcript selected directly.</span></span></button>
      </div>
    </section>}

    {stage === "discovery" && discovery && <section className="rounded-3xl border border-border bg-card/45 p-5">
      <div className="flex items-start justify-between gap-3"><div><h3 className="font-display text-sm font-semibold">Discovery metadata</h3><p className="mt-1 text-[11px] text-muted-foreground">{discovery.artifacts.length} items found. Content has not been persisted.</p></div><button type="button" onClick={() => setStage("source")} className="text-[11px] text-muted-foreground hover:text-foreground">Change source</button></div>
      <div className="mt-4 space-y-2">{discovery.artifacts.map(artifact => <label key={artifact.artifactId} className="flex items-start gap-3 rounded-2xl border border-border p-3.5">
        <input type="checkbox" className="mt-0.5" checked={artifactIds.includes(artifact.artifactId)} onChange={event => toggle(artifact.artifactId, event.target.checked, setArtifactIds, artifactIds)}/>
        <span className="min-w-0 flex-1"><span className="flex flex-wrap items-center gap-1.5"><b className="truncate text-[12px] font-medium">{artifact.sourceLabel}</b><span className="rounded-full bg-foreground/[0.06] px-1.5 py-0.5 text-[8px] uppercase tracking-wide text-muted-foreground">{kindLabel(artifact.kind)}</span><span className={cn("rounded-full px-1.5 py-0.5 text-[8px] uppercase tracking-wide", artifact.stability === "stable" ? "bg-success/10 text-success" : "bg-warning/10 text-warning")}>{artifact.stability.replaceAll("_", " ")}</span></span><span className="mt-1 block text-[10px] text-muted-foreground">{formatBytes(artifact.estimatedBytes)}{artifact.requiredSchemaGate ? ` · gated by ${artifact.requiredSchemaGate}` : " · documented source"}</span></span>
      </label>)}</div>
      {discovery.diagnostics.map(diagnostic => <p key={`${diagnostic.code}:${diagnostic.sourceLabel}`} className="mt-2 rounded-xl border border-warning/25 bg-warning/5 px-3 py-2 text-[10.5px] text-warning">{diagnostic.message}</p>)}
      <div className="mt-4 flex items-center justify-between"><p className="text-[10px] text-muted-foreground">Nothing is selected by default.</p><button type="button" disabled={busy || artifactIds.length === 0} onClick={() => void preview()} className="inline-flex h-9 items-center gap-2 rounded-xl bg-foreground px-3.5 text-xs font-medium text-background disabled:opacity-40">{busy && <LoaderCircle size={13} className="animate-spin"/>}Preview selected</button></div>
    </section>}

    {stage === "preview" && <section className="rounded-3xl border border-border bg-card/45 p-5">
      <div><h3 className="font-display text-sm font-semibold">Select what Bridge may import</h3><p className="mt-1 text-[11px] text-muted-foreground">Review provenance, confidence, stability, redactions, and warnings. Setup stays disabled.</p></div>
      <div className="mt-4 space-y-2">{candidates.map(candidate => <label key={candidate.candidateId} className="flex items-start gap-3 rounded-2xl border border-border p-3.5">
        <input type="checkbox" className="mt-0.5" checked={candidateIds.includes(candidate.candidateId)} onChange={event => toggle(candidate.candidateId, event.target.checked, setCandidateIds, candidateIds)}/>
        <span className="min-w-0 flex-1"><span className="flex flex-wrap items-center gap-1.5"><b className="truncate text-[12px] font-medium">{candidate.title}</b><span className="rounded-full bg-foreground/[0.06] px-1.5 py-0.5 text-[8px] uppercase tracking-wide text-muted-foreground">{kindLabel(candidate.kind)}</span>{SETUP_KINDS.has(candidate.kind) && <span className="rounded-full border border-border px-1.5 py-0.5 text-[8px] uppercase tracking-wide text-muted-foreground">disabled setup</span>}</span><span className="mt-1 block font-mono text-[9.5px] text-muted-foreground">Claude Code · {candidate.source.sourcePathFingerprint.slice(0, 18)}… · {(candidate.confidenceBps / 100).toFixed(0)}% confidence · {candidate.stability.replaceAll("_", " ")}</span>{candidate.redactionSummary.textValuesRedacted + candidate.redactionSummary.structuredFieldsExcluded > 0 && <span className="mt-1 block text-[10px] text-warning">{candidate.redactionSummary.textValuesRedacted} values redacted · {candidate.redactionSummary.structuredFieldsExcluded} structured fields excluded</span>}{candidate.diagnostics.map(diagnostic => <span key={diagnostic.code} className="mt-1 block text-[10px] text-warning">{diagnostic.message}</span>)}</span>
      </label>)}</div>
      {memorySelected && <label className="mt-4 block text-[11px] font-medium text-muted-foreground">Required memory scope<select className={cn(control, "mt-1.5 w-full")} value={memoryScope} onChange={event => setMemoryScope(event.target.value)}><option value="">Choose a Bridge scope…</option><option value="account:local">This Bridge account</option></select><span className="mt-1 block text-[10px] font-normal">Memories are never auto-merged; the selected scope is stored with imported provenance.</span></label>}
      <div className="mt-4 grid gap-3 sm:grid-cols-2"><label className="text-[11px] font-medium text-muted-foreground">Conflict policy<select className={cn(control, "mt-1.5 w-full")} value={conflictPolicy} onChange={event => setConflictPolicy(event.target.value as ExternalImportConflictPolicy)}><option value="skip">Skip conflicts</option><option value="import_as_new_historical_revision">Import changed history as a revision</option><option value="keep_existing">Keep existing memories</option><option value="import_alongside">Import memories alongside</option></select></label><label className="flex items-center gap-2 self-end rounded-xl border border-border px-3 py-2.5 text-[11px] text-muted-foreground"><input type="checkbox" checked={dryRun} onChange={event => setDryRun(event.target.checked)}/>Dry run — validate without writing</label></div>
      <div className="mt-4 flex items-center justify-between"><button type="button" onClick={() => setStage("discovery")} className="text-[11px] text-muted-foreground hover:text-foreground">Back to discovery</button><button type="button" disabled={!canConfirm} onClick={() => setStage("confirm")} className="h-9 rounded-xl bg-foreground px-3.5 text-xs font-medium text-background disabled:opacity-40">Review exact commit</button></div>
    </section>}

    {stage === "confirm" && <section className="rounded-3xl border border-border bg-card/45 p-5">
      <h3 className="font-display text-sm font-semibold">Final confirmation</h3><p className="mt-1 text-[11px] text-muted-foreground">Bridge will {dryRun ? "validate" : "commit"} exactly {selectedCandidates.length} selected {selectedCandidates.length === 1 ? "item" : "items"} in one transaction.</p>
      <ul className="mt-4 space-y-1.5">{selectedCandidates.map(candidate => <li key={candidate.candidateId} className="flex items-center justify-between gap-3 rounded-xl border border-border px-3 py-2 text-[11px]"><span className="truncate">{candidate.title}</span><span className="shrink-0 capitalize text-muted-foreground">{kindLabel(candidate.kind)}</span></li>)}</ul>
      <div className="mt-4 rounded-xl border border-border px-3 py-2.5 text-[10.5px] leading-relaxed text-muted-foreground">Historical chats are immutable and cannot resume the foreign Claude session. Commands, hooks, agents, skills, plugins, and MCP servers remain inert. No repository, prompt, credential, or environment value is changed.</div>
      <div className="mt-4 flex items-center justify-between"><button type="button" disabled={busy} onClick={() => setStage("preview")} className="text-[11px] text-muted-foreground hover:text-foreground">Back to selection</button><button type="button" disabled={busy} onClick={() => void commitSelection()} className="inline-flex h-9 items-center gap-2 rounded-xl bg-foreground px-3.5 text-xs font-medium text-background disabled:opacity-40">{busy && <LoaderCircle size={13} className="animate-spin"/>}{dryRun ? "Run validation" : `Import ${selectedCandidates.length} selected`}</button></div>
    </section>}

    {stage === "result" && commit && <section className="rounded-3xl border border-border bg-card/45 p-5">
      <div className="flex items-start gap-3"><CheckCircle2 size={18} className="mt-0.5 text-success"/><div><h3 className="font-display text-sm font-semibold">{commit.rollbackState === "dry_run" ? "Validation complete" : "Import committed"}</h3><p className="mt-1 text-[11px] text-muted-foreground">{commit.imported} imported · {commit.skipped} duplicate · {commit.changed} changed · {commit.conflicted} conflicted · {commit.rejected} rejected · {commit.unsupported} unsupported</p></div></div>
      <div className="mt-4 grid gap-2 sm:grid-cols-2">{Object.entries(countStatuses(commit)).map(([status, count]) => <div key={status} className="rounded-xl border border-border px-3 py-2"><b className="text-[11px] font-medium capitalize">{status.replaceAll("_", " ")}</b><span className="ml-2 font-mono text-[10px] text-muted-foreground">{count}</span></div>)}</div>
      {commit.diagnostics.map(diagnostic => <p key={diagnostic.code} className="mt-2 text-[10.5px] text-warning">{diagnostic.message}{diagnostic.recovery ? ` ${diagnostic.recovery}` : ""}</p>)}
      <button type="button" onClick={() => setStage("source")} className="mt-4 h-9 rounded-xl border border-border px-3 text-xs text-foreground hover:bg-accent">Import another source</button>
    </section>}
  </div>;
}
