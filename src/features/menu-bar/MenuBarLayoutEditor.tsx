import { useState } from "react";
import { ChevronLeft, ChevronRight, X } from "lucide-react";
import type { MenuBarLayoutToken } from "../../protocol/generated/protocol";
import { GhostButton, PrimaryButton, SettingsBlockRow, TextArea, TextButton } from "../../components/settings/kit";
import { cn } from "@/lib/utils";
import { layoutExample, layoutTokens, parseLayout } from "./layout";

type Props = { layout: MenuBarLayoutToken[][]; busy: boolean; onSave: (layout: MenuBarLayoutToken[][]) => Promise<void> };
/** A token you can add: smaller than a GhostButton because there are fourteen. */
const CHIP = "inline-flex h-6 items-center rounded-md border border-border-card px-2 text-caption text-muted-foreground transition-colors hover:bg-accent hover:text-foreground disabled:opacity-40";
const TOKEN_ACTION = "grid size-5 place-items-center rounded text-muted-foreground transition-colors hover:bg-background hover:text-foreground disabled:opacity-30";

// A bounded token compositor, following CodexBar's array-of-lines model.
// Layout content is data: it never evaluates scripts or provider expressions.
export function MenuBarLayoutEditor({ layout, busy, onSave }: Props) {
  const [draft, setDraft] = useState(layout);
  const [activeLine, setActiveLine] = useState(0);
  const [transfer, setTransfer] = useState("");
  const [message, setMessage] = useState<string | null>(null);
  const dirty = JSON.stringify(draft) !== JSON.stringify(layout);
  const customActive = layout.flat().some(token => token !== "space" && token !== "dot");
  function append(token: MenuBarLayoutToken) {
    const lines = draft.length ? draft.map(line => [...line]) : [[]];
    if (lines[activeLine].length < 12) lines[activeLine].push(token);
    setDraft(lines);
  }
  function remove(line: number, index: number) { setDraft(draft.map((items, i) => i === line ? items.filter((_, j) => j !== index) : items)); }
  function move(line: number, index: number, direction: number) {
    setDraft(draft.map((items, i) => {
      if (i !== line) return items;
      const next = [...items];
      [next[index], next[index + direction]] = [next[index + direction], next[index]];
      return next;
    }));
  }
  const applicable = dirty && (draft.length === 0 || draft.flat().some(token => token !== "space" && token !== "dot"));
  // Rows of the surrounding settings card, built from the kit's controls, so
  // the editor reads as part of the page rather than a tool pasted into it.
  return <>
    <SettingsBlockRow label="Start from" description={`${customActive ? "Custom layout is active." : "Standard display is active."} Edit the items below, then Apply layout.`}>
      <div className="flex flex-wrap gap-2">
        <GhostButton disabled={busy} onClick={() => { setDraft([["icon"]]); setActiveLine(0); }}>Icon only</GhostButton>
        <GhostButton disabled={busy} onClick={() => { setDraft([["icon", "space", "used"]]); setActiveLine(0); }}>Icon + used</GhostButton>
        <GhostButton disabled={busy} onClick={() => { setDraft([["icon", "space", "fiveHourUsed"], ["weeklyUsed"]]); setActiveLine(0); }}>Two limits</GhostButton>
      </div>
    </SettingsBlockRow>
    <SettingsBlockRow label="Add items" description={draft.length > 1 ? `Items go on line ${activeLine + 1}. Choose a line below to switch.` : "Up to twelve items per line, and two lines."}>
      <div className="flex flex-wrap gap-1.5" aria-label="Available layout items">
        {layoutTokens.map(token => <button key={token.value} type="button" className={CHIP}
          disabled={busy || (draft[activeLine]?.length ?? 0) >= 12 || (token.value === "icon" && draft.flat().includes("icon"))}
          onClick={() => append(token.value)}>+ {token.label}</button>)}
        <button type="button" className={CHIP} disabled={busy || draft.length >= 2}
          onClick={() => { setDraft(draft.length ? [...draft, []] : [[], []]); setActiveLine(1); }}>+ Line break</button>
      </div>
      {draft.length > 0 && <div className="mt-3 space-y-2">
        {draft.map((line, lineIndex) => <div key={lineIndex} className="flex flex-wrap items-center gap-1.5 rounded-lg border border-border-card bg-popover p-1.5">
          <button type="button" aria-pressed={activeLine === lineIndex} className={cn(CHIP, "aria-pressed:border-foreground/40 aria-pressed:text-foreground")} disabled={busy}
            onClick={() => setActiveLine(lineIndex)}>Line {lineIndex + 1}</button>
          {line.length === 0 && <span className="px-1 text-caption text-muted-foreground">Empty</span>}
          {line.map((token, index) => <span key={`${index}-${token}`} className="inline-flex h-6 items-center gap-0.5 rounded-md bg-accent pl-2 pr-0.5 text-caption text-foreground">
            {layoutTokens.find(item => item.value === token)!.label}
            <button type="button" disabled={busy || index === 0} aria-label={`Move ${token} left on line ${lineIndex + 1}`} className={TOKEN_ACTION} onClick={() => move(lineIndex, index, -1)}><ChevronLeft size={12} aria-hidden="true" /></button>
            <button type="button" disabled={busy || index === line.length - 1} aria-label={`Move ${token} right on line ${lineIndex + 1}`} className={TOKEN_ACTION} onClick={() => move(lineIndex, index, 1)}><ChevronRight size={12} aria-hidden="true" /></button>
            <button type="button" disabled={busy} aria-label={`Remove ${token} from line ${lineIndex + 1}`} className={TOKEN_ACTION} onClick={() => remove(lineIndex, index)}><X size={12} aria-hidden="true" /></button>
          </span>)}
          {draft.length > 1 && <span className="ml-auto"><TextButton disabled={busy} onClick={() => { setDraft(draft.filter((_, i) => i !== lineIndex)); setActiveLine(0); }}>Remove line</TextButton></span>}
        </div>)}
      </div>}
    </SettingsBlockRow>
    <SettingsBlockRow label="Preview">
      <div className="flex min-h-11 items-center justify-center rounded-lg border border-border-card bg-popover px-3 py-2">
        {draft.length
          ? <pre className="whitespace-pre-wrap text-center font-mono text-caption leading-snug text-foreground">{layoutExample(draft) || "Empty layout"}</pre>
          : <span className="text-meta text-muted-foreground">Uses “Beside the icon” above</span>}
      </div>
    </SettingsBlockRow>
    <div className="flex flex-wrap items-center gap-2 px-3.5 py-2.5">
      <TextButton disabled={busy || !customActive} onClick={() => void onSave([])}>Use standard display</TextButton>
      <span className="flex-1" />
      <TextButton disabled={busy || !dirty} onClick={() => { setDraft(layout); setActiveLine(0); }}>Reset draft</TextButton>
      <PrimaryButton disabled={busy || !applicable} onClick={() => void onSave(draft)}>Apply layout</PrimaryButton>
    </div>
    <details className="group/transfer px-3.5 py-2.5">
      <summary className="flex w-fit cursor-pointer list-none items-center gap-1.5 text-ui text-muted-foreground transition-colors hover:text-foreground [&::-webkit-details-marker]:hidden">
        <ChevronRight size={12} strokeWidth={1.7} aria-hidden="true" className="transition-transform group-open/transfer:rotate-90" />Copy or paste a layout
      </summary>
      <div className="mt-2 space-y-2">
        <TextArea label="Layout JSON" rows={3} value={transfer} placeholder={JSON.stringify(draft)} onChange={setTransfer} />
        <div className="flex flex-wrap gap-2">
          <GhostButton onClick={() => { const text = JSON.stringify(draft); setTransfer(text); void (async () => { try { await navigator.clipboard.writeText(text); setMessage("Layout copied."); } catch { setMessage("Select the layout text above to copy it."); } })(); }}>Copy layout</GhostButton>
          <GhostButton disabled={busy} onClick={() => { try { setDraft(parseLayout(transfer)); setActiveLine(0); setMessage("Layout loaded. Apply to save it."); } catch (error) { setMessage(error instanceof Error ? error.message : "Invalid layout"); } }}>Load pasted layout</GhostButton>
        </div>
      </div>
      {message && <p role="status" className="mt-2 text-meta text-muted-foreground">{message}</p>}
    </details>
  </>;
}
