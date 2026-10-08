/**
 * The document family: metric, cards, compare, table, callout, steps,
 * checklist, findings, pros-cons, glossary. Text renders as text nodes only;
 * nothing a model writes is ever parsed as markup.
 */

import { AlertTriangle, Check, CheckCircle2, Circle, Info, Minus, Plus, XCircle } from "lucide-react";
import type { ReactNode } from "react";
import { cn } from "@/lib/utils";
import type { DocumentBlock } from "../spec";
import { formatCell } from "./format";

type Item = Record<string, unknown>;

const text = (value: unknown) => (typeof value === "string" ? value : typeof value === "number" ? String(value) : "");
const list = (value: unknown): Item[] => (Array.isArray(value) ? value.filter((item): item is Item => typeof item === "object" && item !== null) : []);
const strings = (value: unknown): string[] => (Array.isArray(value) ? value.filter((item): item is string => typeof item === "string") : []);

export function Estimate() {
  return <span className="ml-1.5 rounded-md border border-dashed border-border px-1 text-[10.5px] font-normal tracking-normal text-muted-foreground">est.</span>;
}

/** Citation marks for a list of source ids, numbered by declaration order. */
export function Cites({ ids, numbers }: { ids: unknown; numbers: Map<string, number> }) {
  const known = strings(ids).filter(id => numbers.has(id));
  if (known.length === 0) return null;
  return (
    <span className="ml-1.5 inline-flex gap-1 align-[1px]">
      {known.map(id => (
        <span key={id} className="inline-flex h-[17px] min-w-[17px] items-center justify-center rounded-[5px] border border-border px-1 text-[10.5px] tabular-nums text-muted-foreground">{numbers.get(id)}</span>
      ))}
    </span>
  );
}

function Metric({ content }: { content: Item }) {
  return (
    <div className="grid grid-cols-[repeat(auto-fit,minmax(150px,1fr))] gap-x-8 gap-y-4">
      {list(content.items).map((item, at) => {
        const tone = item.tone === "positive" ? "text-success" : item.tone === "negative" ? "text-destructive" : "text-muted-foreground";
        return (
          <div key={at} className="min-w-0">
            <div className="text-[11px] font-medium uppercase tracking-[0.08em] text-muted-foreground">{text(item.label)}</div>
            <div className="mt-1.5 flex items-baseline text-[32px] font-light leading-none tracking-[-0.03em] text-foreground tabular-nums">
              {text(item.value)}
              {item.estimate === true && <Estimate />}
            </div>
            {(item.delta !== undefined || item.note !== undefined) && (
              <div className="mt-1.5 text-[12px] text-muted-foreground">
                {item.delta !== undefined && <span className={cn("mr-1.5 font-medium tabular-nums", tone)}>{text(item.delta)}</span>}
                {text(item.note)}
              </div>
            )}
          </div>
        );
      })}
    </div>
  );
}

function Cards({ content }: { content: Item }) {
  return (
    <div className="grid grid-cols-[repeat(auto-fill,minmax(200px,1fr))] gap-3">
      {list(content.items).map((item, at) => (
        <div key={at} className="min-w-0 rounded-xl border border-border p-3.5">
          <div className="text-[13.5px] font-medium leading-snug text-foreground">{text(item.title)}</div>
          {item.body !== undefined && <p className="mt-1 text-[12.5px] leading-relaxed text-muted-foreground">{text(item.body)}</p>}
          {(item.meta !== undefined || strings(item.tags).length > 0) && (
            <div className="mt-2.5 flex flex-wrap items-center gap-1.5 text-[11.5px] text-muted-foreground">
              {item.meta !== undefined && <span className="mr-1">{text(item.meta)}</span>}
              {strings(item.tags).map(tag => <span key={tag} className="rounded-full border border-border px-2 py-px font-mono text-[10.5px]">{tag}</span>)}
            </div>
          )}
        </div>
      ))}
    </div>
  );
}

function CompareValue({ value }: { value: unknown }) {
  if (value === true) return <Check size={14} className="text-success" aria-label="yes" />;
  if (value === false) return <Minus size={14} className="text-muted-foreground" aria-label="no" />;
  return <span className="tabular-nums">{formatCell(value)}</span>;
}

const COMPARE_COLUMNS: Record<number, string> = { 2: "sm:grid-cols-2", 3: "sm:grid-cols-3", 4: "sm:grid-cols-4", 5: "sm:grid-cols-5" };

function Compare({ content }: { content: Item }) {
  const criteria = strings(content.criteria);
  const options = list(content.options);
  return (
    <div className={cn("grid gap-3", COMPARE_COLUMNS[options.length] ?? "sm:grid-cols-2")}>
      {options.map((option, at) => {
        const values = (typeof option.values === "object" && option.values !== null ? option.values : {}) as Item;
        return (
          <div key={at} className={cn("min-w-0 rounded-xl border p-3.5", option.highlight === true ? "border-ring ring-1 ring-ring/40" : "border-border")}>
            <div className="text-[13.5px] font-semibold text-foreground">{text(option.name)}</div>
            {option.summary !== undefined && <p className="mt-1 text-[12px] leading-snug text-muted-foreground">{text(option.summary)}</p>}
            <dl className="mt-3 grid gap-1.5 text-[12.5px]">
              {criteria.map(criterion => (
                <div key={criterion} className="flex items-center justify-between gap-3 border-t border-border pt-1.5 first:border-t-0 first:pt-0">
                  <dt className="truncate text-muted-foreground">{criterion}</dt>
                  <dd className="shrink-0 text-foreground"><CompareValue value={values[criterion]} /></dd>
                </div>
              ))}
            </dl>
            {option.highlight === true && <div className="mt-3 text-[11.5px] text-ring">Best fit</div>}
          </div>
        );
      })}
    </div>
  );
}

function Table({ content }: { content: Item }) {
  const columns = list(content.columns);
  const rows = list(content.rows);
  const align = (column: Item) => column.align === "right" || (column.align === undefined && ["number", "percent", "currency", "compact"].includes(text(column.format))) ? "text-right" : column.align === "center" ? "text-center" : "text-left";
  return (
    <div className="overflow-x-auto">
      <table className="w-full border-collapse text-[12.5px]">
        <thead>
          <tr>
            {columns.map(column => <th key={text(column.key)} className={cn("border-b border-border pb-2 text-[11px] font-medium uppercase tracking-[0.06em] text-muted-foreground", align(column))}>{text(column.label)}</th>)}
          </tr>
        </thead>
        <tbody>
          {rows.map((row, at) => (
            <tr key={at} className="border-b border-border last:border-b-0">
              {columns.map((column, columnAt) => (
                <td key={text(column.key)} className={cn("py-2 text-foreground tabular-nums", align(column), columnAt === 0 && "pr-4")}>
                  {formatCell(row[text(column.key)], text(column.format) || undefined)}
                  {columnAt === columns.length - 1 && row.estimate === true && <Estimate />}
                </td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function Callout({ content }: { content: Item }) {
  const tone = text(content.tone);
  const icon = tone === "warning" ? <AlertTriangle size={15} className="text-warning" /> : tone === "danger" ? <XCircle size={15} className="text-destructive" /> : tone === "success" ? <CheckCircle2 size={15} className="text-success" /> : <Info size={15} className="text-muted-foreground" />;
  return (
    <div className="flex gap-3 rounded-xl border border-border bg-muted/50 p-3.5">
      <span className="mt-0.5 shrink-0" aria-hidden="true">{icon}</span>
      <div className="min-w-0">
        {content.title !== undefined && <div className="text-[13.5px] font-medium text-foreground">{text(content.title)}</div>}
        <p className="text-[12.5px] leading-relaxed text-muted-foreground">{text(content.body)}</p>
      </div>
    </div>
  );
}

function Steps({ content }: { content: Item }) {
  const items = list(content.items);
  return (
    <ol className="grid gap-0">
      {items.map((item, at) => (
        <li key={at} className="relative flex gap-3 pb-4 last:pb-0">
          {at < items.length - 1 && <span className="absolute left-[11px] top-6 bottom-0 w-px bg-border" aria-hidden="true" />}
          <span className="flex size-6 shrink-0 items-center justify-center rounded-full border border-border text-[11.5px] font-medium tabular-nums text-foreground">{at + 1}</span>
          <div className="min-w-0 pt-0.5">
            <div className="text-[13.5px] font-medium text-foreground">{text(item.title)}</div>
            {item.body !== undefined && <p className="mt-0.5 text-[12.5px] leading-relaxed text-muted-foreground">{text(item.body)}</p>}
          </div>
        </li>
      ))}
    </ol>
  );
}

function Checklist({ content }: { content: Item }) {
  const items = list(content.items);
  const done = items.filter(item => item.done === true).length;
  return (
    <div>
      <div className="mb-2 text-[12px] text-muted-foreground tabular-nums">{done} of {items.length} done</div>
      <ul className="grid gap-1.5">
        {items.map((item, at) => (
          <li key={at} className="flex items-start gap-2.5 text-[13px]">
            {item.done === true ? <CheckCircle2 size={16} className="mt-px shrink-0 text-success" aria-label="done" /> : <Circle size={16} className="mt-px shrink-0 text-muted-foreground" aria-label="not done" />}
            <span className={item.done === true ? "text-muted-foreground" : "text-foreground"}>{text(item.text)}</span>
          </li>
        ))}
      </ul>
    </div>
  );
}

function Findings({ content, numbers }: { content: Item; numbers: Map<string, number> }) {
  return (
    <ol className="grid gap-2">
      {list(content.items).map((item, at) => (
        <li key={at} className="flex gap-3 text-[13px] leading-relaxed">
          <span className="w-4 shrink-0 font-mono text-[12px] text-muted-foreground tabular-nums">{at + 1}</span>
          <span className="min-w-0 text-foreground">
            {text(item.text)}
            {item.estimate === true && <Estimate />}
            <Cites ids={item.sourceIds} numbers={numbers} />
          </span>
        </li>
      ))}
    </ol>
  );
}

function ProsCons({ content }: { content: Item }) {
  const side = (items: string[], positive: boolean) => (
    <ul className="grid content-start gap-1.5">
      {items.map((item, at) => (
        <li key={at} className="flex gap-2 text-[13px] leading-relaxed text-foreground">
          {positive ? <Plus size={14} className="mt-1 shrink-0 text-success" aria-label="pro" /> : <Minus size={14} className="mt-1 shrink-0 text-destructive" aria-label="con" />}
          {item}
        </li>
      ))}
    </ul>
  );
  return (
    <div className="grid gap-5 sm:grid-cols-2">
      <div><div className="mb-2 text-[11px] font-medium uppercase tracking-[0.08em] text-muted-foreground">For</div>{side(strings(content.pros), true)}</div>
      <div><div className="mb-2 text-[11px] font-medium uppercase tracking-[0.08em] text-muted-foreground">Against</div>{side(strings(content.cons), false)}</div>
    </div>
  );
}

function Glossary({ content }: { content: Item }) {
  return (
    <dl className="grid gap-2.5">
      {list(content.items).map((item, at) => (
        <div key={at} className="grid gap-0.5 sm:grid-cols-[140px_1fr] sm:gap-4">
          <dt className="text-[13px] font-medium text-foreground">{text(item.term)}</dt>
          <dd className="text-[13px] leading-relaxed text-muted-foreground">{text(item.definition)}</dd>
        </div>
      ))}
    </dl>
  );
}

export function DocumentView({ block, numbers }: { block: DocumentBlock; numbers: Map<string, number> }): ReactNode {
  const content = block.content as Item;
  switch (block.form) {
    case "metric": return <Metric content={content} />;
    case "cards": return <Cards content={content} />;
    case "compare": return <Compare content={content} />;
    case "table": return <Table content={content} />;
    case "callout": return <Callout content={content} />;
    case "steps": return <Steps content={content} />;
    case "checklist": return <Checklist content={content} />;
    case "findings": return <Findings content={content} numbers={numbers} />;
    case "pros-cons": return <ProsCons content={content} />;
    case "glossary": return <Glossary content={content} />;
    default: return null;
  }
}
