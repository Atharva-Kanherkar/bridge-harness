/**
 * A call to Bridge's `visualize` tool, as the transcript shows it.
 *
 * Drawn: a card whose body is the sandboxed MCP Apps view. Still being
 * written: a quiet placeholder. Refused by the server: one muted line the
 * reader can open for the reasons, never an empty frame. The model usually
 * fixes a refusal on its next call, which then draws as a card of its own.
 */

import { createContext, useContext, useState } from "react";
import { ChartColumn, ChevronRight, FileText, Maximize2, Minimize2, Network } from "lucide-react";
import { Dialog, DialogPopup } from "@/components/ui/dialog";
import { cn } from "@/lib/utils";
import { McpAppFrame, useNearViewport, type FrameActions } from "../mcp-apps/McpAppFrame";
import { parseVisualSpec, type VisualSpec } from "../mcp-apps/spec";
import type { ConversationItem } from "../conversation";
import { visualCallInput, visualCallRefusal, visualCallState } from "../transcript/visual";
import { useDarkTheme } from "./Markdown";

/** What a visual may do in this transcript: propose a follow-up, open a source. */
export const VisualActionsContext = createContext<FrameActions | null>(null);

function FamilyIcon({ spec }: { spec: VisualSpec }) {
  const family = spec.blocks[0]?.family;
  const Icon = family === "diagram" ? Network : family === "document" ? FileText : ChartColumn;
  return <Icon size={14} className="shrink-0 text-muted-foreground" aria-hidden="true" />;
}

function Placeholder({ title }: { title?: string }) {
  return (
    <div data-visual-state="drawing" className="rounded-2xl border border-dashed border-border px-4 py-3 text-[12.5px] text-muted-foreground">
      <span className="animate-pulse">Drawing{title ? ` “${title}”` : " a visual"}…</span>
    </div>
  );
}

function QuietRow({ label, detail }: { label: string; detail?: string }) {
  const [open, setOpen] = useState(false);
  return (
    <div data-visual-state="refused" className="min-w-0 text-[12px] text-muted-foreground">
      <button
        type="button"
        className="flex min-h-7 items-center gap-2 rounded-md px-1.5 text-left transition-colors hover:bg-accent/50 disabled:cursor-default"
        aria-expanded={detail ? open : undefined}
        disabled={!detail}
        onClick={() => setOpen(!open)}
      >
        <ChartColumn size={13} aria-hidden="true" />
        <span>{label}</span>
        {detail && <ChevronRight size={12} className={cn("transition-transform", open && "rotate-90")} aria-hidden="true" />}
      </button>
      {open && detail && <pre className="ml-6 mt-1 max-h-60 overflow-auto whitespace-pre-wrap rounded-lg bg-code p-3 font-mono text-[11.5px] leading-relaxed">{detail}</pre>}
    </div>
  );
}

function VisualCard({ spec, input }: { spec: VisualSpec; input: Record<string, unknown> }) {
  const dark = useDarkTheme();
  const theme = dark ? "dark" : "light";
  const actions = useContext(VisualActionsContext) ?? undefined;
  const [expanded, setExpanded] = useState(false);
  const [height, setHeight] = useState(320);
  const [ref, near] = useNearViewport<HTMLDivElement>();

  const header = (fullscreen: boolean) => (
    <div className="flex min-w-0 items-center gap-2.5 border-b border-border px-4 py-2.5">
      <FamilyIcon spec={spec} />
      <span className="truncate text-[13px] font-medium text-foreground">{spec.title}</span>
      {spec.subtitle && <span className="hidden truncate text-[12.5px] text-muted-foreground sm:inline">{spec.subtitle}</span>}
      <span className="flex-1" />
      <span className="hidden shrink-0 items-center gap-1.5 rounded-full border border-border px-2 py-0.5 font-mono text-[10.5px] text-muted-foreground sm:inline-flex" title="Drawn by Bridge's visualize tool from data the model sent">
        <svg width="6" height="6" aria-hidden="true"><circle cx="3" cy="3" r="3" fill="var(--success)" /></svg>
        bridge.visualize
      </span>
      <button
        type="button"
        className="code-block-copy shrink-0"
        onClick={() => setExpanded(!fullscreen)}
        aria-label={fullscreen ? "Exit fullscreen" : "Expand visual"}
        title={fullscreen ? "Exit fullscreen (Esc)" : "Expand"}
      >
        {fullscreen ? <Minimize2 size={12} aria-hidden="true" /> : <Maximize2 size={12} aria-hidden="true" />}
        {fullscreen ? "Close" : "Expand"}
      </button>
    </div>
  );

  return (
    <div ref={ref} data-visual-state="drawn" className="min-w-0 overflow-hidden rounded-2xl border border-border bg-card">
      {header(false)}
      {near
        ? <McpAppFrame input={input} theme={theme} actions={actions} title={spec.title} initialHeight={height} onHeight={setHeight} />
        : <div aria-hidden="true" style={{ height }} />}
      {expanded && (
        <Dialog open onOpenChange={next => { if (!next) setExpanded(false); }}>
          <DialogPopup showCloseButton={false} aria-label={spec.title} className="flex h-[84dvh] max-w-[calc(100vw-4rem)] flex-col overflow-hidden">
            {header(true)}
            <McpAppFrame input={input} theme={theme} actions={actions} title={spec.title} displayMode="fullscreen" className="min-h-0 flex-1" />
          </DialogPopup>
        </Dialog>
      )}
    </div>
  );
}

export function VisualCall({ item, turnActive }: { item: ConversationItem; turnActive: boolean }) {
  const state = visualCallState(item);
  const input = visualCallInput(item);
  if (state === "refused") {
    return <QuietRow label="Visual not drawn: the tool sent its reasons back to the model" detail={visualCallRefusal(item)} />;
  }
  if (state === "drawing") {
    if (turnActive) return <Placeholder title={typeof input?.title === "string" ? input.title : undefined} />;
    return <QuietRow label="Visual not finished: the turn ended before it was drawn" />;
  }
  const parsed = parseVisualSpec(input);
  if (!input || !parsed.ok) {
    const first = parsed.ok ? undefined : parsed.errors[0];
    return <QuietRow label="Visual could not be drawn" detail={first ? `${first.path || "spec"}: ${first.message}` : undefined} />;
  }
  return <VisualCard spec={parsed.spec} input={input} />;
}
