import { useEffect, useRef } from "react";
import { Send, X } from "lucide-react";
import { Button } from "./ui/button";

/**
 * Bridge's own approval chrome.
 *
 * `effect` is the sentence the host composed and refused on — it names the
 * destination and quotes the literal text. Rendering the host's sentence rather
 * than one assembled here is what keeps the dialog honest about what the second
 * call will actually do.
 */
export function ConnectorApprovalSheet({ effect, detail, busy, onApprove, onDeny, inFlow = false }: {
  effect: string;
  /** Exact payload shown verbatim under the sentence, e.g. the JSON a write sends. */
  detail?: string;
  inFlow?: boolean;
  busy: boolean;
  onApprove: () => void;
  onDeny: () => void;
}) {
  const dialog = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const previous = document.activeElement as HTMLElement | null;
    return () => { if (previous?.isConnected) previous.focus(); };
  }, []);
  useEffect(() => {
    if (!busy && !dialog.current?.contains(document.activeElement)) dialog.current?.querySelector<HTMLButtonElement>("button")?.focus();
  }, [busy]);
  return <div className={inFlow ? "grid bg-background/70" : "absolute inset-0 z-20 grid place-items-end bg-background/70 backdrop-blur-sm"}>
    <div ref={dialog} role="dialog" aria-modal="true" aria-label="Confirm connector action" onKeyDown={event => {
      if (event.key === "Escape") { event.preventDefault(); event.stopPropagation(); if (!busy) onDeny(); }
      if (event.key === "Tab") {
        const buttons = dialog.current?.querySelectorAll<HTMLButtonElement>("button:not(:disabled)");
        if (!buttons?.length) { event.preventDefault(); return; }
        const first = buttons[0]; const last = buttons[buttons.length - 1];
        if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last.focus(); }
        else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first.focus(); }
      }
    }} className="u-glass-popover connector-approval-in m-2.5 w-[calc(100%-1.25rem)] rounded-xl border border-border p-3.5 shadow-2xl">
      <div className="flex items-center gap-2 text-ui font-semibold text-foreground">
        <Send size={14} aria-hidden="true" className="shrink-0" />Send this?
      </div>
      <p className="mt-2 max-h-64 overflow-auto whitespace-pre-wrap break-words rounded-lg bg-muted/60 p-2 text-meta leading-relaxed text-foreground">
        {effect}
      </p>
      {detail !== undefined && <pre aria-label="Exact arguments" className="mt-2 max-h-64 overflow-auto whitespace-pre-wrap break-all rounded-lg bg-muted/60 p-2 font-mono text-caption leading-relaxed text-foreground">{detail}</pre>}
      <div className="mt-3 flex justify-end gap-2">
        <Button variant="ghost" size="sm" onClick={onDeny} disabled={busy}><X size={12} />Cancel</Button>
        <Button size="sm" loading={busy} onClick={onApprove}><Send size={12} />Send</Button>
      </div>
    </div>
  </div>;
}
