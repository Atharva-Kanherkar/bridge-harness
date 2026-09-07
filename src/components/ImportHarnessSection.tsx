import { useMemo, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { CircleNotch, FileJs, FolderOpen, LockSimple, ShieldCheck, Warning } from "@phosphor-icons/react";
import { bridgeApi } from "../api";
import type {
  ExternalImportArtifact,
  ExternalImportCandidate,
  ExternalImportCommit,
  ExternalImportConflictPolicy,
  ExternalImportDiscovery,
} from "../protocol/generated/protocol";
import {
  GhostButton, PrimaryButton, Select, SettingsGroup, SettingsPage, SettingsRow, StatusPill, Switch,
  TextButton,
} from "./settings/kit";

// The import wizard, in the settings chrome. The flow is unchanged: choose a
// source, pick artifacts, preview candidates, confirm, read the result. Only
// the surface moved, because the wizard's own logic is out of this issue's
// scope and its guarantees (read-only discovery, nothing persisted before the
// final confirmation) are the same guarantees either way.

const CLAUDE_FORMAT_VERSIONS = {
  "claude-auto-memory-v1": "claude-auto-memory-v1",
  "claude-jsonl-v1": "claude-jsonl-v1",
  "claude-settings-v1": "claude-settings-v1",
};

const SETUP_KINDS = new Set(["instruction", "rule", "prompt", "command", "skill", "agent", "hook", "mcp_server", "plugin"]);
type Stage = "source" | "discovery" | "preview" | "confirm" | "result";

const STAGE_LABEL: Record<Stage, string> = {
  source: "Choose a source",
  discovery: "Choose what to read",
  preview: "Choose what to import",
  confirm: "Confirm",
  result: "Result",
};

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
      const chosen = ("__TAURI_INTERNALS__" in window)
        ? await open(kind === "directory"
          ? { directory: true, multiple: false, title: "Choose a Claude Code project or configuration folder" }
          : { directory: false, multiple: false, title: "Choose a Claude Code JSONL export", filters: [{ name: "Claude Code history", extensions: ["jsonl"] }] })
        : (kind === "directory" ? "/mock/.claude" : "/mock/.claude/projects/demo/session.jsonl");
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
    if (!canConfirm || !discovery) return;
    setBusy(true);
    try {
      const result = await bridgeApi.commitExternalImport(discovery.discoveryId, candidates, {
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

  return <SettingsPage
    title="Import"
    description="Bring selected Claude Code history, memory, and reusable setup into Bridge. Discovery is read-only and nothing is stored until the final confirmation."
    action={<StatusPill>{STAGE_LABEL[stage]}</StatusPill>}
  >
    <SettingsGroup label="What this does and does not do">
      <SettingsRow
        lead={<ShieldCheck size={12} weight="regular" aria-hidden="true" />}
        label="Local only"
        description="Source content stays on this Mac. There is no background scan, sync, or cloud upload."
      />
      <SettingsRow
        lead={<LockSimple size={12} weight="regular" aria-hidden="true" />}
        label="Secrets excluded"
        description="Credentials and authorization material are removed before preview or persistence."
      />
      <SettingsRow
        lead={<Warning size={12} weight="regular" aria-hidden="true" />}
        label="Not an encrypted backup"
        description="Bridge's local database follows the app's existing local-state posture and is not an encrypted backup."
      />
    </SettingsGroup>

    {stage === "source" && <SettingsGroup
      label="Source"
      note="Bridge reads only what you choose"
    >
      <SettingsRow
        lead={<FolderOpen size={12} weight="regular" aria-hidden="true" />}
        label="Claude folder"
        description="A project, .claude folder, or Claude home you approve."
        control={<GhostButton disabled={busy} onClick={() => void discover("directory")}>Choose folder</GhostButton>}
      />
      <SettingsRow
        lead={<FileJs size={12} weight="regular" aria-hidden="true" />}
        label="JSONL export"
        description="A version-gated Claude Code transcript selected directly."
        control={<GhostButton disabled={busy} onClick={() => void discover("export")}>Choose export</GhostButton>}
      />
    </SettingsGroup>}

    {stage === "discovery" && discovery && <>
      <SettingsGroup
        label="Discovered"
        note={<span className="flex items-center gap-2">
          {discovery.artifacts.length} found, nothing persisted
          <TextButton onClick={() => setStage("source")}>Change source</TextButton>
        </span>}
      >
        {discovery.artifacts.map(artifact => <SettingsRow
          key={artifact.artifactId}
          label={artifact.sourceLabel}
          description={`${kindLabel(artifact.kind)} · ${formatBytes(artifact.estimatedBytes)}${artifact.requiredSchemaGate ? ` · gated by ${artifact.requiredSchemaGate}` : " · documented source"}`}
          control={<>
            <StatusPill tone={artifact.stability === "stable" ? "success" : "warning"}>
              {artifact.stability.replaceAll("_", " ")}
            </StatusPill>
            <Switch
              label={artifact.sourceLabel}
              checked={artifactIds.includes(artifact.artifactId)}
              onChange={next => toggle(artifact.artifactId, next, setArtifactIds, artifactIds)}
            />
          </>}
        />)}
        {discovery.diagnostics.map(diagnostic => <SettingsRow
          key={`${diagnostic.code}:${diagnostic.sourceLabel}`}
          label={<span className="text-warning">{diagnostic.message}</span>}
        />)}
        <SettingsRow
          label="Nothing is selected by default."
          control={<PrimaryButton disabled={busy || artifactIds.length === 0} onClick={() => void preview()}>
            {busy && <CircleNotch size={12} weight="regular" className="animate-spin" aria-hidden="true" />}Preview selected
          </PrimaryButton>}
        />
      </SettingsGroup>
    </>}

    {stage === "preview" && <>
      <SettingsGroup label="Candidates" note="Setup stays disabled on import">
        {candidates.map(candidate => {
          const unsupported = candidate.kind === "unsupported";
          const redacted = candidate.redactionSummary.textValuesRedacted + candidate.redactionSummary.structuredFieldsExcluded;
          return <SettingsRow
            key={candidate.candidateId}
            label={candidate.title}
            disabled={unsupported}
            description={<span className="block">
              <span className="block truncate font-mono text-[10.5px]">
                Claude Code · {candidate.source.sourcePathFingerprint.slice(0, 18)}… · {(candidate.confidenceBps / 100).toFixed(0)}% confidence · {candidate.stability.replaceAll("_", " ")}
              </span>
              {redacted > 0 && <span className="block text-warning">
                {candidate.redactionSummary.textValuesRedacted} values redacted · {candidate.redactionSummary.structuredFieldsExcluded} structured fields excluded
              </span>}
              {candidate.diagnostics.map(diagnostic => <span key={diagnostic.code} className="block text-warning">{diagnostic.message}</span>)}
            </span>}
            control={<>
              <StatusPill>{kindLabel(candidate.kind)}</StatusPill>
              {SETUP_KINDS.has(candidate.kind) && <StatusPill>disabled setup</StatusPill>}
              <Switch
                label={candidate.title}
                checked={!unsupported && candidateIds.includes(candidate.candidateId)}
                disabled={unsupported}
                onChange={next => toggle(candidate.candidateId, next, setCandidateIds, candidateIds)}
              />
            </>}
          />;
        })}
      </SettingsGroup>

      <SettingsGroup label="How to import">
        {memorySelected && <SettingsRow
          label="Memory scope"
          description="Memories are never auto-merged; the selected scope is stored with imported provenance."
          control={<Select
            label="Required memory scope"
            value={memoryScope}
            options={[
              { value: "", label: "Choose a Bridge scope…" },
              { value: "account:local", label: "This Bridge account" },
            ]}
            onChange={setMemoryScope}
          />}
        />}
        <SettingsRow
          label="Conflict policy"
          control={<Select
            label="Conflict policy"
            value={conflictPolicy}
            width="w-64"
            options={[
              { value: "skip", label: "Skip conflicts" },
              { value: "import_as_new_historical_revision", label: "Import changed history as a revision" },
              { value: "keep_existing", label: "Keep existing memories" },
              { value: "import_alongside", label: "Import memories alongside" },
            ]}
            onChange={value => setConflictPolicy(value as ExternalImportConflictPolicy)}
          />}
        />
        <SettingsRow
          label="Dry run"
          description="Validate without writing."
          control={<Switch label="Dry run" checked={dryRun} onChange={setDryRun} />}
        />
        <SettingsRow
          label={<TextButton onClick={() => setStage("discovery")}>Back to discovery</TextButton>}
          control={<PrimaryButton disabled={!canConfirm} onClick={() => setStage("confirm")}>Review exact commit</PrimaryButton>}
        />
      </SettingsGroup>
    </>}

    {stage === "confirm" && <SettingsGroup
      label="Final confirmation"
      note={`${selectedCandidates.length} ${selectedCandidates.length === 1 ? "item" : "items"}, one transaction`}
    >
      {selectedCandidates.map(candidate => <SettingsRow
        key={candidate.candidateId}
        label={candidate.title}
        control={<StatusPill>{kindLabel(candidate.kind)}</StatusPill>}
      />)}
      <SettingsRow
        label="What stays inert"
        description="Historical chats are immutable and cannot resume the foreign Claude session. Commands, hooks, agents, skills, plugins, and MCP servers remain inert. No repository, prompt, credential, or environment value is changed."
      />
      <SettingsRow
        label={<TextButton disabled={busy} onClick={() => setStage("preview")}>Back to selection</TextButton>}
        control={<PrimaryButton disabled={busy} onClick={() => void commitSelection()}>
          {busy && <CircleNotch size={12} weight="regular" className="animate-spin" aria-hidden="true" />}
          {dryRun ? "Run validation" : `Import ${selectedCandidates.length} selected`}
        </PrimaryButton>}
      />
    </SettingsGroup>}

    {stage === "result" && commit && <SettingsGroup
      label={commit.rollbackState === "dry_run" ? "Validation complete" : "Import committed"}
    >
      <SettingsRow
        label="Outcome"
        description={`${commit.imported} imported · ${commit.skipped} duplicate · ${commit.changed} changed · ${commit.conflicted} conflicted · ${commit.rejected} rejected · ${commit.unsupported} unsupported`}
      />
      {Object.entries(countStatuses(commit)).map(([status, count]) => <SettingsRow
        key={status}
        label={<span className="capitalize">{status.replaceAll("_", " ")}</span>}
        control={<span className="font-mono text-[10.5px] text-muted-foreground">{count}</span>}
      />)}
      {commit.diagnostics.map(diagnostic => <SettingsRow
        key={diagnostic.code}
        label={<span className="text-warning">{diagnostic.message}{diagnostic.recovery ? ` ${diagnostic.recovery}` : ""}</span>}
      />)}
      <SettingsRow
        label="Import another source"
        control={<GhostButton onClick={() => setStage("source")}>Start over</GhostButton>}
      />
    </SettingsGroup>}
  </SettingsPage>;
}
