/**
 * A whole `VisualSpec`, drawn: the blocks in their layout, honesty notes,
 * numbered sources, and follow-up questions. Pure and host-agnostic; the
 * MCP Apps wiring lives in `main.tsx` and hands in the two actions a view may
 * take: ask a follow-up and open a source.
 */

import { ArrowUpRight, MessageCircle } from "lucide-react";
import { useMemo } from "react";
import { cn } from "@/lib/utils";
import { DiagramFigure } from "@/components/DiagramFigure";
import type { DiagramSpec } from "@/components/DiagramFigure";
import type { VisualBlock, VisualSource, VisualSpec } from "../spec";
import { ChartHeight, ChartView } from "./charts";
import { Cites, DocumentView } from "./documents";

export interface VisualActions {
  ask?: (text: string) => void;
  open?: (url: string) => void;
}

function sourceLabel(source: VisualSource): { name: string; detail?: string } {
  switch (source.kind) {
    case "web": {
      try {
        return { name: new URL(source.ref).hostname.replace(/^www\./, ""), detail: source.title };
      } catch {
        return { name: source.ref, detail: source.title };
      }
    }
    case "user": return { name: "You", detail: source.title ?? source.ref };
    case "file": return { name: source.ref.split("/").pop() || source.ref, detail: source.title };
    case "tool": return { name: source.title ?? "Tool output", detail: source.title ? undefined : source.ref };
    case "computed": return { name: "Computed", detail: source.ref };
    case "estimate": return { name: "Estimate", detail: source.ref };
  }
}

function Block({ block, numbers, citeBlocks }: { block: VisualBlock; numbers: Map<string, number>; citeBlocks: boolean }) {
  // With one block the footer's source list already says where it came from;
  // per-block marks earn their place only when blocks draw on different sources.
  // Marks sit on the block's title row, so an untitled block leaves them to the footer.
  const cited = !citeBlocks || !block.title ? undefined : block.family === "chart" ? block.sourceIds : block.family === "document" && block.form !== "findings" ? block.sourceIds : undefined;
  return (
    <section className="min-w-0" data-visual-block={`${block.family}/${block.form}`}>
      {(block.title || cited?.length) && (
        <div className="mb-3 flex items-baseline justify-between gap-3">
          {block.title ? <h3 className="text-[13px] font-medium text-foreground">{block.title}</h3> : <span />}
          <Cites ids={cited} numbers={numbers} />
        </div>
      )}
      {block.family === "chart" && <ChartView block={block} />}
      {block.family === "document" && <DocumentView block={block} numbers={numbers} />}
      {block.family === "diagram" && <div className="text-foreground"><DiagramFigure spec={block.graph as DiagramSpec} /></div>}
    </section>
  );
}

export function VisualView({ spec, actions = {}, showTitle = true, fullscreen = false }: { spec: VisualSpec; actions?: VisualActions; showTitle?: boolean; fullscreen?: boolean }) {
  const numbers = useMemo(() => new Map((spec.sources ?? []).map((source, at) => [source.id, at + 1])), [spec.sources]);
  const grid = spec.layout === "grid" && spec.blocks.length > 1;
  // Fullscreen gives a single chart most of the frame; several share it.
  const height = fullscreen && typeof window !== "undefined"
    ? Math.round(Math.min(640, Math.max(280, (window.innerHeight - 220) / Math.max(1, grid ? 1 : spec.blocks.length))))
    : 240;
  return (
    <ChartHeight.Provider value={height}>
    <article className="text-foreground" aria-label={spec.title}>
      <div className="px-5 pt-4 pb-4">
        {showTitle && (
          <header className="mb-4">
            <h2 className="text-[15px] font-semibold tracking-[-0.01em]">{spec.title}</h2>
            {spec.subtitle && <p className="mt-0.5 text-[12.5px] text-muted-foreground">{spec.subtitle}</p>}
          </header>
        )}
        <div className={cn("grid gap-6", grid && "sm:grid-cols-2")}>
          {spec.blocks.map((block, at) => <Block key={at} block={block} numbers={numbers} citeBlocks={spec.blocks.length > 1} />)}
        </div>
        {spec.notes && spec.notes.length > 0 && (
          <ul className="mt-4 grid gap-1 text-[12px] leading-relaxed text-muted-foreground">
            {spec.notes.map((note, at) => <li key={at}>{note}</li>)}
          </ul>
        )}
      </div>
      {((spec.sources?.length ?? 0) > 0 || (spec.followUps?.length ?? 0) > 0) && (
        <footer className="grid gap-3 border-t border-border px-5 py-3">
          {spec.sources && spec.sources.length > 0 && (
            <div className="flex flex-wrap gap-1.5" aria-label="Sources">
              {spec.sources.map((source, at) => {
                const { name, detail } = sourceLabel(source);
                const link = source.kind === "web" && actions.open ? () => actions.open!(source.ref) : undefined;
                const body = (
                  <>
                    <span className="inline-flex h-[17px] min-w-[17px] items-center justify-center rounded-[5px] border border-border px-1 text-[10.5px] tabular-nums text-muted-foreground">{at + 1}</span>
                    <span className="font-medium text-foreground">{name}</span>
                    {detail && <span className="truncate text-muted-foreground">{detail}</span>}
                    {source.kind === "estimate" && <span className="rounded border border-dashed border-border px-1 text-[10.5px] text-muted-foreground">est.</span>}
                    {link && <ArrowUpRight size={12} className="shrink-0 text-muted-foreground" aria-hidden="true" />}
                  </>
                );
                const chip = "inline-flex max-w-full items-center gap-1.5 rounded-full border border-border px-2.5 py-1 text-[12px]";
                return link
                  ? <button key={source.id} type="button" onClick={link} className={cn(chip, "transition-colors hover:bg-accent")} title={source.ref}>{body}</button>
                  : <span key={source.id} className={chip} title={source.ref}>{body}</span>;
              })}
            </div>
          )}
          {spec.followUps && spec.followUps.length > 0 && actions.ask && (
            <div className="flex flex-wrap gap-1.5" aria-label="Follow-up questions">
              {spec.followUps.map(question => (
                <button key={question} type="button" onClick={() => actions.ask!(question)} className="inline-flex items-center gap-1.5 rounded-full border border-border bg-background px-2.5 py-1 text-[12.5px] text-foreground transition-colors hover:bg-accent">
                  <MessageCircle size={12} className="text-muted-foreground" aria-hidden="true" />
                  {question}
                </button>
              ))}
            </div>
          )}
        </footer>
      )}
    </article>
    </ChartHeight.Provider>
  );
}
