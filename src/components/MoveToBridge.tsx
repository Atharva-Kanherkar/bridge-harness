import { useEffect, useMemo, useState } from "react";
import { LoaderCircle as CircleNotch } from "lucide-react";
import { bridgeApi } from "../api";
import type { DetectedImportSource, ExternalImportDiscovery } from "../protocol/generated/protocol";
import { addBatch, chunk, emptyReport, groupChats, importableIds, isBackgroundFolder, sourceSummary, type ImportReport } from "../moveToBridge";
import { GhostButton, PrimaryButton, SettingsGroup, SettingsRow, StatusPill, Switch, TextButton } from "./settings/kit";

// "Move to Bridge": find the tools already on this Mac, review what each
// holds by project, import in batches, and say what came over and what did
// not. Nothing is read beyond file listings until the person picks a tool.

type Phase = "found" | "review" | "importing" | "done";

const ROWS_PER_PROJECT = 8;

export function MoveToBridge({ onError }: { onError: (message: string) => void }) {
  const [phase, setPhase] = useState<Phase>("found");
  const [sources, setSources] = useState<DetectedImportSource[] | null>(null);
  const [busy, setBusy] = useState(false);
  const [source, setSource] = useState<DetectedImportSource>();
  const [discovery, setDiscovery] = useState<ExternalImportDiscovery>();
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const [progress, setProgress] = useState({ done: 0, total: 0 });
  const [report, setReport] = useState<ImportReport>(emptyReport());

  const report_ = (error: unknown) => onError(error instanceof Error ? error.message : String(error));

  useEffect(() => {
    let cancelled = false;
    bridgeApi.detectImportSources()
      .then(found => { if (!cancelled) setSources(found.sources); })
      .catch(error => { if (!cancelled) { setSources([]); report_(error); } });
    return () => { cancelled = true; };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const groups = useMemo(() => groupChats(discovery?.artifacts ?? []), [discovery]);
  const memories = useMemo(() => (discovery?.artifacts ?? []).filter(item => item.kind === "memory"), [discovery]);
  const setup = useMemo(() => (discovery?.artifacts ?? []).filter(item => item.kind !== "memory" && item.kind !== "conversation"), [discovery]);
  const people = groups.filter(group => !group.background);
  const chatCount = people.reduce((sum, group) => sum + group.chats.length, 0);
  const selectedChats = people.reduce((sum, group) => sum + group.chats.filter(chat => selected.has(chat.artifactId)).length, 0);
  const selectedBackground = groups.filter(group => group.background).reduce((sum, group) => sum + group.chats.filter(chat => selected.has(chat.artifactId)).length, 0);

  async function review(next: DetectedImportSource) {
    setBusy(true);
    try {
      const found = await bridgeApi.discoverExternalImport({
        provider: next.tool,
        approvedRoots: next.roots,
        selectedExport: null,
        sourceVersion: null,
        schemaVersion: null,
        formatVersions: {},
      });
      setSource(next);
      setDiscovery(found);
      // Chats and memory are selected by default; setup stays a deliberate
      // choice, and background runs from temp folders are left out.
      setSelected(new Set(found.artifacts
        .filter(item => (item.kind === "conversation" && !isBackgroundFolder(item.projectPath)) || item.kind === "memory")
        .map(item => item.artifactId)));
      setExpanded(new Set());
      setPhase("review");
    } catch (error) { report_(error); }
    finally { setBusy(false); }
  }

  async function runImport() {
    if (!discovery || selected.size === 0) return;
    const ids = discovery.artifacts.map(item => item.artifactId).filter(id => selected.has(id));
    const batches = chunk(ids);
    setProgress({ done: 0, total: ids.length });
    setReport(emptyReport());
    setPhase("importing");
    let running = emptyReport();
    for (const batch of batches) {
      try {
        const preview = await bridgeApi.previewExternalImport(discovery, batch);
        const candidateIds = importableIds(preview.candidates);
        const memorySelected = preview.candidates.some(candidate => candidate.kind === "memory" && candidateIds.includes(candidate.candidateId));
        const commit = candidateIds.length === 0 ? null : await bridgeApi.commitExternalImport(discovery.discoveryId, preview.candidates, {
          selectedCandidateIds: candidateIds,
          conflictPolicy: "skip",
          setupActivationPolicy: "disabled",
          memoryScope: memorySelected ? "account:local" : null,
          dryRun: false,
          createdAt: new Date().toISOString(),
        }, batch);
        running = addBatch(running, preview.candidates, commit);
      } catch (error) {
        running = addBatch(running, [], null, error instanceof Error ? error.message : String(error));
      }
      setReport(running);
      setProgress(current => ({ ...current, done: Math.min(current.total, current.done + batch.length) }));
    }
    setPhase("done");
  }

  const setMany = (ids: string[], on: boolean) => setSelected(current => {
    const next = new Set(current);
    for (const id of ids) { if (on) next.add(id); else next.delete(id); }
    return next;
  });

  if (phase === "found") {
    return <SettingsGroup label="Found on this Mac" note="Bridge only lists folders here; nothing is read until you review a tool">
      {sources === null && <SettingsRow label={<span className="flex items-center gap-2 text-muted-foreground"><CircleNotch size={12} strokeWidth={1.7} className="animate-spin" aria-hidden="true" />Looking for other tools…</span>} />}
      {sources?.map(item => <SettingsRow
        key={item.tool}
        label={item.label}
        disabled={!item.installed}
        description={<span className="block">
          <span className="block">{sourceSummary(item)}</span>
          {item.installed && item.note && <span className="block text-muted-foreground">{item.note}</span>}
        </span>}
        control={item.supported && item.installed
          ? <PrimaryButton disabled={busy} onClick={() => void review(item)}>
              {busy && <CircleNotch size={12} strokeWidth={1.7} className="animate-spin" aria-hidden="true" />}Review
            </PrimaryButton>
          : <StatusPill tone={item.installed ? "warning" : "neutral"}>{item.installed ? "Coming" : "Not found"}</StatusPill>}
      />)}
    </SettingsGroup>;
  }

  if (phase === "review" && discovery && source) {
    return <>
      <SettingsGroup
        label={`Review ${source.label}`}
        note={<span className="flex items-center gap-2">
          {selectedChats} of {chatCount} chats selected{selectedBackground > 0 ? `, plus ${selectedBackground} background` : ""}
          <TextButton onClick={() => setPhase("found")}>Back</TextButton>
        </span>}
      >
        {groups.length === 0 && <SettingsRow label="No chats found" description="This tool has no saved chats Bridge can read." />}
        {groups.map(group => {
          const ids = group.chats.map(chat => chat.artifactId);
          const all = ids.every(id => selected.has(id));
          const open = expanded.has(group.key);
          const visible = open ? group.chats : group.chats.slice(0, group.background ? 0 : ROWS_PER_PROJECT);
          return <div key={group.key || "unknown"} data-import-project={group.label}>
            <SettingsRow
              label={<span className="font-medium">{group.label}</span>}
              description={group.background
                ? `${group.chats.length} ${group.chats.length === 1 ? "session" : "sessions"} that ran in a temporary folder, usually SDK probes. Left out unless you turn them on.`
                : `${group.chats.length} ${group.chats.length === 1 ? "chat" : "chats"}`}
              control={<Switch label={`All chats in ${group.label}`} checked={all} onChange={on => setMany(ids, on)} />}
            />
            {visible.map(chat => <SettingsRow
              key={chat.artifactId}
              className="pl-6"
              label={chat.title}
              description={chat.startedAt ? new Date(chat.startedAt).toLocaleString() : undefined}
              control={<Switch label={chat.title} checked={selected.has(chat.artifactId)} onChange={on => setMany([chat.artifactId], on)} />}
            />)}
            {group.chats.length > (group.background ? 0 : ROWS_PER_PROJECT) && <SettingsRow
              className="pl-6"
              label={<TextButton onClick={() => setExpanded(current => { const next = new Set(current); if (open) next.delete(group.key); else next.add(group.key); return next; })}>
                {open ? "Show fewer" : `Show all ${group.chats.length}`}
              </TextButton>}
            />}
          </div>;
        })}
      </SettingsGroup>
      {(memories.length > 0 || setup.length > 0) && <SettingsGroup label="Memory and setup" note="Setup arrives switched off; you turn on each item later">
        {[...memories, ...setup].map(item => <SettingsRow
          key={item.artifactId}
          label={item.sourceLabel}
          description={item.kind.replaceAll("_", " ")}
          control={<Switch label={item.sourceLabel} checked={selected.has(item.artifactId)} onChange={on => setMany([item.artifactId], on)} />}
        />)}
      </SettingsGroup>}
      <SettingsGroup label="Import">
        <SettingsRow
          label="What happens"
          description="Bridge copies the selected chats into its own history. Your files in the other tool are not changed, and a chat Bridge already has is never imported twice."
          control={<PrimaryButton disabled={selected.size === 0} onClick={() => void runImport()}>
            Import {selected.size} selected
          </PrimaryButton>}
        />
      </SettingsGroup>
    </>;
  }

  if (phase === "importing") {
    const percent = progress.total === 0 ? 0 : Math.round((progress.done / progress.total) * 100);
    return <SettingsGroup label="Importing" note={`${progress.done} of ${progress.total}`}>
      <SettingsRow
        label={<span className="flex items-center gap-2"><CircleNotch size={12} strokeWidth={1.7} className="animate-spin" aria-hidden="true" />Bringing your chats over</span>}
        description={<span className="mt-2 block h-1 w-full overflow-hidden rounded-full bg-muted">
          <span role="progressbar" aria-valuenow={percent} aria-valuemin={0} aria-valuemax={100} aria-label="Import progress" className="block h-full bg-foreground transition-[width]" style={{ width: `${percent}%` }} />
        </span>}
      />
      <SettingsRow label="Imported so far" control={<span className="font-mono text-[11px] text-muted-foreground">{report.imported}</span>} />
    </SettingsGroup>;
  }

  return <SettingsGroup label="Done" note={source ? `From ${source.label}` : undefined}>
    <SettingsRow
      label="Came over"
      description="Imported chats are in the sidebar, filed under their project when Bridge knows the folder."
      control={<span data-import-result="imported" className="font-mono text-[11px]">{report.imported}</span>}
    />
    <SettingsRow label="Already in Bridge" description="Imported before, or chats Bridge ran itself." control={<span data-import-result="skipped" className="font-mono text-[11px] text-muted-foreground">{report.alreadyInBridge}</span>} />
    {report.changed > 0 && <SettingsRow label="Changed since the last import" description="Left as they were. Import again with a new revision to bring the changes." control={<span className="font-mono text-[11px] text-muted-foreground">{report.changed}</span>} />}
    {report.failed > 0 && <SettingsRow label="Could not be imported" control={<span className="font-mono text-[11px] text-warning">{report.failed}</span>} />}
    {report.notCarried.length > 0 && <SettingsRow
      label="What did not come over"
      description={<span className="block">{report.notCarried.map(item => <span key={item.text} className="block">{item.count} × {item.text}</span>)}</span>}
    />}
    {report.errors.map(error => <SettingsRow key={error} label={<span className="text-warning">{error}</span>} />)}
    <SettingsRow label="Import from another tool" control={<GhostButton onClick={() => { setPhase("found"); setDiscovery(undefined); }}>Start over</GhostButton>} />
  </SettingsGroup>;
}
