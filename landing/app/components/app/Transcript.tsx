import { ArrowLeftRight, Check, ChevronDown, Circle, CircleCheck, FileText, LoaderCircle, Pencil, SquareTerminal } from "lucide-react";
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

/*
 * Each entry plays itself from `p` 0 to 1: text streams in, tool rows spin and then settle,
 * diff lines land one by one, checks move from pending to running to passed. At `p = 1` the
 * entry is exactly the finished state the scene data describes, which is what the server
 * renders. `entryTicks` is how long the player gives each kind to play out.
 */
export function entryTicks(entry: Entry) {
  switch (entry.kind) {
    case "user":
      return 4;
    case "assistant":
      return Math.ceil(entry.text.length / 4);
    case "collapsed":
      return 10;
    case "rail":
      return 14;
    case "switch":
      return 12;
    case "worker":
      return 16;
    case "notice":
      return 22 + (entry.code ? 10 : 0) + (entry.actions ? 6 : 0);
    case "activity":
      return entry.rows.length * 11 + (entry.diff ? entry.diff.split("\n").length * 3 : 0) + 4;
    case "checks":
      return entry.checks.length * 12 + 6;
  }
}

const clamp = (v: number) => Math.min(1, Math.max(0, v));
/** Progress through the slice [from, to] of the entry's own 0..1. */
const span = (p: number, from: number, to: number) => clamp((p - from) / (to - from));
const stream = (text: string, p: number) => (p >= 1 ? text : text.slice(0, Math.ceil(text.length * p)));

function Caret() {
  return <span className="ml-px inline-block h-[1.05em] w-px translate-y-[0.18em] bg-foreground animate-caret motion-reduce:animate-none" aria-hidden="true" />;
}

function Spinner({ size = 12 }: { size?: number }) {
  return <LoaderCircle size={size} className="shrink-0 animate-spin text-muted-foreground motion-reduce:animate-none" aria-hidden="true" />;
}

function Code({ children }: { children: string }) {
  return (
    <code className="mt-1 block min-h-[2.4em] max-w-full overflow-hidden whitespace-pre rounded-md border border-border bg-code px-2.5 py-2 font-mono text-[11.5px] leading-relaxed text-foreground">
      {children}
    </code>
  );
}

/** The app tints whole lines rather than inline spans, and keeps the gutter monospaced. */
function Diff({ text, shown }: { text: string; shown: number }) {
  return (
    <div className="mt-2 overflow-hidden rounded-md border border-border bg-code font-mono text-[11px] leading-[1.7]">
      {text
        .split("\n")
        .slice(0, shown)
        .map((line, i) => {
          const added = line.startsWith("+");
          const removed = line.startsWith("-");
          return (
            <div
              key={i}
              className={`flex gap-2 px-2.5 animate-entry-in motion-reduce:animate-none ${added ? "bg-success/10 text-foreground" : removed ? "bg-destructive/10 text-foreground" : "text-muted-foreground"}`}
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

export default function TranscriptEntry({ entry, p = 1 }: { entry: Entry; p?: number }) {
  const live = p < 1;

  switch (entry.kind) {
    case "user":
      return (
        <div className="ml-auto w-fit max-w-[85%] rounded-2xl border border-border bg-accent/70 px-4 py-2.5 text-[13.5px] leading-6 tracking-[-0.006em] text-foreground">
          {entry.text}
        </div>
      );

    case "assistant":
      return (
        <p className="w-full min-w-0 text-[13.5px] leading-6 text-body">
          {stream(entry.text, p)}
          {live && <Caret />}
        </p>
      );

    case "collapsed":
      return (
        <div className="-ml-2 flex min-h-[30px] w-full items-center gap-[9px] rounded-md px-2 py-1 text-[12.5px] text-muted-foreground">
          {live ? <Spinner size={11} /> : <span className="size-1 shrink-0 rounded-full bg-faint-2" aria-hidden="true" />}
          <span className="min-w-0 flex-1 truncate">{entry.label}</span>
          <ChevronDown size={12} className="shrink-0 -rotate-90 text-faint" aria-hidden="true" />
        </div>
      );

    case "rail":
      return (
        <div className={`min-w-0 border-l-2 py-1 pl-3 transition-colors duration-500 ${live ? "border-info" : "border-border"}`}>
          <header className="flex items-center gap-2 text-ui">
            <b className="min-w-0 truncate font-medium text-foreground">{entry.label}</b>
            {entry.status && (
              <small className="ml-auto flex shrink-0 items-center gap-1 text-caption text-muted-foreground">
                {live && <Spinner size={10} />}
                {entry.status}
              </small>
            )}
          </header>
          <p className="mt-1 text-ui text-muted-foreground">{stream(entry.text, span(p, 0.1, 0.9))}</p>
        </div>
      );

    case "switch":
      return (
        <div className="flex items-center gap-2 text-[11.5px] text-muted-foreground">
          <span className="h-px flex-1 bg-border" aria-hidden="true" />
          <span className="flex items-center gap-1.5">
            <HarnessMark harness={entry.from} size={12} />
            <ArrowLeftRight size={11} className={live ? "animate-pulse text-foreground" : "text-faint"} aria-hidden="true" />
            <HarnessMark harness={entry.to} size={12} />
            <span className="text-foreground">{live ? "Restarting the provider session…" : `Switched to ${entry.model}. History kept.`}</span>
          </span>
          <span className="h-px flex-1 bg-border" aria-hidden="true" />
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
            <span className="mt-0.5 block truncate text-[12px] text-muted-foreground">{stream(entry.text, p)}</span>
          </span>
          <span className={`flex shrink-0 items-center gap-1.5 text-[11px] uppercase tracking-[0.04em] ${ink[entry.tone]}`}>
            <span className={`size-1.5 rounded-full ${dot[entry.tone]} ${entry.tone === "success" ? "motion-safe:animate-pulse" : ""}`} aria-hidden="true" />
            {entry.status}
          </span>
        </div>
      );

    case "activity": {
      const diffLines = entry.diff ? entry.diff.split("\n").length : 0;
      const rowsEnd = entry.rows.length * 11 / entryTicks(entry);
      const rowsShown = Math.min(entry.rows.length, Math.floor(span(p, 0, rowsEnd) * entry.rows.length) + 1);
      const rowsDone = p >= rowsEnd ? entry.rows.length : rowsShown - 1;
      const diffShown = entry.diff ? Math.ceil(span(p, rowsEnd, 0.97) * diffLines) : 0;
      return (
        <div className="min-w-0 overflow-hidden rounded-lg border border-border bg-card">
          <header className="flex items-center gap-2 px-3.5 py-2.5">
            {rowsDone < entry.rows.length ? <Spinner size={13} /> : <CircleCheck size={13} className="shrink-0 text-muted-foreground" aria-hidden="true" />}
            <span className="min-w-0 flex-1">
              <b className="block text-ui font-medium text-foreground">Activity</b>
              <span className="block truncate text-[12px] text-muted-foreground">{rowsDone < entry.rows.length ? `${entry.rows[rowsShown - 1].label} ${entry.rows[rowsShown - 1].path ?? ""}…` : entry.summary}</span>
            </span>
            <small className="shrink-0 text-[11px] tabular-nums text-muted-foreground">{rowsDone < entry.rows.length ? `${rowsDone} of ${entry.rows.length} steps` : entry.steps}</small>
          </header>
          <div className="border-t border-border">
            {entry.rows.slice(0, rowsShown).map((row, i) => (
              <div key={i} className="flex items-center gap-2 border-b border-border px-3.5 py-2 animate-entry-in last:border-b-0 motion-reduce:animate-none">
                {row.label === "Ran" ? <SquareTerminal size={12} className="shrink-0 text-faint" aria-hidden="true" /> : row.label === "Edited" ? <Pencil size={12} className="shrink-0 text-faint" aria-hidden="true" /> : <FileText size={12} className="shrink-0 text-faint" aria-hidden="true" />}
                <span className="text-[12px] text-foreground">{row.label}</span>
                <span className="min-w-0 flex-1 truncate font-mono text-[11.5px] text-muted-foreground">{row.path}</span>
                {i < rowsDone ? row.stat && <small className="shrink-0 font-mono text-[11px] tabular-nums text-muted-foreground">{row.stat}</small> : <Spinner size={11} />}
              </div>
            ))}
          </div>
          {entry.diff && diffShown > 0 && <div className="px-3.5 pb-3">{<Diff text={entry.diff} shown={diffShown} />}</div>}
        </div>
      );
    }

    case "checks": {
      // Each check runs in turn and settles to the state the data gives it.
      const states = entry.checks.map((check, i) => {
        const at = span(p, 0, 0.95) * entry.checks.length;
        if (check.state === "pending" || at < i) return "pending";
        if (check.state === "running" || at < i + 0.75) return "running";
        return "passed";
      });
      const passed = states.filter(state => state === "passed").length;
      return (
        <div className="min-w-0 overflow-hidden rounded-lg border border-l-2 border-border border-l-info bg-card">
          <header className="flex flex-wrap items-center gap-x-2 gap-y-1 px-4 pt-3">
            <ChevronDown size={13} className="shrink-0 text-muted-foreground" aria-hidden="true" />
            <b className="text-ui font-semibold text-foreground">{entry.title}</b>
            <small className="text-[11px] tabular-nums text-muted-foreground">{live ? `${passed} of ${entry.checks.length} required checks passed` : entry.status}</small>
            <small className="ml-auto text-[11px] text-muted-foreground">Proof and checks</small>
          </header>
          <p className="mt-1.5 px-4 font-mono text-[11px] text-muted-foreground">{entry.revision}</p>
          <div className="mt-2 flex flex-col gap-1.5 px-4 pb-3.5">
            {entry.checks.map((check, i) => {
              const state = states[i];
              return (
                <div key={check.name} className="flex items-center gap-2 text-[12px]">
                  {state === "passed" ? (
                    <Check size={12} className="shrink-0 text-success" aria-hidden="true" />
                  ) : state === "running" ? (
                    <Circle size={11} className="shrink-0 animate-spin text-warning [stroke-dasharray:20] motion-reduce:animate-none" aria-hidden="true" />
                  ) : (
                    <Circle size={11} className="shrink-0 text-faint-2" aria-hidden="true" />
                  )}
                  <span className="font-mono text-[11.5px] text-foreground">{check.name}</span>
                  <span className="text-[11px] text-muted-foreground">{check.kind}</span>
                  {check.family && <span className="text-[11px] text-faint">· {harnessLabel[check.family]}</span>}
                  {check.detail && state === "passed" && <span className="truncate font-mono text-[11px] text-faint">{check.detail}</span>}
                  <span className={`ml-auto shrink-0 text-[11px] ${state === "passed" ? "text-success" : state === "running" ? "text-warning" : "text-faint"}`}>
                    {state === "passed" ? "Passed" : state === "running" ? "Running" : "Pending"}
                  </span>
                </div>
              );
            })}
          </div>
        </div>
      );
    }

    case "notice": {
      const codeAt = entry.code ? 0.35 : 1;
      const actionsAt = entry.actions ? 0.82 : 1;
      return (
        <div className={`min-w-0 overflow-hidden rounded-lg border border-l-2 border-border bg-card ${edge[entry.edge]}`}>
          <header className="flex flex-wrap items-baseline gap-x-2 gap-y-1 px-4 pt-3">
            <b className="text-ui font-semibold text-foreground">{entry.title}</b>
            {entry.status && (
              <small className={`text-[11px] tracking-[0.03em] ${ink[entry.edge]} ${entry.edge === "warning" ? "motion-safe:animate-pulse" : ""}`}>{entry.status}</small>
            )}
          </header>
          <p className="mt-1 px-4 text-ui leading-relaxed text-muted-foreground">{stream(entry.text, span(p, 0, codeAt))}</p>
          {(entry.caption || entry.code) && p >= codeAt && (
            <div className="mt-2 px-4 animate-entry-in motion-reduce:animate-none">
              {entry.caption && <small className="block text-[11px] uppercase tracking-[0.03em] text-muted-foreground">{entry.caption}</small>}
              {entry.code && <Code>{stream(entry.code, span(p, codeAt, actionsAt - 0.04))}</Code>}
            </div>
          )}
          {entry.actions && p >= actionsAt && (
            <div className="flex flex-wrap items-center gap-2 px-4 pb-3 pt-3 animate-entry-in motion-reduce:animate-none">
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
          {!(entry.actions && p >= actionsAt) && <div className="pb-3" />}
        </div>
      );
    }
  }
}
