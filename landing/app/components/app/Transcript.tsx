import { Check, ChevronDown, Circle, CircleCheck, FileText, Pencil, SquareTerminal } from "lucide-react";
import HarnessMark from "./HarnessMark";
import { harnessLabel, type Entry, type Tone } from "../../content/appScenes";

const edge: Record<Tone, string> = {
  success: "border-l-success",
  warning: "border-l-warning",
  info: "border-l-info",
  destructive: "border-l-destructive",
  faint: "border-l-border",
};

const ink: Record<Tone, string> = {
  success: "text-success",
  warning: "text-warning",
  info: "text-info",
  destructive: "text-destructive",
  faint: "text-muted-foreground",
};

const dot: Record<Tone, string> = {
  success: "bg-success",
  warning: "bg-warning",
  info: "bg-info",
  destructive: "bg-destructive",
  faint: "bg-faint",
};

function Code({ children }: { children: string }) {
  return (
    <code className="mt-1 block max-w-full overflow-hidden whitespace-pre rounded-md border border-border bg-code px-2.5 py-2 font-mono text-[11.5px] leading-relaxed text-foreground">
      {children}
    </code>
  );
}

/** The app tints whole lines rather than inline spans, and keeps the gutter monospaced. */
function Diff({ text }: { text: string }) {
  return (
    <div className="mt-2 overflow-hidden rounded-md border border-border bg-code font-mono text-[11px] leading-[1.7]">
      {text.split("\n").map((line, i) => {
        const added = line.startsWith("+");
        const removed = line.startsWith("-");
        return (
          <div
            key={i}
            className={`flex gap-2 px-2.5 ${added ? "bg-success/10 text-foreground" : removed ? "bg-destructive/10 text-foreground" : "text-muted-foreground"}`}
          >
            <span className={`w-3 shrink-0 select-none ${added ? "text-success" : removed ? "text-destructive" : "text-faint-2"}`}>
              {added ? "+" : removed ? "−" : ""}
            </span>
            <span className="min-w-0 truncate">{line.replace(/^[+-]\s?/, "")}</span>
          </div>
        );
      })}
    </div>
  );
}

export default function TranscriptEntry({ entry }: { entry: Entry }) {
  switch (entry.kind) {
    case "user":
      return (
        <div className="ml-auto w-fit max-w-[85%] rounded-2xl border border-border bg-accent/70 px-4 py-2.5 text-[13.5px] leading-6 tracking-[-0.006em] text-foreground">
          {entry.text}
        </div>
      );

    case "assistant":
      return <p className="w-full min-w-0 text-[13.5px] leading-6 text-body">{entry.text}</p>;

    case "collapsed":
      return (
        <div className="-ml-2 flex min-h-[30px] w-full items-center gap-[9px] rounded-md px-2 py-1 text-[12.5px] text-muted-foreground">
          <span className="size-1 shrink-0 rounded-full bg-faint-2" aria-hidden="true" />
          <span className="min-w-0 flex-1 truncate">{entry.label}</span>
          <ChevronDown size={12} className="shrink-0 -rotate-90 text-faint" aria-hidden="true" />
        </div>
      );

    case "rail":
      return (
        <div className="min-w-0 border-l-2 border-border py-1 pl-3">
          <header className="flex items-center gap-2 text-ui">
            <b className="min-w-0 truncate font-medium text-foreground">{entry.label}</b>
            {entry.status && <small className="ml-auto shrink-0 text-caption text-muted-foreground">{entry.status}</small>}
          </header>
          <p className="mt-1 text-ui text-muted-foreground">{entry.text}</p>
        </div>
      );

    case "worker":
      return (
        <div className="flex min-w-0 items-center gap-3 rounded-lg border border-border-card bg-card px-3.5 py-2.5">
          <HarnessMark harness={entry.harness} size={15} />
          <span className="min-w-0 flex-1">
            <span className="flex flex-wrap items-baseline gap-x-2">
              <b className="text-ui font-medium text-foreground">{entry.label}</b>
              <small className="text-[11px] text-faint">{harnessLabel[entry.harness]}</small>
            </span>
            <span className="mt-0.5 block truncate text-[12px] text-muted-foreground">{entry.text}</span>
          </span>
          <span className={`flex shrink-0 items-center gap-1.5 text-[11px] uppercase tracking-[0.04em] ${ink[entry.tone]}`}>
            <span className={`size-1.5 rounded-full ${dot[entry.tone]} ${entry.tone === "success" ? "motion-safe:animate-pulse" : ""}`} aria-hidden="true" />
            {entry.status}
          </span>
        </div>
      );

    case "activity":
      return (
        <div className="min-w-0 overflow-hidden rounded-lg border border-border bg-card">
          <header className="flex items-center gap-2 px-3.5 py-2.5">
            <CircleCheck size={13} className="shrink-0 text-muted-foreground" aria-hidden="true" />
            <span className="min-w-0 flex-1">
              <b className="block text-ui font-medium text-foreground">Activity</b>
              <span className="block truncate text-[12px] text-muted-foreground">{entry.summary}</span>
            </span>
            <small className="shrink-0 text-[11px] tabular-nums text-muted-foreground">{entry.steps}</small>
          </header>
          <div className="border-t border-border">
            {entry.rows.map((row, i) => (
              <div key={i} className="flex items-center gap-2 border-b border-border px-3.5 py-2 last:border-b-0">
                {row.label === "Ran" ? <SquareTerminal size={12} className="shrink-0 text-faint" aria-hidden="true" /> : row.label === "Edited" ? <Pencil size={12} className="shrink-0 text-faint" aria-hidden="true" /> : <FileText size={12} className="shrink-0 text-faint" aria-hidden="true" />}
                <span className="text-[12px] text-foreground">{row.label}</span>
                <span className="min-w-0 flex-1 truncate font-mono text-[11.5px] text-muted-foreground">{row.path}</span>
                {row.stat && <small className="shrink-0 font-mono text-[11px] tabular-nums text-muted-foreground">{row.stat}</small>}
              </div>
            ))}
          </div>
          {entry.diff && <div className="px-3.5 pb-3">{<Diff text={entry.diff} />}</div>}
        </div>
      );

    case "checks":
      return (
        <div className="min-w-0 overflow-hidden rounded-lg border border-l-2 border-border border-l-info bg-card">
          <header className="flex flex-wrap items-center gap-x-2 gap-y-1 px-4 pt-3">
            <ChevronDown size={13} className="shrink-0 text-muted-foreground" aria-hidden="true" />
            <b className="text-ui font-semibold text-foreground">{entry.title}</b>
            <small className="text-[11px] text-muted-foreground">{entry.status}</small>
            <small className="ml-auto text-[11px] text-muted-foreground">Proof and checks</small>
          </header>
          <p className="mt-1.5 px-4 font-mono text-[11px] text-muted-foreground">{entry.revision}</p>
          <div className="mt-2 flex flex-col gap-1.5 px-4 pb-3.5">
            {entry.checks.map(check => (
              <div key={check.name} className="flex items-center gap-2 text-[12px]">
                {check.state === "passed" ? (
                  <Check size={12} className="shrink-0 text-success" aria-hidden="true" />
                ) : check.state === "running" ? (
                  <Circle size={11} className="shrink-0 animate-spin text-warning [stroke-dasharray:20] motion-reduce:animate-none" aria-hidden="true" />
                ) : (
                  <Circle size={11} className="shrink-0 text-faint-2" aria-hidden="true" />
                )}
                <span className="font-mono text-[11.5px] text-foreground">{check.name}</span>
                <span className="text-[11px] text-muted-foreground">{check.kind}</span>
                {check.family && <span className="text-[11px] text-faint">· {harnessLabel[check.family]}</span>}
                {check.detail && <span className="truncate font-mono text-[11px] text-faint">{check.detail}</span>}
                <span className={`ml-auto shrink-0 text-[11px] ${check.state === "passed" ? "text-success" : check.state === "running" ? "text-warning" : "text-faint"}`}>
                  {check.state === "passed" ? "Passed" : check.state === "running" ? "Running" : "Pending"}
                </span>
              </div>
            ))}
          </div>
        </div>
      );

    case "notice":
      return (
        <div className={`min-w-0 overflow-hidden rounded-lg border border-l-2 border-border bg-card ${edge[entry.edge]}`}>
          <header className="flex flex-wrap items-baseline gap-x-2 gap-y-1 px-4 pt-3">
            <b className="text-ui font-semibold text-foreground">{entry.title}</b>
            {entry.status && <small className={`text-[11px] tracking-[0.03em] ${ink[entry.edge]}`}>{entry.status}</small>}
          </header>
          <p className="mt-1 px-4 text-ui leading-relaxed text-muted-foreground">{entry.text}</p>
          {(entry.caption || entry.code) && (
            <div className="mt-2 px-4">
              {entry.caption && <small className="block text-[11px] uppercase tracking-[0.03em] text-muted-foreground">{entry.caption}</small>}
              {entry.code && <Code>{entry.code}</Code>}
            </div>
          )}
          {entry.actions && (
            <div className="flex flex-wrap items-center gap-2 px-4 pb-3 pt-3">
              {entry.actions.map((action, i) => (
                <span
                  key={action}
                  className={`inline-flex h-7 items-center rounded-md px-2.5 text-[12px] ${
                    i === entry.actions!.length - 1 ? "bg-primary text-primary-foreground" : "border border-border text-muted-foreground"
                  }`}
                >
                  {action}
                </span>
              ))}
            </div>
          )}
          {!entry.actions && <div className="pb-3" />}
        </div>
      );
  }
}
