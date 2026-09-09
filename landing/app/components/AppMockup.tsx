import { Fragment } from "react";
import type { Card, CardBody, ConversationItem, Scene, SidebarRow, Tone } from "../content/scenes";

const dotTone: Record<Tone, string> = {
  success: "bg-success",
  warning: "bg-warning",
  destructive: "bg-destructive",
  faint: "bg-faint",
  fg: "bg-foreground",
};

const inkTone: Record<Tone, string> = {
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

function Dot({ tone }: { tone: Tone }) {
  return <span className={`inline-block size-1.5 shrink-0 rounded-full ${dotTone[tone]}`} />;
}

function Row({ label, sub, tone, right, depth = 0, active = false }: SidebarRow) {
  return (
    <div className={`flex items-center gap-2 rounded-md py-1.5 pr-2 ${depth === 1 ? "pl-6" : "pl-2"} ${active ? "bg-muted" : ""}`}>
      <Dot tone={tone} />
      <div className="min-w-0 flex-1">
        <div className="truncate text-[12px] leading-4 text-foreground">{label}</div>
        {sub && <div className="truncate text-[11px] leading-4 text-muted-foreground">{sub}</div>}
      </div>
      {right && <span className="text-[10px] tabular-nums text-faint">{right}</span>}
    </div>
  );
}

function Message({ who, meta, text }: Extract<ConversationItem, { kind: "message" }>) {
  return (
    <div className={`flex flex-col gap-1 ${who === "you" ? "items-end" : "items-start"}`}>
      {meta && <span className="text-[10px] uppercase tracking-wider text-faint">{meta}</span>}
      <div className={`max-w-[85%] rounded-xl px-3 py-2 text-[12.5px] leading-5 ${who === "you" ? "bg-muted text-foreground" : "text-body"}`}>
        {text}
      </div>
    </div>
  );
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

function MockCard({ card }: { card: Card }) {
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

export default function AppMockup({ scene }: { scene: Scene }) {
  return (
    <div className="w-full overflow-hidden rounded-xl border border-border-card bg-background text-left text-foreground shadow-[0_0_0_1px_#000,0_40px_80px_-30px_rgba(0,0,0,0.9)]">
      <div className="flex h-10 items-center border-b border-border bg-sidebar px-3">
        <div className="flex gap-1.5" aria-hidden="true">
          <span className="size-3 rounded-full bg-[#ff5f57]" />
          <span className="size-3 rounded-full bg-[#febc2e]" />
          <span className="size-3 rounded-full bg-[#28c840]" />
        </div>
        <span className="ml-4 text-[12px] text-muted-foreground">Bridge</span>
        <div className="ml-auto flex min-w-0 items-center gap-2 overflow-hidden whitespace-nowrap text-[11px] text-muted-foreground">
          {scene.chips.map((chip, i) => (
            <span key={chip} className={`shrink-0 rounded-md border border-border px-2 py-0.5 ${i > 0 ? "max-sm:hidden" : ""}`}>
              {chip}
            </span>
          ))}
        </div>
      </div>

      <div className="grid h-[640px] grid-cols-[230px_1fr_320px] max-lg:grid-cols-[230px_1fr] max-md:grid-cols-1">
        <aside className="flex flex-col border-r border-border bg-sidebar p-2 max-md:hidden">
          <div className="px-2 pb-2 pt-1 text-[10px] uppercase tracking-wider text-faint">Repositories</div>
          <Row label="harness" sub="github.com/bridge/harness" tone="fg" />
          <div className="mt-2 px-2 pb-1 text-[10px] uppercase tracking-wider text-faint">Tasks</div>
          {scene.sidebar.map((row) => (
            <Row key={`${row.label}-${row.sub}`} {...row} />
          ))}
          <div className="mt-auto flex items-center gap-2 border-t border-border px-2 pt-2 text-[11px] text-muted-foreground">
            <Dot tone="success" /> codex · claude · opencode
          </div>
        </aside>

        <section className="flex min-h-0 min-w-0 flex-col">
          <div className="flex h-9 items-center gap-4 overflow-hidden whitespace-nowrap border-b border-border px-4 text-[12px]">
            <span className="shrink-0 border-b border-foreground pb-2 pt-2 text-foreground">Conversation</span>
            <span className="shrink-0 text-muted-foreground">
              Changes <span className="text-faint">{scene.changes}</span>
            </span>
            <span className="shrink-0 text-muted-foreground max-sm:hidden">Terminal</span>
            <span className="shrink-0 text-muted-foreground max-sm:hidden">Browser</span>
            <span className="ml-auto shrink-0 text-[11px] text-faint max-sm:hidden">{scene.model}</span>
          </div>
          <div className="relative min-h-0 flex-1 overflow-hidden">
            <div className="flex flex-col gap-3 px-5 py-4 animate-fade-up motion-reduce:animate-none">
              {scene.conversation.map((item, i) =>
                item.kind === "message" ? <Message key={i} {...item} /> : <MockCard key={i} card={item.card} />,
              )}
            </div>
            <div aria-hidden="true" className="pointer-events-none absolute inset-x-0 bottom-0 h-8 bg-linear-to-t from-background to-transparent" />
          </div>
          <div className="border-t border-border p-3">
            <div className="flex items-center gap-2 rounded-full border border-border-card bg-card px-3 py-2 text-[12px] text-faint">
              <span className="flex-1">Message Bridge…</span>
              <span className="rounded-full bg-foreground px-2 py-0.5 text-[10px] text-background">⏎</span>
            </div>
          </div>
        </section>

        <aside className="flex flex-col border-l border-border bg-sidebar max-lg:hidden">
          <div className="flex h-9 items-center border-b border-border px-3 text-[12px] text-foreground">Work</div>
          <div className="flex flex-col gap-3 overflow-hidden p-3 animate-fade-up motion-reduce:animate-none">
            {scene.work.map((card) => (
              <MockCard key={card.title} card={card} />
            ))}
          </div>
        </aside>
      </div>
    </div>
  );
}
