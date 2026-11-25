// Editing a formula: TeX in, a live preview below, Enter to keep it.

import { useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import katex from "katex";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";

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
      <div className="rounded-lg bg-muted px-2.5 py-2 font-mono text-meta leading-relaxed has-focus-visible:ring-2 has-focus-visible:ring-ring">
      <Textarea unstyled size="bare"
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
        className="block w-full"
      />
      </div>
      <div aria-live="polite" className="mt-2 min-h-8 overflow-x-auto px-1 text-foreground">
        {preview.error
          ? <p className="m-0 text-meta text-destructive">{preview.error}</p>
          : preview.html ? <div dangerouslySetInnerHTML={{ __html: preview.html }} /> : <p className="m-0 text-meta text-faint">The formula shows here as you type.</p>}
      </div>
      <div className="mt-2 flex items-center gap-1">
        <span className="flex-1 text-caption text-faint">Enter to keep · Shift+Enter for a new line</span>
        {edit.existing && <Button variant="quiet" size="xs" onClick={onRemove}>Remove</Button>}
        <Button variant="default" size="xs" type="submit">Done</Button>
      </div>
    </form>
  </>, document.body);
}
