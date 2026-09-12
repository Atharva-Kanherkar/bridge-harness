import { useState } from "react";
import type { MenuBarLayoutToken } from "../../protocol/generated/protocol";
import { layoutExample, layoutTokens, parseLayout } from "./layout";

type Props = { layout: MenuBarLayoutToken[][]; busy: boolean; onSave: (layout: MenuBarLayoutToken[][]) => Promise<void> };
const button = "rounded-md border border-border px-2 py-1 text-xs text-foreground hover:bg-accent disabled:opacity-40";

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
  return <div className="space-y-3 px-4 py-3">
    <p className="text-xs text-muted-foreground">{customActive ? "Custom layout is active." : "Standard display is active."} Edit the items below, then Apply layout.</p>
    <div className="flex flex-wrap gap-2">
      <button type="button" className={button} disabled={busy || !customActive} onClick={() => void onSave([])}>Use standard display</button>
      <button type="button" className={button} disabled={busy} onClick={() => { setDraft([["icon"]]); setActiveLine(0); }}>Icon only</button>
      <button type="button" className={button} disabled={busy} onClick={() => { setDraft([["icon", "space", "used"]]); setActiveLine(0); }}>Icon + used</button>
      <button type="button" className={button} disabled={busy} onClick={() => { setDraft([["icon", "space", "fiveHourUsed"], ["weeklyUsed"]]); setActiveLine(0); }}>Two limits</button>
    </div>
    <div className="flex flex-wrap gap-1.5" aria-label="Available layout items">
      {layoutTokens.map(token => <button key={token.value} type="button" className={button}
        disabled={busy || (draft[activeLine]?.length ?? 0) >= 12 || (token.value === "icon" && draft.flat().includes("icon"))}
        onClick={() => append(token.value)}>+ {token.label}</button>)}
      <button type="button" className={button} disabled={busy || draft.length >= 2}
        onClick={() => { setDraft(draft.length ? [...draft, []] : [[], []]); setActiveLine(1); }}>+ Line break</button>
    </div>
    {draft.map((line, lineIndex) => <div key={lineIndex} className="flex flex-wrap items-center gap-1.5 rounded-lg border border-border p-2">
      <button type="button" aria-pressed={activeLine === lineIndex} className={`${button} aria-pressed:bg-accent`} disabled={busy}
        onClick={() => setActiveLine(lineIndex)}>Line {lineIndex + 1}</button>
      {line.map((token, index) => <span key={`${index}-${token}`} className="inline-flex items-center gap-1 rounded border border-border bg-background px-1.5 py-1 text-xs">
        {layoutTokens.find(item => item.value === token)!.label}
        <button type="button" disabled={busy || index === 0} aria-label={`Move ${token} left on line ${lineIndex + 1}`} className="px-1 hover:bg-accent disabled:opacity-30" onClick={() => move(lineIndex, index, -1)}>←</button>
        <button type="button" disabled={busy || index === line.length - 1} aria-label={`Move ${token} right on line ${lineIndex + 1}`} className="px-1 hover:bg-accent disabled:opacity-30" onClick={() => move(lineIndex, index, 1)}>→</button>
        <button type="button" disabled={busy} aria-label={`Remove ${token} from line ${lineIndex + 1}`} className="px-1 hover:bg-accent" onClick={() => remove(lineIndex, index)}>×</button>
      </span>)}
      {draft.length > 1 && <button type="button" disabled={busy} className={button} onClick={() => { setDraft(draft.filter((_, i) => i !== lineIndex)); setActiveLine(0); }}>Remove line</button>}
    </div>)}
    <div className="u-glass-soft rounded-lg p-3">
      <p className="mb-2 text-xs text-muted-foreground">Example preview</p>
      <pre className="min-h-5 whitespace-pre-wrap text-center font-mono text-xs text-foreground">{draft.length ? layoutExample(draft) || "Empty layout" : "Uses “Beside the icon” above"}</pre>
    </div>
    <div className="flex items-center gap-2">
      <button type="button" className={button} disabled={busy || !dirty || (draft.length > 0 && !draft.flat().some(token => token !== "space" && token !== "dot"))}
        onClick={() => void onSave(draft)}>Apply layout</button>
      <button type="button" className={button} disabled={busy} onClick={() => { setDraft(layout); setActiveLine(0); }}>Reset draft</button>
    </div>
    <details className="text-xs">
      <summary className="cursor-pointer text-muted-foreground">Copy or paste a layout</summary>
      <textarea aria-label="Layout JSON" value={transfer} placeholder={JSON.stringify(draft)} spellCheck={false}
        className="mt-2 min-h-16 w-full rounded-lg border border-border bg-background p-2 font-mono text-xs"
        onChange={event => setTransfer(event.target.value)} />
      <div className="mt-2 flex gap-2">
        <button type="button" className={button} onClick={() => { const text = JSON.stringify(draft); setTransfer(text); void (async () => { try { await navigator.clipboard.writeText(text); setMessage("Layout copied."); } catch { setMessage("Select the layout text above to copy it."); } })(); }}>Copy layout</button>
        <button type="button" className={button} disabled={busy} onClick={() => { try { setDraft(parseLayout(transfer)); setActiveLine(0); setMessage("Layout loaded. Apply to save it."); } catch (error) { setMessage(error instanceof Error ? error.message : "Invalid layout"); } }}>Load pasted layout</button>
      </div>
    </details>
    {message && <p role="status" className="text-xs text-muted-foreground">{message}</p>}
  </div>;
}
