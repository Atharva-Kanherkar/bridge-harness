import { useMemo, useState, type ReactNode } from "react";
import { Brain, Check, FilePlus2, FileText, Folder, GitFork, Globe, ListChecks, LoaderCircle, Navigation, Pencil, Search, SquareTerminal, Wrench, X } from "lucide-react";
import type { ToolCallDisplay, ToolGlyph } from "../conversation";
import { EFFECT_LABEL, type CommandEffect, type CommandStep } from "../transcript/command";
import { detectStructuredOutput, ghSummary, listRows, runSummary, type GhSummary, type StructuredOutput } from "../transcript/outputRender";
import { looksLikeDiff } from "./highlight";
import { PatchView } from "./DiffView";
import { CopyButton, Markdown } from "./Markdown";
import { CONNECTOR_LOGOS, GitMark, logoForMcpServer } from "./connectorLogos";
import { cn } from "@/lib/utils";

// A shell call, opened: the steps it chained, its output drawn when Bridge
// knows the shape, and the command and raw output exactly as they ran. The
// row above it already says what the call was for; this is the evidence.

const GitHubMark = CONNECTOR_LOGOS.github;

// The row is achromatic on purpose: the tool glyph identifies the action, and
// colour is left to the things that carry meaning: diffstats and failures.
// Brand marks (git, GitHub, a connector) are identity, not decoration, and
// render in `currentColor`.
const TOOL_ICON: Record<ToolGlyph, ReactNode> = {
  pencil: <Pencil size={12}/>,
  "file-plus": <FilePlus2 size={12}/>,
  file: <FileText size={12}/>,
  folder: <Folder size={12}/>,
  terminal: <SquareTerminal size={12}/>,
  search: <Search size={12}/>,
  globe: <Globe size={12}/>,
  fork: <GitFork size={12}/>,
  list: <ListChecks size={12}/>,
  wrench: <Wrench size={12}/>,
  brain: <Brain size={12}/>,
  navigation: <Navigation size={12}/>,
  git: <GitMark size={12}/>,
  github: <GitHubMark size={12}/>,
  mcp: <Wrench size={12}/>,
};

/// A call's icon: its glyph, or for an MCP call the server's connector logo
/// when Bridge knows one.
export function toolIcon(call: { glyph: ToolGlyph; server?: string }): ReactNode {
  if (call.glyph === "mcp" && call.server) {
    const Logo = logoForMcpServer(call.server);
    if (Logo) return <Logo size={12}/>;
  }
  return TOOL_ICON[call.glyph];
}

const CHIP = "inline-flex shrink-0 items-center rounded-full border border-border px-1.5 py-px text-[10px] leading-4 text-muted-foreground";

/// What a shell command does to the machine, as a neutral outlined chip. It is
/// a fact to scan for, not an alarm, so it never takes a colour.
export function EffectChip({ effect, className }: { effect: CommandEffect; className?: string }) {
  return <span data-effect={effect} className={cn(CHIP, className)}>{EFFECT_LABEL[effect]}</span>;
}

const OUTPUT_DISPLAY_CAP = 6000;

export function CappedOutput({ text, className }: { text: string; className?: string }) {
  const [all, setAll] = useState(false);
  const clipped = !all && text.length > OUTPUT_DISPLAY_CAP;
  return (
    <div>
      <pre className={className}>{clipped ? text.slice(-OUTPUT_DISPLAY_CAP) : text}</pre>
      {clipped && (
        <button
          type="button"
          className="px-3.5 pb-2.5 text-left text-[11px] text-muted-foreground underline-offset-2 hover:text-foreground hover:underline"
          onClick={() => setAll(true)}
        >
          Show earlier output
        </button>
      )}
    </div>
  );
}

/// A command the way a terminal shows one: a `❯` prompt line carrying what ran,
/// and the output dimmed a step below it on the code ground.
export function TerminalBlock({ command, output }: { command?: string; output?: string }) {
  return <div className="bg-code font-mono text-[12px] leading-[1.7]">
    {command && <div className="flex gap-2 px-3.5 pb-1 pt-2.5">
      <span className="shrink-0 select-none font-semibold text-success" aria-hidden="true">❯</span>
      <span className="min-w-0 whitespace-pre-wrap break-words text-foreground">{command}</span>
    </div>}
    {output && <CappedOutput text={output} className="max-h-[260px] overflow-auto whitespace-pre-wrap break-words px-3.5 pb-2.5 pl-[30px] text-muted-foreground"/>}
  </div>;
}

/// The steps a command chained, numbered in the order they run. The plumbing
/// each one carried (`2>/dev/null`, `| head -50`) is a note, not a word of the
/// command the reader has to parse.
function StepsList({ steps, live }: { steps: CommandStep[]; live: boolean }) {
  return <ol data-command-steps className="grid py-1">
    {steps.map((step, index) => (
      <li
        key={index}
        title={step.text}
        className="grid grid-cols-[1rem_0.875rem_minmax(0,1fr)] items-center gap-x-2.5 gap-y-1 border-t border-dashed border-border/60 px-3.5 py-1.5 text-[12.5px] first:border-t-0 sm:grid-cols-[1rem_0.875rem_minmax(0,1fr)_auto]"
      >
        <span className="text-right text-[11px] tabular-nums text-faint">{index + 1}</span>
        <span className="flex text-muted-foreground" aria-hidden="true">{toolIcon(step)}</span>
        <span className="flex min-w-0 items-baseline gap-1.5">
          <span className="shrink-0 text-foreground">{live ? step.doing : step.done}</span>
          {step.target && <code className="min-w-0 truncate rounded bg-muted px-1 text-[11.5px] text-foreground">{step.target}</code>}
        </span>
        <span className="col-start-3 flex flex-wrap gap-1 sm:col-start-auto sm:justify-end">
          {step.notes.map(note => <span key={note} className={CHIP}>{note}</span>)}
          {step.effect !== "read" && <EffectChip effect={step.effect}/>}
        </span>
      </li>
    ))}
  </ol>;
}

type View = "steps" | "rendered" | "output";
const VIEW_LABEL: Record<View, string> = { steps: "Steps", rendered: "Rendered", output: "Output" };

/// A shell call's body. One view is just that view; more than one is a tab
/// bar, opening on the most readable: drawn output, else the steps, else the
/// terminal. The command and its raw output are always one tab away.
export function CommandPanel({ call, live }: { call: ToolCallDisplay; live: boolean }) {
  // Detected only while the panel is open: never at ingestion, never per delta
  // for a row nobody is looking at.
  const structured = useMemo(() => detectStructuredOutput(call.command, call.output), [call.command, call.output]);
  const steps = call.steps && call.steps.length > 1 ? call.steps : undefined;
  const views: View[] = [...(steps ? ["steps" as const] : []), ...(structured ? ["rendered" as const] : []), "output"];
  const [chosen, setChosen] = useState<View | null>(null);
  const active: View = chosen && views.includes(chosen) ? chosen : structured ? "rendered" : steps ? "steps" : "output";
  // A diff read through the shell (`git diff`) still reads best as a diff.
  const diff = call.verb !== "run" && !!call.output && looksLikeDiff(call.output);
  const output = diff
    ? <PatchView patch={call.output ?? ""} path={call.path ?? ""} className="max-h-[320px] px-1" foldAfterHunks={2}/>
    : <TerminalBlock command={call.command} output={call.output}/>;
  if (views.length === 1) return output;
  return <div data-command-panel>
    <div role="tablist" className="flex items-center gap-0.5 border-b border-border/60 px-2 py-1.5 text-[11.5px]">
      {views.map(view => (
        <button
          key={view}
          type="button"
          role="tab"
          aria-selected={active === view}
          className={cn("rounded-md px-2 py-0.5 transition-colors", active === view ? "bg-accent text-foreground" : "text-muted-foreground hover:text-foreground")}
          onClick={() => setChosen(view)}
        >{VIEW_LABEL[view]}</button>
      ))}
      {call.command && <CopyButton text={call.command} className="ml-auto inline-flex items-center gap-1 rounded-md px-1.5 py-0.5 text-[11px] text-muted-foreground transition-colors hover:text-foreground"/>}
    </div>
    <div role="tabpanel">
      {active === "steps" && steps && <StepsList steps={steps} live={live}/>}
      {active === "rendered" && structured && <StructuredView structured={structured}/>}
      {active === "output" && output}
    </div>
  </div>;
}

/* ── Drawn output ─────────────────────────────────────────────────────── */

function StructuredView({ structured }: { structured: StructuredOutput }) {
  return <div data-structured={structured.kind} className="max-h-[520px] overflow-auto">
    {structured.truncated && <p className="px-4 pt-3 text-[11.5px] text-muted-foreground">Output was cut off; showing what arrived.</p>}
    {(structured.kind === "gh-pr" || structured.kind === "gh-issue") && <GhCard summary={ghSummary(structured.data)} pr={structured.kind === "gh-pr"}/>}
    {structured.kind === "gh-run" && <RunCard data={structured.data}/>}
    {structured.kind === "gh-list" && <ListView data={structured.data}/>}
    {structured.kind === "json" && <div className="px-3.5 py-2.5 font-mono text-[12px] leading-[1.65]"><JsonNode value={structured.data} depth={0}/></div>}
  </div>;
}

/// The dot a state wears: green for open or passing, red for closed or
/// failing, the ink colour for merged or done, muted for anything else.
function StateBadge({ state, draft }: { state?: string; draft?: boolean }) {
  if (!state && !draft) return null;
  const word = draft ? "draft" : state!.toLowerCase().replaceAll("_", " ");
  const tone = draft ? "bg-muted-foreground" : /^(open|success|completed|passed)$/.test(word) ? "bg-success" : /^(closed|failure|failed|cancelled|timed out|error)$/.test(word) ? "bg-destructive" : /^(merged)$/.test(word) ? "bg-foreground" : "bg-muted-foreground";
  return <span className={cn(CHIP, "gap-1.5 text-[11px] capitalize")}><span className={cn("size-1.5 rounded-full", tone)} aria-hidden="true"/>{word}</span>;
}

function Section({ label, children }: { label: string; children: ReactNode }) {
  return <section className="space-y-1.5">
    <h5 className="text-[10.5px] font-medium uppercase tracking-[0.06em] text-muted-foreground">{label}</h5>
    {children}
  </section>;
}

const SHOWN_FILES = 6;

/// `CHANGES_REQUESTED` → "Changes requested".
function sentence(word: string): string {
  const spaced = word.toLowerCase().replaceAll("_", " ");
  return spaced.charAt(0).toUpperCase() + spaced.slice(1);
}

function GhCard({ summary, pr }: { summary: GhSummary; pr: boolean }) {
  const stats = pr ? [
    { label: "Files", value: summary.changedFiles !== undefined ? String(summary.changedFiles) : undefined },
    { label: "Lines", value: summary.additions !== undefined ? <><span className="text-success">+{summary.additions}</span> <span className="text-destructive">−{summary.deletions ?? 0}</span></> : undefined },
    { label: "Commits", value: summary.commits !== undefined ? String(summary.commits) : undefined },
    { label: "Reviews", value: summary.reviewDecision ? sentence(summary.reviewDecision) : summary.reviews !== undefined ? (summary.reviews ? String(summary.reviews) : "None yet") : undefined },
  ].filter(stat => stat.value !== undefined) : [];
  const checks = summary.checks;
  return <div className="space-y-3 px-4 py-3.5 text-[12.5px]">
    <div className="flex flex-wrap items-center gap-2 text-[12px] text-muted-foreground">
      <span className="flex" aria-hidden="true"><GitHubMark size={13}/></span>
      <span>{[summary.repo, summary.number !== undefined ? `#${summary.number}` : undefined].filter(Boolean).join(" · ") || (pr ? "Pull request" : "Issue")}</span>
      <StateBadge state={summary.state} draft={summary.draft}/>
      {checks && <span>{checks.failed ? <span className="text-destructive">{checks.failed} failing</span> : checks.pending ? `${checks.pending} checks running` : `${checks.passed} checks passed`}</span>}
      {!pr && summary.comments !== undefined && <span>{summary.comments} comment{summary.comments === 1 ? "" : "s"}</span>}
    </div>
    {summary.title && <h4 className="text-[15px] font-medium leading-snug tracking-[-0.01em] text-foreground">{summary.title}</h4>}
    {(summary.author || summary.head) && <p className="flex flex-wrap items-center gap-1.5 text-[12px] text-muted-foreground">
      {summary.author && <span>{summary.author}{pr && summary.head ? " wants to merge" : ""}</span>}
      {pr && summary.head && <><code className="rounded bg-muted px-1.5 text-[11.5px] text-foreground">{summary.head}</code>{summary.base && <>into<code className="rounded bg-muted px-1.5 text-[11.5px] text-foreground">{summary.base}</code></>}</>}
      {summary.labels.map(label => <span key={label} className={CHIP}>{label}</span>)}
    </p>}
    {stats.length > 0 && <dl className="grid grid-cols-2 overflow-hidden rounded-lg border border-border sm:grid-cols-4">
      {stats.map(stat => <div key={stat.label} className="border-border px-3 py-2 not-last:border-r">
        <dt className="text-[10.5px] uppercase tracking-[0.05em] text-muted-foreground">{stat.label}</dt>
        <dd className="mt-0.5 text-[14px] tabular-nums text-foreground">{stat.value}</dd>
      </div>)}
    </dl>}
    {summary.body?.trim() && <Section label="Description">
      <div className="max-h-[320px] overflow-auto rounded-lg border border-border/60 px-3 py-2.5 [&_.md]:text-[13px]!"><Markdown text={summary.body}/></div>
    </Section>}
    {summary.files.length > 0 && <Section label="Changed files">
      <ul className="font-mono text-[11.5px] leading-[1.9]">
        {summary.files.slice(0, SHOWN_FILES).map(file => <li key={file.path} className="flex gap-2.5">
          <span className="w-3 shrink-0 text-faint">{file.change ?? ""}</span>
          <span className="min-w-0 flex-1 truncate text-foreground" title={file.path}>{file.path}</span>
          {file.additions !== undefined && <span className="shrink-0 tabular-nums"><span className="text-success">+{file.additions}</span> <span className="text-destructive">−{file.deletions ?? 0}</span></span>}
        </li>)}
      </ul>
      {summary.files.length > SHOWN_FILES && <p className="text-[11.5px] text-muted-foreground">+ {summary.files.length - SHOWN_FILES} more files</p>}
    </Section>}
  </div>;
}

function JobGlyph({ status, conclusion }: { status?: string; conclusion?: string }) {
  const outcome = (conclusion || status || "").toLowerCase();
  if (outcome === "success") return <Check size={12} className="text-success" aria-hidden="true"/>;
  if (/failure|cancelled|timed_out|startup_failure/.test(outcome)) return <X size={12} className="text-destructive" aria-hidden="true"/>;
  if (/in_progress|queued|pending|waiting/.test(outcome)) return <LoaderCircle size={12} className="text-muted-foreground" aria-hidden="true"/>;
  return <span className="size-1.5 rounded-full bg-muted-foreground" aria-hidden="true"/>;
}

function RunCard({ data }: { data: unknown }) {
  const run = runSummary(data);
  return <div className="space-y-3 px-4 py-3.5 text-[12.5px]">
    <div className="flex flex-wrap items-center gap-2 text-[12px] text-muted-foreground">
      <span className="flex" aria-hidden="true"><GitHubMark size={13}/></span>
      <span>{[run.workflow, run.branch, run.event].filter(Boolean).join(" · ") || "Workflow run"}</span>
      <StateBadge state={run.conclusion || run.status}/>
    </div>
    {run.title && <h4 className="text-[15px] font-medium leading-snug tracking-[-0.01em] text-foreground">{run.title}</h4>}
    {run.jobs.length > 0 && <Section label="Jobs">
      <ul className="grid gap-1">
        {run.jobs.map(job => <li key={job.name} className="flex items-center gap-2"><span className="flex size-3 items-center justify-center"><JobGlyph status={job.status} conclusion={job.conclusion}/></span><span className="text-foreground">{job.name}</span><span className="ml-auto text-[11.5px] text-muted-foreground">{(job.conclusion || job.status || "").replaceAll("_", " ")}</span></li>)}
      </ul>
    </Section>}
  </div>;
}

function ListView({ data }: { data: unknown }) {
  const rows = listRows(data);
  return <ul className="grid py-1.5 text-[12.5px]">
    {rows.map(row => <li key={row.id} className="flex items-center gap-2.5 px-4 py-1">
      <span className="w-12 shrink-0 font-mono text-[11.5px] text-faint">{row.id}</span>
      <span className="min-w-0 flex-1 truncate text-foreground">{row.title}</span>
      {row.state && <StateBadge state={row.state}/>}
    </li>)}
  </ul>;
}

/* ── JSON tree ────────────────────────────────────────────────────────── */

const SHOWN_ENTRIES = 200;
const SHOWN_CHARS = 4000;

function JsonScalar({ value }: { value: unknown }) {
  if (typeof value === "string") {
    const shown = value.length > SHOWN_CHARS ? `${value.slice(0, SHOWN_CHARS)}…` : value;
    return <span className="min-w-0 whitespace-pre-wrap break-words text-syn-string">{value.includes("\n") ? shown : JSON.stringify(shown)}</span>;
  }
  if (typeof value === "number") return <span className="text-syn-number">{value}</span>;
  return <span className="text-syn-keyword">{String(value)}</span>;
}

function JsonNode({ name, value, depth }: { name?: string; value: unknown; depth: number }) {
  const key = name !== undefined && <span className="shrink-0 text-syn-property">{name}:</span>;
  if (value === null || typeof value !== "object") {
    return <div className="flex min-w-0 gap-1.5">{key}<JsonScalar value={value}/></div>;
  }
  const entries: Array<[string, unknown]> = Array.isArray(value) ? value.map((entry, index) => [String(index), entry]) : Object.entries(value);
  const shape = Array.isArray(value) ? `[${entries.length}]` : `{${entries.length}}`;
  return <details open={depth < 2} className="min-w-0">
    <summary className="cursor-pointer select-none text-muted-foreground marker:text-faint">{key} <span className="text-faint">{shape}</span></summary>
    <div className="ml-1.5 border-l border-border/60 pl-3">
      {entries.slice(0, SHOWN_ENTRIES).map(([entryName, entry]) => <JsonNode key={entryName} name={entryName} value={entry} depth={depth + 1}/>)}
      {entries.length > SHOWN_ENTRIES && <div className="text-faint">… {entries.length - SHOWN_ENTRIES} more</div>}
    </div>
  </details>;
}
