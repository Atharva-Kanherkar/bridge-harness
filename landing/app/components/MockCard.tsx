import { Fragment } from "react";
import type { Card, CardBody, Tone } from "../content/scenes";

export const dotTone: Record<Tone, string> = {
  success: "bg-success",
  warning: "bg-warning",
  destructive: "bg-destructive",
  faint: "bg-faint",
  fg: "bg-foreground",
};

export const inkTone: Record<Tone, string> = {
  success: "text-success",
  warning: "text-warning",
  destructive: "text-destructive",
  faint: "text-faint",
  fg: "text-foreground",
};

const glyphTone: Record<Tone, string> = {
  success: "✓",
  warning: "●",
  destructive: "✕",
  faint: "○",
  fg: "·",
};

export function Dot({ tone }: { tone: Tone }) {
  return <span className={`inline-block size-1.5 shrink-0 rounded-full ${dotTone[tone]}`} />;
}

function diffTone(line: string) {
  if (line.startsWith("+")) return "text-success";
  if (line.startsWith("-")) return "text-destructive";
  return "text-muted-foreground";
}

function CardContent({ body }: { body: CardBody }) {
  switch (body.kind) {
    case "text":
      return <p className="text-[12px] leading-5 text-body">{body.text}</p>;
    case "grid":
      return (
        <div className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-0.5 font-mono text-[11px] text-muted-foreground">
          {body.rows.map(([key, value], i) => (
            <Fragment key={`${key}-${i}`}>
              <span>{key}</span>
              <span className="truncate text-foreground">{value}</span>
            </Fragment>
          ))}
        </div>
      );
    case "checks":
      return (
        <div className="flex flex-col gap-1 text-[11px] text-muted-foreground">
          {body.items.map((item) => (
            <span key={item.label} className="truncate">
              <span className={inkTone[item.tone]}>{glyphTone[item.tone]}</span> {item.label}
              {item.detail && <span className="text-faint"> · {item.detail}</span>}
            </span>
          ))}
        </div>
      );
    case "code":
      return (
        <pre className="overflow-hidden font-mono text-[10.5px] leading-4">
          {body.lines.map((line, i) => (
            <div key={i} className={diffTone(line)}>
              {line}
            </div>
          ))}
        </pre>
      );
  }
}

export default function MockCard({ card }: { card: Card }) {
  return (
    <div className="w-full rounded-lg border border-border-card bg-card">
      <div className="flex items-center justify-between gap-3 border-b border-border px-3 py-1.5 text-[11px] text-muted-foreground">
        <span className="truncate">{card.title}</span>
        {card.status && <span className={`shrink-0 ${inkTone[card.status.tone]}`}>{card.status.label}</span>}
      </div>
      <div className="px-3 py-2">
        <CardContent body={card.body} />
      </div>
    </div>
  );
}
