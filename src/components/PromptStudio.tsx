import { useEffect, useRef, useState } from "react";
import { Download, RotateCcw, Save } from "lucide-react";
import { bridgeApi } from "../api";
import type { CompiledPromptPreviewResult, PromptSectionStatePayload, PromptSectionView, PromptStackView, PromptTargetChoice } from "../types";
import { CodeEditor } from "./editor/CodeEditor";
import { cn } from "@/lib/utils";

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

const TARGET_GROUPS: { label: string; targets: { id: PromptTargetChoice; label: string }[] }[] = [
  { label: "Orchestrator", targets: TARGETS.filter(item => item.id === "orchestrator") },
  { label: "Workers", targets: TARGETS.filter(item => item.id.startsWith("worker:")) },
  { label: "Direct", targets: TARGETS.filter(item => item.id === "direct_session") },
];

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
  const fileInputRef = useRef<HTMLInputElement>(null);
  const previousPrefixHashByTarget = useRef<Partial<Record<PromptTargetChoice, string>>>({});

  const stack = stacks[target];

  // One load of every stack at mount so each target row can carry an honest
  // "has overrides" dot. This only fills gaps: any target already populated
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
      setSectionId(current => (current && next.sections.some(item => item.id === current)) ? current : next.sections[0]?.id);
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
  const editorInstanceKey = docKey !== undefined ? `${docKey}:${section?.revisions.length ?? 0}` : undefined;
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

  // Target buttons keep their label as the entire text content — dots and
  // counts render as siblings, never inside the button.
  return <div className="flex h-full min-h-0">
    <aside className="flex w-60 shrink-0 flex-col overflow-hidden border-r border-border/60">
      <nav aria-label="Prompt targets" className="min-h-0 flex-1 overflow-y-auto p-3">
        <h2 className="px-2 pb-2 text-[10.5px] font-semibold uppercase tracking-[0.09em] text-muted-foreground/80">Prompt Studio</h2>
        {TARGET_GROUPS.map(group => <div key={group.label} className="mb-3 last:mb-0">
          <p className="px-2 pb-1 text-[9.5px] font-semibold uppercase tracking-[0.08em] text-muted-foreground/50">{group.label}</p>
          <div className="space-y-0.5">
            {group.targets.map(item => {
              const itemStack = stacks[item.id];
              const hasOverrides = itemStack?.sections.some(sectionItem => sectionItem.state.state !== "default") ?? false;
              return <div
                key={item.id}
                title={hasOverrides ? `${item.label} has overrides` : undefined}
                className="flex items-center gap-1.5"
              >
                <span aria-hidden className="ml-1.5 w-1.5 shrink-0">
                  {hasOverrides && <span className="block h-1.5 w-1.5 rounded-full bg-warning/80" />}
                </span>
                {hasOverrides && <span className="sr-only">Has overrides</span>}
                <button
                  type="button"
                  aria-current={target === item.id}
                  onClick={() => setTarget(item.id)}
                  className={cn(
                    "flex h-8 flex-1 items-center rounded-lg px-2.5 text-left text-[12.5px] transition-colors",
                    target === item.id ? "bg-foreground/[0.07] font-medium text-foreground" : "text-muted-foreground hover:bg-foreground/[0.045] hover:text-foreground",
                  )}
                >{item.label}</button>
              </div>;
            })}
          </div>
          {group.targets.some(item => item.id === target) && stack && stack.sections.length > 0 && (
            <ul role="listbox" aria-label={`${targetLabel} prompt sections`} className="ml-4 mt-1 space-y-0.5 border-l border-border/60 pb-1 pl-2">
              {stack.sections.map(item => <li key={item.id}>
                <button
                  type="button"
                  role="option"
                  aria-selected={sectionId === item.id}
                  onClick={() => setSectionId(item.id)}
                  className={cn("flex w-full items-center gap-2 rounded-lg px-2.5 py-1.5 text-left text-[12px]", sectionId === item.id ? "bg-foreground/[0.07] text-foreground" : "text-muted-foreground hover:bg-foreground/[0.045] hover:text-foreground")}
                >
                  <span className="min-w-0 flex-1 truncate font-mono text-[11.5px]">{item.id}</span>
                  {docKeyFor(target, item.id) in drafts && <span className="h-1.5 w-1.5 shrink-0 rounded-full bg-info" aria-label="Unsaved draft" />}
                  <span className="shrink-0 font-mono text-[10px] tabular-nums text-muted-foreground/70">{item.tokenEstimate} tok</span>
                  {item.state.state === "deleted" && <span className="shrink-0 rounded-full bg-destructive/15 px-1.5 py-0.5 text-[9px] font-semibold uppercase tracking-wide text-destructive">Deleted</span>}
                  {item.state.state === "overridden" && <span className="shrink-0 rounded-full bg-warning/15 px-1.5 py-0.5 text-[9px] font-semibold uppercase tracking-wide text-warning">Modified</span>}
                </button>
              </li>)}
            </ul>
          )}
        </div>)}
      </nav>

      <div className="shrink-0 border-t border-border/60 p-3">
        <div className="flex items-center gap-1">
          <button
            type="button"
            disabled={busy}
            onClick={() => void handleExport()}
            className="inline-flex h-7 items-center gap-1.5 rounded-lg px-2 text-[11px] text-muted-foreground transition-colors hover:bg-foreground/[0.05] hover:text-foreground disabled:opacity-40"
          ><Download size={12} aria-hidden="true" />Export overrides</button>
        </div>
        <input
          ref={fileInputRef}
          type="file"
          accept="application/json"
          aria-label="Import prompt overrides"
          disabled={busy}
          onChange={event => { const { files } = event.target; event.target.value = ""; void handleImportFile(files); }}
          className="mt-1.5 block w-full text-[10.5px] text-muted-foreground file:mr-2 file:rounded-lg file:border-0 file:bg-foreground/[0.07] file:px-2 file:py-1 file:text-[10.5px] file:text-foreground"
        />
        {importResults && <ul className="mt-1.5 space-y-0.5 text-[10px]">
          {importResults.map(item => <li key={item.key} className={item.ok ? "text-muted-foreground" : "text-destructive"}>{item.key}: {item.ok ? "saved" : `failed (${item.message})`}</li>)}
        </ul>}
        {overriddenInTarget.length > 0 && <button
          type="button"
          disabled={busy}
          onClick={() => void handleResetAll()}
          className="mt-1.5 inline-flex h-7 items-center gap-1.5 rounded-lg px-2 text-[11px] text-muted-foreground transition-colors hover:bg-foreground/[0.05] hover:text-foreground disabled:opacity-40"
        ><RotateCcw size={12} aria-hidden="true" />Reset all for {targetLabel}</button>}
      </div>
    </aside>

    <div className="flex min-w-0 flex-1 flex-col overflow-y-auto px-6 pb-10 pt-5">
      {error && <p role="alert" className="mb-3 rounded-lg border border-destructive/40 bg-destructive/10 px-3 py-2 text-[12px] text-destructive">{error}</p>}
      {isEmptyTarget ? <div className="flex flex-1 items-center justify-center p-8">
        <p role="status" className="max-w-sm text-center text-xs leading-relaxed text-muted-foreground">
          Direct sessions run on the provider's own system prompt. There is nothing here for Bridge to override.
        </p>
      </div> : !stack ? <p className="text-xs text-muted-foreground">Loading…</p> : !section ? <p className="text-xs text-muted-foreground">Select a section to edit.</p> : <>
        <header className="mb-3 flex flex-wrap items-center gap-x-3 gap-y-2">
          <div className="flex min-w-0 items-baseline gap-2">
            <span className="shrink-0 text-[12px] text-muted-foreground">{targetLabel}</span>
            <span aria-hidden className="text-muted-foreground/40">/</span>
            <h3 className="truncate font-mono text-[13.5px] font-medium text-foreground">{section.id}</h3>
          </div>
          {section.state.state === "overridden" && <span className="rounded-full border border-warning/30 bg-warning/10 px-2 py-0.5 text-[9.5px] font-semibold uppercase tracking-wide text-warning">Modified</span>}
          {isDeleted && <span className="rounded-full border border-destructive/30 bg-destructive/10 px-2 py-0.5 text-[9.5px] font-semibold uppercase tracking-wide text-destructive">Deleted</span>}
          <div className="ml-auto flex items-center gap-2">
            <span className="font-mono text-[11px] tabular-nums text-muted-foreground/70">{section.tokenEstimate} tok · est</span>
            <button
              type="button"
              disabled={busy || !canReset}
              onClick={() => void handleReset(section.id)}
              className="inline-flex h-8 items-center gap-1.5 rounded-lg px-2.5 text-[11.5px] text-muted-foreground transition-colors hover:bg-foreground/[0.05] hover:text-foreground disabled:opacity-40"
            ><RotateCcw size={12} aria-hidden="true" />Reset {section.id}</button>
            <button
              type="button"
              disabled={busy || !isDirty}
              onClick={() => void handleSave()}
              className="inline-flex h-8 items-center gap-1.5 rounded-lg bg-foreground px-3.5 text-[11.5px] font-medium text-background transition-colors disabled:opacity-40"
            ><Save size={12} aria-hidden="true" />Save {section.id}</button>
          </div>
        </header>

        {warnings.length > 0 && <div role="status" className="mb-3 flex flex-col gap-1 rounded-r-lg border-l-2 border-warning/70 bg-warning/10 px-3.5 py-2.5 text-[11.5px] leading-relaxed text-warning">
          {warnings.map(warning => <p key={warning.marker}>{warning.message}</p>)}
        </div>}

        <div className="u-surface flex min-h-[24rem] flex-1 flex-col overflow-hidden rounded-xl">
          <div className="flex h-8 shrink-0 items-center gap-2 border-b border-border/60 bg-muted/40 px-3.5">
            <span className="font-mono text-[10.5px] text-muted-foreground/80">prompt.md</span>
            {isDirty && <span className="ml-auto flex items-center gap-1.5 text-[10.5px] text-muted-foreground">
              <span aria-hidden className="h-1.5 w-1.5 rounded-full bg-info" />Unsaved draft
            </span>}
          </div>
          <CodeEditor
            key={editorInstanceKey}
            docKey={editorInstanceKey!}
            doc={draftText}
            path="prompt.md"
            readOnly={isDeleted}
            onChange={value => setDrafts(current => ({ ...current, [docKey!]: value }))}
            onSave={() => void handleSave()}
            className="min-h-0 flex-1"
          />
        </div>

        {section.revisions.length > 0 && <section aria-label={`${section.id} revision history`} className="mt-7 shrink-0">
          <h4 className="mb-3 text-[10.5px] font-semibold uppercase tracking-[0.09em] text-muted-foreground/80">History</h4>
          <ol className="flex">
            <li className="relative min-w-0 flex-1 pt-4">
              <span aria-hidden className="absolute left-2 right-0 top-[4px] h-px bg-border/70" />
              <span aria-hidden className="absolute left-0 top-0 h-2 w-2 rounded-full border-2 border-foreground bg-foreground" />
              <p className="truncate text-[12px] font-medium text-foreground">Now</p>
              <p className="truncate text-[10.5px] text-muted-foreground/70">{isDirty ? "unsaved draft" : "current text"}</p>
            </li>
            {[...section.revisions].reverse().map((revision, index, reversed) => <li key={revision.id} className="group relative min-w-0 flex-1 pt-4">
              <span aria-hidden className={cn("absolute top-[4px] h-px bg-border/70", index === reversed.length - 1 ? "left-0 right-[calc(100%-0.5rem)]" : "left-0 right-0")} />
              <span aria-hidden className="absolute left-0 top-0 h-2 w-2 rounded-full border-2 border-muted-foreground/60 bg-background group-hover:border-foreground/80" />
              <p className="truncate text-[12px] font-medium text-foreground">{revision.operation}</p>
              <p className="truncate text-[10.5px] text-muted-foreground/70">{revision.createdAt}</p>
              <button
                type="button"
                disabled={busy}
                aria-label={`Restore ${section.id} to revision ${revision.id} (${revision.operation})`}
                onClick={() => void handleRestore(section.id, revision.id)}
                className="mt-0.5 rounded-md text-[10.5px] text-info opacity-0 pointer-events-none transition-opacity hover:underline focus-visible:opacity-100 focus-visible:pointer-events-auto group-hover:opacity-100 group-hover:pointer-events-auto group-hover:disabled:opacity-40 focus-visible:disabled:opacity-40"
              >Restore</button>
            </li>)}
          </ol>
        </section>}
      </>}

      {/* Rendered regardless of isEmptyTarget/loading state — a direct
         session has no sections to edit, but its provider-layer breakdown
         is still the most relevant fact on the page, and it was never
         gated on section selection before this redesign. */}
      <aside aria-label="Compiled prompt preview and overrides" className="mt-7 shrink-0">
        <details open className="u-surface rounded-xl">
          <summary className="cursor-pointer select-none px-4 py-2.5 text-[10.5px] font-semibold uppercase tracking-[0.09em] text-muted-foreground/80 marker:content-[''] [&::-webkit-details-marker]:hidden">Compiled preview</summary>
          <div className="border-t border-border/60 px-4 py-3">
            {previewError && <p role="alert" className="mb-2 text-[11px] text-destructive">{previewError}</p>}
            {!preview ? <p className="text-[11px] text-muted-foreground">Loading preview…</p> : <div className="space-y-3">
              <div>
                <h4 className="text-[10.5px] font-semibold uppercase tracking-wide text-foreground">Exact Bridge bytes</h4>
                <dl className="mt-1 grid grid-cols-2 gap-x-3 gap-y-0.5 font-mono text-[10px] text-muted-foreground">
                  <dt>Hash</dt><dd className="truncate">{preview.prefixHash}</dd>
                  <dt>ID</dt><dd className="truncate">{preview.prefixId}</dd>
                  <dt>Bytes (exact)</dt><dd>{preview.prefixBytes}</dd>
                  <dt>Tokens (est.)</dt><dd>{preview.prefixTokenEstimate}</dd>
                </dl>
                <p className="mt-1.5 text-[10px] font-medium uppercase tracking-wide text-muted-foreground/70">Stable prefix — exact bytes</p>
                <pre className="mt-0.5 max-h-32 overflow-auto whitespace-pre-wrap break-words rounded-lg border border-border bg-foreground/[0.03] p-2 font-mono text-[10px]">{preview.stablePrefix}</pre>
                <p className="mt-1.5 text-[10px] font-medium uppercase tracking-wide text-muted-foreground/70">Variable suffix — exact bytes</p>
                <pre className="mt-0.5 max-h-24 overflow-auto whitespace-pre-wrap break-words rounded-lg border border-border bg-foreground/[0.03] p-2 font-mono text-[10px]">{preview.variableSuffix}</pre>
              </div>

              {prefixChanged && <p role="status" className="rounded-lg border border-warning/40 bg-warning/10 p-2 text-[10.5px] leading-relaxed text-warning">
                Bridge prefix changed — the next turn is likely a cache miss on the Bridge prefix. Provider-side cache effects are estimated, not measured.
              </p>}

              <div>
                <h4 className="text-[10.5px] font-semibold uppercase tracking-wide text-foreground">Provider layers (not exact)</h4>
                <ul className="mt-1 space-y-1">
                  {preview.providerLayers.map(layer => <li key={`${layer.layer}:${layer.adapter}`} className="rounded-lg border border-border/60 p-1.5 text-[10.5px]">
                    <div className="flex items-center justify-between gap-2">
                      <span className="font-mono">{layer.adapter}</span>
                      <span className="rounded-full bg-foreground/[0.08] px-1.5 py-0.5 text-[9px] uppercase tracking-wide text-muted-foreground">{layer.source}</span>
                    </div>
                    {layer.detail && <p className="mt-0.5 text-muted-foreground">{layer.detail}</p>}
                  </li>)}
                </ul>
              </div>
            </div>}
          </div>
        </details>
      </aside>
    </div>
    <p aria-live="polite" className="sr-only">{announcement}</p>
  </div>;
}

function dedupeWarnings(warnings: { marker: string; message: string }[]): { marker: string; message: string }[] {
  const seen = new Set<string>();
  return warnings.filter(warning => {
    if (seen.has(warning.marker)) return false;
    seen.add(warning.marker);
    return true;
  });
}
