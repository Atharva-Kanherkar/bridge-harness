import { useEffect, useState } from "react";
import { RotateCcw, Save } from "lucide-react";
import { bridgeApi } from "../api";
import type { PromptSectionView, PromptStackView, PromptTargetChoice } from "../types";
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

export function PromptStudio() {
  const [target, setTarget] = useState<PromptTargetChoice>("orchestrator");
  const [stacks, setStacks] = useState<Partial<Record<PromptTargetChoice, PromptStackView>>>({});
  const [sectionId, setSectionId] = useState<string>();
  const [drafts, setDrafts] = useState<Record<string, string>>({});
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const [announcement, setAnnouncement] = useState("");

  const stack = stacks[target];

  useEffect(() => {
    let active = true;
    bridgeApi.promptStack(target).then(next => {
      if (!active) return;
      setStacks(current => ({ ...current, [target]: next }));
      setSectionId(current => (current && next.sections.some(item => item.id === current)) ? current : next.sections[0]?.id);
    }).catch(err => { if (active) setError(err instanceof Error ? err.message : String(err)); });
    return () => { active = false; };
  }, [target]);

  const targetLabel = TARGETS.find(item => item.id === target)?.label ?? target;
  const section = stack?.sections.find(item => item.id === sectionId);
  const docKey = sectionId ? docKeyFor(target, sectionId) : undefined;
  // CodeEditor only re-seeds its document when `docKey` changes (by design —
  // it owns undo history and cursor state otherwise). A save or reset must
  // still force a fresh seed, so the revision count rides along in the key
  // the editor actually sees; the stable `docKey` above is what dirty
  // tracking keys off, per the drafts-survive-navigation contract.
  const editorInstanceKey = docKey !== undefined ? `${docKey}:${section?.revisions.length ?? 0}` : undefined;
  const draftText = docKey !== undefined && docKey in drafts ? drafts[docKey] : seedFor(section);
  const isDirty = docKey !== undefined && docKey in drafts;
  const isDeleted = section?.state.state === "deleted";
  const canReset = section !== undefined && section.state.state !== "default";
  const warnings = section
    ? dedupeWarnings([...section.lintWarnings, ...draftLintWarnings(section.id, draftText)])
    : [];
  const isEmptyTarget = stack !== undefined && stack.sections.length === 0;

  async function withBusy(action: () => Promise<void>) {
    setBusy(true);
    setError(undefined);
    try { await action(); }
    catch (err) { setError(err instanceof Error ? err.message : String(err)); }
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

  return <div className="flex h-full min-h-0">
    <nav aria-label="Prompt targets" className="w-44 shrink-0 border-r border-border/60 p-3">
      {TARGETS.map(item => <button
        type="button"
        key={item.id}
        aria-current={target === item.id}
        onClick={() => setTarget(item.id)}
        className={cn(
          "flex h-9 w-full items-center rounded-xl px-3 text-left text-[13px] transition-colors",
          target === item.id ? "bg-foreground/[0.08] text-foreground" : "text-muted-foreground hover:bg-foreground/[0.045] hover:text-foreground",
        )}
      >{item.label}</button>)}
    </nav>

    {isEmptyTarget ? <div className="flex flex-1 items-center justify-center p-8">
      <p role="status" className="max-w-sm text-center text-xs text-muted-foreground">
        Direct sessions run on the provider's own system prompt. There is nothing here for Bridge to override.
      </p>
    </div> : <>
      <div className="w-64 shrink-0 overflow-y-auto border-r border-border/60 p-3">
        <h3 className="mb-2 text-[11px] font-medium text-muted-foreground">Sections</h3>
        {!stack ? <p className="text-xs text-muted-foreground">Loading…</p> : <ul role="listbox" aria-label={`${targetLabel} prompt sections`} className="space-y-1">
          {stack.sections.map(item => <li key={item.id}>
            <button
              type="button"
              role="option"
              aria-selected={sectionId === item.id}
              onClick={() => setSectionId(item.id)}
              className={cn("flex w-full items-center gap-2 rounded-xl px-3 py-2 text-left text-[12.5px]", sectionId === item.id ? "bg-foreground/[0.08] text-foreground" : "text-muted-foreground hover:bg-foreground/[0.045] hover:text-foreground")}
            >
              <span className="min-w-0 flex-1 truncate font-mono">{item.id}</span>
              <span className="shrink-0 text-[10px] text-muted-foreground/80">{item.tokenEstimate} tok</span>
              {item.state.state === "deleted" && <span className="shrink-0 rounded-full bg-destructive/15 px-1.5 py-0.5 text-[9px] font-semibold uppercase tracking-wide text-destructive">Deleted</span>}
              {item.state.state === "overridden" && <span className="shrink-0 rounded-full bg-warning/15 px-1.5 py-0.5 text-[9px] font-semibold uppercase tracking-wide text-warning">Modified</span>}
            </button>
          </li>)}
        </ul>}
        {stack && stack.sections.some(item => item.state.state !== "default") && <button
          type="button"
          disabled={busy}
          onClick={() => void handleResetAll()}
          className="mt-3 inline-flex h-8 items-center gap-1.5 rounded-xl px-2.5 text-[11px] text-muted-foreground hover:bg-foreground/[0.05] disabled:opacity-40"
        ><RotateCcw size={12} aria-hidden="true" />Reset all for {targetLabel}</button>}
      </div>

      <div className="flex min-w-0 flex-1 flex-col p-4">
        {error && <p role="alert" className="mb-2 text-xs text-destructive">{error}</p>}
        {!stack ? <p className="text-xs text-muted-foreground">Loading…</p> : !section ? <p className="text-xs text-muted-foreground">Select a section to edit.</p> : <>
          <div className="mb-2 flex items-center justify-between gap-2">
            <h3 className="font-mono text-[13px] text-foreground">{section.id}</h3>
            <div className="flex items-center gap-2">
              <button
                type="button"
                disabled={busy || !canReset}
                onClick={() => void handleReset(section.id)}
                className="inline-flex h-8 items-center gap-1.5 rounded-xl px-2.5 text-[11px] text-muted-foreground hover:bg-foreground/[0.05] disabled:opacity-40"
              ><RotateCcw size={12} aria-hidden="true" />Reset {section.id}</button>
              <button
                type="button"
                disabled={busy || !isDirty}
                onClick={() => void handleSave()}
                className="inline-flex h-8 items-center gap-1.5 rounded-xl bg-foreground px-3 text-[11px] font-medium text-background disabled:opacity-40"
              ><Save size={12} aria-hidden="true" />Save {section.id}</button>
            </div>
          </div>
          {warnings.length > 0 && <div role="status" className="mb-2 flex flex-col gap-1 rounded-xl border border-warning/40 bg-warning/10 p-2.5 text-[11px] text-warning">
            {warnings.map(warning => <p key={warning.marker}>{warning.message}</p>)}
          </div>}
          <div className="min-h-0 flex-1 overflow-hidden rounded-xl border border-border">
            <CodeEditor
              key={editorInstanceKey}
              docKey={editorInstanceKey!}
              doc={draftText}
              path="prompt.md"
              readOnly={isDeleted}
              onChange={value => setDrafts(current => ({ ...current, [docKey!]: value }))}
              onSave={() => void handleSave()}
              className="h-full"
            />
          </div>
        </>}
      </div>
    </>}
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
