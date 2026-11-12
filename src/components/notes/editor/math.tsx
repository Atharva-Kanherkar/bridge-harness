// Editing a formula: TeX in, a live preview below, Enter to keep it.

import { useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import katex from "katex";

export interface MathEdit {
  kind: "inline" | "block";
  /** Where the formula is (editing) or goes (inserting). */
  pos: number;
  latex: string;
  existing: boolean;
  rect: DOMRect | null;
  /** Selected text the new formula replaces. */
  replace?: { from: number; to: number };
}

export function MathPopover({ edit, onSave, onRemove, onClose }: { edit: MathEdit; onSave: (latex: string) => void; onRemove: () => void; onClose: () => void }) {
  const [latex, setLatex] = useState(edit.latex);
  const box = useRef<HTMLTextAreaElement>(null);
  useEffect(() => { box.current?.focus(); box.current?.select(); }, []);
  const preview = useMemo(() => {
    if (!latex.trim()) return { html: "", error: null };
    try { return { html: katex.renderToString(latex, { displayMode: edit.kind === "block", throwOnError: true, output: "htmlAndMathml" }), error: null }; }
    catch (error) { return { html: "", error: error instanceof Error ? error.message.replace(/^KaTeX parse error: /, "") : String(error) }; }
  }, [latex, edit.kind]);
  const rect = edit.rect;
  const left = Math.max(12, Math.min(rect?.left ?? 120, window.innerWidth - 392));
  const top = Math.min((rect?.bottom ?? 160) + 8, window.innerHeight - 260);
  // In the body: a transformed pane would make `fixed` relative to itself.
  return createPortal(<>
    <div aria-hidden="true" className="fixed inset-0 z-40" onMouseDown={onClose} />
    <form
      aria-label={edit.kind === "block" ? "Math block" : "Inline math"}
      style={{ left, top }}
      onSubmit={event => { event.preventDefault(); if (latex.trim()) onSave(latex.trim()); else if (edit.existing) onRemove(); else onClose(); }}
      className="fixed z-50 w-[23rem] rounded-xl bg-popover p-2 shadow-lg ring-1 ring-border"
    >
      <textarea
        ref={box}
        aria-label="TeX"
        value={latex}
        rows={edit.kind === "block" ? 3 : 1}
        spellCheck={false}
        placeholder={edit.kind === "block" ? "\\int_0^1 x^2\\,dx = \\frac{1}{3}" : "e^{i\\pi} + 1 = 0"}
        onChange={event => setLatex(event.target.value)}
        onKeyDown={event => {
          if (event.key === "Escape") { event.preventDefault(); onClose(); }
          if (event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing) { event.preventDefault(); event.currentTarget.form?.requestSubmit(); }
        }}
        className="block w-full resize-none rounded-lg bg-muted px-2.5 py-2 font-mono text-[12.5px] leading-relaxed text-foreground outline-none placeholder:text-faint focus:ring-2 focus:ring-ring"
      />
      <div aria-live="polite" className="mt-2 min-h-8 overflow-x-auto px-1 text-foreground">
        {preview.error
          ? <p className="m-0 text-[12px] text-destructive">{preview.error}</p>
          : preview.html ? <div dangerouslySetInnerHTML={{ __html: preview.html }} /> : <p className="m-0 text-[12px] text-faint">The formula shows here as you type.</p>}
      </div>
      <div className="mt-2 flex items-center gap-1">
        <span className="flex-1 text-[11px] text-faint">Enter to keep · Shift+Enter for a new line</span>
        {edit.existing && <button type="button" onClick={onRemove} className="h-7 rounded-full px-2.5 text-[12px] text-muted-foreground hover:bg-accent hover:text-foreground">Remove</button>}
        <button type="submit" className="h-7 rounded-full bg-primary px-3 text-[12px] font-medium text-primary-foreground hover:opacity-90">Done</button>
      </div>
    </form>
  </>, document.body);
}
