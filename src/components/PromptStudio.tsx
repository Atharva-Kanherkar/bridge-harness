import { useEffect, useRef, useState } from "react";
import { DownloadSimple, Plus } from "@phosphor-icons/react";
import { bridgeApi } from "../api";
import type { CompiledPromptPreviewResult, PromptSectionStatePayload, PromptSectionView, PromptStackView, PromptTargetChoice } from "../types";
import { CodeEditor } from "./editor/CodeEditor";
import {
  GhostButton, SaveBar, Select, SettingsBlockRow, SettingsGroup, SettingsPage, SettingsRow,
  StatusPill, TextButton,
} from "./settings/kit";
import { cn } from "@/lib/utils";

// Prompts: one target at a time, its sections as rows, one section open at a
// time.
//
// The old Prompt Studio drew a third sidebar with targets and their sections
// nested inside it, so the editor started 440px into the window and the column
// moved every time Settings changed page. Target is now a select in the page
// header, sections are rows, and a row opens the editor in the same column.
//
// The compiled preview stays on the *list* page. It describes the target, not
// any one section, and a direct session compiles a prompt while having no
// section to open — gating the preview on a selected section would hide the
// most relevant fact on that target's page.

const TARGETS: { id: PromptTargetChoice; label: string }[] = [
  { id: "orchestrator", label: "Orchestrator" },
  { id: "worker:research", label: "Research" },
  { id: "worker:implementation", label: "Implementation" },
  { id: "worker:verification", label: "Verification" },
  { id: "worker:planning", label: "Planning" },
  { id: "worker:documentation", label: "Documentation" },
  { id: "direct_session", label: "Direct session" },
];
const TARGET_IDS = new Set(TARGETS.map(item => item.id));

// Same vocabulary the compiler/mock lint requires: a draft that drops one of
// these markers still compiles, but the behavior the marker names quietly
// stops working.
const REQUIRED_MARKERS: Record<string, string[]> = {
  delegation_protocol: ["bridge-delegate"],
};

function draftLintWarnings(sectionId: string, text: string): { marker: string; message: string }[] {
  const required = REQUIRED_MARKERS[sectionId];
  if (!required) return [];
  return required
    .filter(marker => !text.includes(marker))
    .map(marker => ({ marker, message: `Typed delegation may stop working because \`${marker}\` is missing.` }));
}

function seedFor(section?: PromptSectionView): string {
  return section?.effectiveText ?? section?.defaultText ?? "";
}

function docKeyFor(target: PromptTargetChoice, sectionId: string): string {
  return `${target}:${sectionId}`;
}

/** Structural validation only — every entry that survives this pass is
 *  guaranteed to be a `{ target, sectionId, text }` triple, so applying it
 *  later can never encounter a shape surprise mid-import. */
function parseOverridesFile(raw: unknown, stacksByTarget: Record<PromptTargetChoice, PromptStackView>): { target: PromptTargetChoice; sectionId: string; text: string }[] {
  if (typeof raw !== "object" || raw === null || Array.isArray(raw)) {
    throw new Error("Import file must be a JSON object keyed by prompt target.");
  }
  const entries: { target: PromptTargetChoice; sectionId: string; text: string }[] = [];
  for (const [targetKey, sectionsValue] of Object.entries(raw as Record<string, unknown>)) {
    if (!TARGET_IDS.has(targetKey as PromptTargetChoice)) throw new Error(`Unknown prompt target "${targetKey}".`);
    const target = targetKey as PromptTargetChoice;
    if (typeof sectionsValue !== "object" || sectionsValue === null || Array.isArray(sectionsValue)) {
      throw new Error(`Sections for "${targetKey}" must be a JSON object.`);
    }
    const validSectionIds = new Set(stacksByTarget[target].sections.map(section => section.id));
    for (const [sectionId, entryValue] of Object.entries(sectionsValue as Record<string, unknown>)) {
      if (!validSectionIds.has(sectionId)) throw new Error(`Unknown section "${sectionId}" for target "${targetKey}".`);
      if (typeof entryValue !== "object" || entryValue === null || Array.isArray(entryValue)) {
        throw new Error(`Malformed override entry for "${targetKey}.${sectionId}".`);
      }
      const entry = entryValue as Record<string, unknown>;
      const text = entry.text;
      if (entry.state !== "overridden" || typeof text !== "string") {
        throw new Error(`Malformed override entry for "${targetKey}.${sectionId}".`);
      }
      entries.push({ target, sectionId, text });
    }
  }
  return entries;
}

/** jsdom's File predates Blob.text(); every real webview has it. */
function readFileText(file: File): Promise<string> {
  if (typeof file.text === "function") return file.text();
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result));
    reader.onerror = () => reject(reader.error ?? new Error("Could not read import file."));
    reader.readAsText(file);
  });
}

export function PromptStudio() {
  const [target, setTarget] = useState<PromptTargetChoice>("orchestrator");
  const [stacks, setStacks] = useState<Partial<Record<PromptTargetChoice, PromptStackView>>>({});
  const [sectionId, setSectionId] = useState<string>();
  const [drafts, setDrafts] = useState<Record<string, string>>({});
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const [announcement, setAnnouncement] = useState("");
  const [preview, setPreview] = useState<CompiledPromptPreviewResult>();
  const [previewError, setPreviewError] = useState<string>();
  const [prefixChanged, setPrefixChanged] = useState(false);
  const [importResults, setImportResults] = useState<{ key: string; ok: boolean; message?: string }[]>();
  // Discarding drops the draft without changing the stored text, so nothing in
  // the editor's own key would move and CodeEditor would keep showing the text
  // that was just thrown away. This counter is the reseed signal.
  const [discards, setDiscards] = useState(0);
  const fileInputRef = useRef<HTMLInputElement>(null);
  const previousPrefixHashByTarget = useRef<Partial<Record<PromptTargetChoice, string>>>({});

  const stack = stacks[target];

  // One load of every stack at mount so each target option can carry an honest
  // "has overrides" note. This only fills gaps: any target already populated
  // by the per-target effect or a mutation handler by the time this resolves
  // keeps its (fresher) entry, so a save/reset/restore/import that lands
  // before this promise settles can never be clobbered by a stale snapshot.
  useEffect(() => {
    let active = true;
    void Promise.all(TARGETS.map(item => bridgeApi.promptStack(item.id))).then(all => {
      if (!active) return;
      setStacks(current => {
        const next = { ...current };
        TARGETS.forEach((item, index) => { next[item.id] = current[item.id] ?? all[index]; });
        return next;
      });
    }).catch(() => undefined);
    return () => { active = false; };
  }, []);

  useEffect(() => {
    let active = true;
    bridgeApi.promptStack(target).then(next => {
      if (!active) return;
      setStacks(current => ({ ...current, [target]: next }));
      setSectionId(current => (current && next.sections.some(item => item.id === current)) ? current : undefined);
    }).catch(err => { if (active) setError(err instanceof Error ? err.message : String(err)); });
    return () => { active = false; };
  }, [target]);

  // Reloads whenever the target changes or a mutation replaces this target's
  // stack, so the preview never drifts from what Save/Reset/Restore just did.
  useEffect(() => {
    if (!stack) return;
    let active = true;
    bridgeApi.previewCompiledPrompt(target).then(next => {
      if (!active) return;
      const previousHash = previousPrefixHashByTarget.current[target];
      setPrefixChanged(previousHash !== undefined && previousHash !== next.prefixHash);
      previousPrefixHashByTarget.current[target] = next.prefixHash;
      setPreview(next);
    }).catch(err => { if (active) setPreviewError(err instanceof Error ? err.message : String(err)); });
    return () => { active = false; };
  }, [target, stack]);

  const targetLabel = TARGETS.find(item => item.id === target)?.label ?? target;
  const section = stack?.sections.find(item => item.id === sectionId);
  const docKey = sectionId ? docKeyFor(target, sectionId) : undefined;
  // CodeEditor only re-seeds its document when `docKey` changes (by design —
  // it owns undo history and cursor state otherwise). A save, reset, or
  // restore must still force a fresh seed, so the revision count rides along
  // in the key the editor actually sees; the stable `docKey` above is what
  // dirty tracking keys off, per the drafts-survive-navigation contract.
  const editorInstanceKey = docKey !== undefined ? `${docKey}:${section?.revisions.length ?? 0}:${discards}` : undefined;
  const draftText = docKey !== undefined && docKey in drafts ? drafts[docKey] : seedFor(section);
  const isDirty = docKey !== undefined && docKey in drafts;
  const isDeleted = section?.state.state === "deleted";
  const canReset = section !== undefined && section.state.state !== "default";
  const warnings = section
    ? dedupeWarnings([...section.lintWarnings, ...draftLintWarnings(section.id, draftText)])
    : [];
  const isEmptyTarget = stack !== undefined && stack.sections.length === 0;
  const overriddenInTarget = stack?.sections.filter(item => item.state.state !== "default") ?? [];

  async function withBusy(action: () => Promise<void>) {
    setBusy(true);
    setError(undefined);
    try { await action(); }
    catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      setError(message);
      setAnnouncement(`Failed: ${message}`);
    }
    finally { setBusy(false); }
  }

  const handleSave = () => withBusy(async () => {
    if (!sectionId || docKey === undefined) return;
    const text = drafts[docKey];
    if (text === undefined) return;
    const result = await bridgeApi.savePromptSection(target, sectionId, text);
    setStacks(current => ({ ...current, [target]: result.stack }));
    setDrafts(current => { const next = { ...current }; delete next[docKey]; return next; });
    setAnnouncement(`Saved ${sectionId}.`);
  });

  const handleReset = (id: string) => withBusy(async () => {
    const result = await bridgeApi.resetPromptSection(target, id);
    setStacks(current => ({ ...current, [target]: result.stack }));
    const key = docKeyFor(target, id);
    setDrafts(current => { const next = { ...current }; delete next[key]; return next; });
    setAnnouncement(`Reset ${id}.`);
  });

  const handleResetAll = () => withBusy(async () => {
    if (!stack) return;
    const overridden = stack.sections.filter(item => item.state.state !== "default");
    if (overridden.length === 0) return;
    if (!window.confirm(`Reset all ${overridden.length} customized section(s) for ${targetLabel}?`)) return;
    let latest = stack;
    for (const item of overridden) latest = (await bridgeApi.resetPromptSection(target, item.id)).stack;
    setStacks(current => ({ ...current, [target]: latest }));
    setDrafts(current => {
      const next = { ...current };
      for (const item of overridden) delete next[docKeyFor(target, item.id)];
      return next;
    });
    setAnnouncement(`Reset ${overridden.length} section(s) for ${targetLabel}.`);
  });

  const handleRestore = (id: string, revisionId: number) => withBusy(async () => {
    const result = await bridgeApi.restorePromptRevision(target, id, revisionId);
    setStacks(current => ({ ...current, [target]: result.stack }));
    const key = docKeyFor(target, id);
    setDrafts(current => { const next = { ...current }; delete next[key]; return next; });
    setAnnouncement(`Restored ${id} to revision ${revisionId}.`);
  });

  const handleExport = () => withBusy(async () => {
    const allStacks = await Promise.all(TARGETS.map(item => bridgeApi.promptStack(item.id)));
    const payload: Record<string, Record<string, PromptSectionStatePayload>> = {};
    TARGETS.forEach((item, index) => {
      const overrides: Record<string, PromptSectionStatePayload> = {};
      for (const sectionItem of allStacks[index].sections) {
        if (sectionItem.state.state === "overridden") overrides[sectionItem.id] = sectionItem.state;
      }
      if (Object.keys(overrides).length > 0) payload[item.id] = overrides;
    });
    const blob = new Blob([JSON.stringify(payload, null, 2)], { type: "application/json" });
    const url = URL.createObjectURL(blob);
    const anchor = document.createElement("a");
    anchor.href = url;
    anchor.download = "bridge-prompt-overrides.json";
    anchor.click();
    URL.revokeObjectURL(url);
    setAnnouncement("Exported prompt overrides.");
  });

  const handleImportFile = (files: FileList | null) => {
    const file = files?.[0];
    if (!file) return;
    return withBusy(async () => {
      const text = await readFileText(file);
      let parsed: unknown;
      try { parsed = JSON.parse(text); }
      catch { throw new Error("Import file is not valid JSON."); }

      const stacksByTarget = Object.fromEntries(
        await Promise.all(TARGETS.map(async item => [item.id, await bridgeApi.promptStack(item.id)] as const)),
      ) as Record<PromptTargetChoice, PromptStackView>;
      // Every entry is validated against the current stacks before anything
      // is applied — a single bad entry rejects the whole file untouched.
      const entries = parseOverridesFile(parsed, stacksByTarget);

      const results = await Promise.all(entries.map(async entry => {
        try {
          await bridgeApi.savePromptSection(entry.target, entry.sectionId, entry.text);
          return { key: `${entry.target}:${entry.sectionId}`, ok: true as const };
        } catch (err) {
          return { key: `${entry.target}:${entry.sectionId}`, ok: false as const, message: err instanceof Error ? err.message : String(err) };
        }
      }));
      setImportResults(results);

      const touchedTargets = [...new Set(entries.map(entry => entry.target))];
      const refreshed = await Promise.all(touchedTargets.map(item => bridgeApi.promptStack(item)));
      setStacks(current => {
        const next = { ...current };
        touchedTargets.forEach((item, index) => { next[item] = refreshed[index]; });
        return next;
      });

      const succeeded = results.filter(item => item.ok).length;
      const failed = results.length - succeeded;
      setAnnouncement(`Imported ${succeeded} section(s)${failed > 0 ? `, ${failed} failed` : ""}.`);
    });
  };

  const live = <p aria-live="polite" className="sr-only">{announcement}</p>;
  const errorRow = error && <SettingsGroup>
    <SettingsRow label={<span role="alert" className="text-destructive">{error}</span>} />
  </SettingsGroup>;

  if (section && sectionId) {
    return <SettingsPage
      title={section.id}
      breadcrumb={[
        { label: "Prompts", onClick: () => setSectionId(undefined) },
        { label: targetLabel, onClick: () => setSectionId(undefined) },
        { label: section.id },
      ]}
      description="Bridge's own text for this section. Prompts change behavior, never permissions."
      action={<TextButton
        disabled={busy || !canReset}
        onClick={() => void handleReset(section.id)}
      >Reset {section.id}</TextButton>}
    >
      {errorRow}

      {warnings.length > 0 && <div role="status" className="rounded-xl border border-warning/30 bg-warning/10 px-3.5 py-2.5 text-[11.5px] leading-relaxed text-warning">
        {warnings.map(warning => <p key={warning.marker}>{warning.message}</p>)}
      </div>}

      <SettingsGroup
        label="prompt.md"
        note={<span className="flex items-center gap-2">
          <span className="font-mono text-[10.5px]">{section.tokenEstimate} tok est</span>
          {section.state.state === "overridden" && <StatusPill tone="warning">Modified</StatusPill>}
          {isDeleted && <StatusPill tone="destructive">Deleted</StatusPill>}
          {isDirty && <StatusPill tone="info">Unsaved draft</StatusPill>}
        </span>}
      >
        <SettingsBlockRow>
          <div className="overflow-hidden rounded-lg border border-border-card bg-popover">
            <CodeEditor
              key={editorInstanceKey}
              docKey={editorInstanceKey!}
              doc={draftText}
              path="prompt.md"
              readOnly={isDeleted}
              onChange={value => setDrafts(current => ({ ...current, [docKey!]: value }))}
              onSave={() => void handleSave()}
              className="min-h-[22rem]"
            />
          </div>
        </SettingsBlockRow>
      </SettingsGroup>

      {section.revisions.length > 0 && <SettingsGroup label="History" note="Most recent first">
        <SettingsRow label="Now" description={isDirty ? "unsaved draft" : "current text"} />
        {[...section.revisions].reverse().map(revision => <SettingsRow
          key={revision.id}
          label={revision.operation}
          description={revision.createdAt}
          mono
          control={<TextButton
            disabled={busy}
            ariaLabel={`Restore ${section.id} to revision ${revision.id} (${revision.operation})`}
            onClick={() => void handleRestore(section.id, revision.id)}
          >Restore</TextButton>}
        />)}
      </SettingsGroup>}

      <SaveBar
        dirty={isDirty}
        saving={busy}
        onSave={() => void handleSave()}
        onDiscard={() => {
          setDrafts(current => { const next = { ...current }; delete next[docKey!]; return next; });
          setDiscards(count => count + 1);
        }}
      />
      {live}
    </SettingsPage>;
  }

  return <SettingsPage
    title="Prompts"
    description="Bridge's own prompt text, per target. Prompts change behavior, never permissions."
    action={<>
      <Select
        label="Prompt target"
        value={target}
        width="w-44"
        options={TARGETS.map(item => ({
          value: item.id,
          label: item.label,
          description: stacks[item.id]?.sections.some(entry => entry.state.state !== "default")
            ? "Has overrides"
            : undefined,
        }))}
        onChange={value => { setTarget(value as PromptTargetChoice); setSectionId(undefined); }}
      />
    </>}
  >
    {errorRow}

    <SettingsGroup
      label="Sections"
      note={<span className="flex items-center gap-2">
        <GhostButton disabled={busy} onClick={() => void handleExport()}>
          <DownloadSimple size={12} weight="regular" aria-hidden="true" />Export overrides
        </GhostButton>
        <GhostButton disabled={busy} onClick={() => fileInputRef.current?.click()}>
          <Plus size={12} weight="regular" aria-hidden="true" />Import overrides
        </GhostButton>
        {overriddenInTarget.length > 0 && <TextButton disabled={busy} onClick={() => void handleResetAll()}>
          Reset all for {targetLabel}
        </TextButton>}
      </span>}
    >
      {isEmptyTarget
        ? <SettingsRow label="Direct sessions run on the provider's own system prompt. There is nothing here for Bridge to override." />
        : !stack
          ? <SettingsRow label="Loading…" />
          : stack.sections.map(item => {
              const key = docKeyFor(target, item.id);
              return <SettingsRow
                key={item.id}
                label={<span className="font-mono text-[11.5px]">{item.id}</span>}
                openLabel={`Edit ${item.id}`}
                description={`${item.tokenEstimate} tok`}
                mono
                onOpen={() => setSectionId(item.id)}
                control={<>
                  {key in drafts && <StatusPill tone="info">Unsaved</StatusPill>}
                  {item.state.state === "deleted" && <StatusPill tone="destructive">Deleted</StatusPill>}
                  {item.state.state === "overridden" && <StatusPill tone="warning">Modified</StatusPill>}
                </>}
              />;
            })}
    </SettingsGroup>

    <input
      ref={fileInputRef}
      type="file"
      accept="application/json"
      aria-label="Import prompt overrides"
      disabled={busy}
      onChange={event => { const { files } = event.target; event.target.value = ""; void handleImportFile(files); }}
      className="sr-only"
    />
    {importResults && <SettingsGroup label="Last import">
      {importResults.map(item => <SettingsRow
        key={item.key}
        label={<span className={cn("font-mono text-[10.5px]", !item.ok && "text-destructive")}>{item.key}</span>}
        control={<span className={cn("text-[11px]", item.ok ? "text-muted-foreground" : "text-destructive")}>
          {item.ok ? "saved" : `failed (${item.message})`}
        </span>}
      />)}
    </SettingsGroup>}

    <CompiledPreview preview={preview} previewError={previewError} prefixChanged={prefixChanged} />
    {live}
  </SettingsPage>;
}

/** What Bridge actually sends, split into the bytes it owns and the layers it
 *  can only describe. A property of the target, so it stays on the list page. */
function CompiledPreview({ preview, previewError, prefixChanged }: {
  preview?: CompiledPromptPreviewResult;
  previewError?: string;
  prefixChanged: boolean;
}) {
  return <SettingsGroup label="Compiled preview" note="What this target actually sends">
    {previewError && <SettingsRow label={<span role="alert" className="text-destructive">{previewError}</span>} />}
    {!preview
      ? <SettingsRow label="Loading preview…" />
      : <>
          <SettingsRow label="Exact Bridge bytes" description={`${preview.prefixBytes} bytes · ${preview.prefixTokenEstimate} tokens est`} />
          <SettingsRow label="Hash" description={preview.prefixHash} mono />
          <SettingsRow label="ID" description={preview.prefixId} mono />
          <SettingsBlockRow label="Stable prefix" description="Exact bytes">
            <pre className="max-h-32 overflow-auto whitespace-pre-wrap break-words rounded-lg border border-border-card bg-popover p-2 font-mono text-[10.5px]">{preview.stablePrefix}</pre>
          </SettingsBlockRow>
          <SettingsBlockRow label="Variable suffix" description="Exact bytes">
            <pre className="max-h-24 overflow-auto whitespace-pre-wrap break-words rounded-lg border border-border-card bg-popover p-2 font-mono text-[10.5px]">{preview.variableSuffix}</pre>
          </SettingsBlockRow>
          {prefixChanged && <SettingsRow
            label={<span role="status" className="text-warning">Bridge prefix changed</span>}
            description="The next turn is likely a cache miss on the Bridge prefix. Provider-side cache effects are estimated, not measured."
          />}
          <SettingsRow label="Provider layers (not exact)" description="Bridge describes these; it does not own their bytes." />
          {preview.providerLayers.map(layer => <SettingsRow
            key={`${layer.layer}:${layer.adapter}`}
            label={<span className="font-mono text-[11.5px]">{layer.adapter}</span>}
            description={layer.detail}
            control={<StatusPill>{layer.source}</StatusPill>}
          />)}
        </>}
  </SettingsGroup>;
}

function dedupeWarnings(warnings: { marker: string; message: string }[]): { marker: string; message: string }[] {
  const seen = new Set<string>();
  return warnings.filter(warning => {
    if (seen.has(warning.marker)) return false;
    seen.add(warning.marker);
    return true;
  });
}
