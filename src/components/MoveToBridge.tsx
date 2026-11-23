import { type ReactNode, useEffect, useRef, useState } from "react";
import { LoaderCircle as CircleNotch } from "lucide-react";
import { bridgeApi } from "../api";
import type { DetectedImportSource, ImportJob } from "../protocol/generated/protocol";
import { finishedLabel, folderLabel, importPercent, newerJob, sourceSummary, timeLeft } from "../moveToBridge";
import { cn } from "@/lib/utils";
import { HarnessMark } from "./harnessMarks";
import { ImportTransfer } from "./ImportTransfer";
import { PrimaryButton, StatusPill } from "./settings/kit";

// "Move to Bridge": every tool found on this Mac in its own section, with its
// mark, what it holds, and its own Import button. A tool's import runs in
// Bridge's own process, a small batch at a time, and its progress and its last
// result show in its own section; the page can close while it works, and
// Bridge says when it is done. One import runs at a time.

const ERRORS_SHOWN = 5;

/** The marks Bridge draws for a tool; the others get their initial. */
const TOOL_MARKS: Record<string, string> = { claude_code: "claude", codex: "codex", opencode: "opencode" };

export function MoveToBridge({ onError }: { onError: (message: string) => void }) {
  const [sources, setSources] = useState<DetectedImportSource[] | null>(null);
  const [jobs, setJobs] = useState<Record<string, ImportJob>>({});
  const [starting, setStarting] = useState<string | null>(null);
  const report = (error: unknown) => onError(error instanceof Error ? error.message : String(error));
  const merge = (next: ImportJob) => setJobs(current => ({ ...current, [next.tool]: newerJob(current[next.tool] ?? null, next) }));

  useEffect(() => {
    let cancelled = false;
    let stop: (() => void) | undefined;
    bridgeApi.detectImportSources()
      .then(found => { if (!cancelled) setSources(found.sources); })
      .catch(error => { if (!cancelled) { setSources([]); report(error); } });
    // The page may open in the middle of an import, or after earlier ones:
    // each tool's section shows its own latest.
    bridgeApi.importStatus()
      .then(status => {
        if (cancelled) return;
        const known = status.jobs?.length ? status.jobs : status.job ? [status.job] : [];
        known.forEach(merge);
      })
      .catch(() => undefined);
    void bridgeApi.onImportProgress(next => { if (!cancelled) merge(next); })
      .then(unlisten => { if (cancelled) unlisten(); else stop = unlisten; });
    return () => { cancelled = true; stop?.(); };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  async function start(tool: string) {
    setStarting(tool);
    try {
      merge(await bridgeApi.startImport(tool));
    } catch (error) { report(error); }
    finally { setStarting(null); }
  }

  const runningTool = Object.values(jobs).find(job => job.status === "running")?.tool ?? starting;
  return <div>
    <h2 className="mb-2 px-0.5 text-meta font-medium text-muted-foreground">Found on this Mac</h2>
    {sources === null && <p className="flex items-center gap-2 px-0.5 text-meta text-muted-foreground"><CircleNotch size={12} strokeWidth={1.7} className="animate-spin" aria-hidden="true" />Looking for other tools…</p>}
    <div className="space-y-3">
      {sources?.map(source => <ToolSection
        key={source.tool}
        source={source}
        job={jobs[source.tool]}
        starting={starting === source.tool}
        otherRunning={!!runningTool && runningTool !== source.tool}
        onImport={() => void start(source.tool)}
      />)}
    </div>
    {sources && sources.length > 0 && <p className="mt-3 px-0.5 text-caption text-muted-foreground">Import brings over chats and setup. Setup arrives switched off.</p>}
  </div>;
}

function ToolMark({ tool, label }: { tool: string; label: string }) {
  const harness = TOOL_MARKS[tool];
  return <div className="grid size-9 shrink-0 place-items-center rounded-lg border border-border-card bg-background" aria-hidden="true" data-tool-mark={tool}>
    {harness ? <HarnessMark harness={harness} size={18} /> : <span className="text-meta font-semibold text-muted-foreground">{label.slice(0, 1)}</span>}
  </div>;
}

/** One tool: its mark, what it holds, its Import button, and its own import. */
function ToolSection({ source, job, starting, otherRunning, onImport }: {
  source: DetectedImportSource;
  job?: ImportJob;
  starting: boolean;
  otherRunning: boolean;
  onImport: () => void;
}) {
  const importable = source.supported && source.installed;
  const running = job?.status === "running";
  return <section aria-label={source.label} data-import-tool={source.tool} className={cn("rounded-xl border border-border-card bg-card", !source.installed && "opacity-60")}>
    <div className="flex items-start gap-3 p-4">
      <ToolMark tool={source.tool} label={source.label} />
      <div className="min-w-0 flex-1">
        <h3 className="text-ui font-medium text-foreground">{source.label}</h3>
        <p className="mt-0.5 text-meta text-muted-foreground">{sourceSummary(source)}</p>
        {source.installed && source.note && <p className="mt-0.5 text-caption text-muted-foreground/80">{source.note}</p>}
        {importable && otherRunning && !running && <p className="mt-1 text-caption text-muted-foreground">Waits for the import that is running.</p>}
      </div>
      {importable
        ? <PrimaryButton disabled={running || starting || otherRunning} onClick={onImport}>
            {(running || starting) && <CircleNotch size={12} strokeWidth={1.7} className="animate-spin" aria-hidden="true" />}
            {running ? "Importing…" : job ? "Import again" : "Import"}
          </PrimaryButton>
        : <StatusPill tone={source.installed ? "warning" : "neutral"}>{source.installed ? "Coming" : "Not found"}</StatusPill>}
    </div>
    {job && <div className="border-t border-border-card/70 p-4">
      {running ? <ImportProgress job={job} /> : <ImportSummary job={job} />}
    </div>}
  </section>;
}

/** A number that glides to its new value instead of jumping. */
function useGliding(target: number): number {
  const [shown, setShown] = useState(target);
  const from = useRef(target);
  useEffect(() => {
    const reduce = typeof window.matchMedia === "function" && window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    if (reduce || typeof requestAnimationFrame !== "function") { setShown(target); from.current = target; return; }
    const origin = from.current;
    let start: number | null = null;
    let frame = 0;
    const step = (now: number) => {
      start ??= now;
      const t = Math.min(1, (now - start) / 450);
      const value = Math.round(origin + (target - origin) * (1 - (1 - t) ** 3));
      setShown(value);
      from.current = value;
      if (t < 1) frame = requestAnimationFrame(step);
    };
    frame = requestAnimationFrame(step);
    return () => cancelAnimationFrame(frame);
  }, [target]);
  return shown;
}

/** The clock, once a second, so the time left keeps moving between batches. */
function useNow(): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, []);
  return now;
}

function ImportProgress({ job }: { job: ImportJob }) {
  const counting = job.total === 0;
  const percent = importPercent(job);
  const processed = useGliding(job.processed);
  const imported = useGliding(job.imported);
  const now = useNow();
  return <section aria-label="Import progress" data-import-job={job.status}>
    <div className="flex items-center gap-2 text-ui font-medium text-foreground">
      <CircleNotch size={13} strokeWidth={1.8} className="animate-spin text-muted-foreground" aria-hidden="true" />
      {counting ? "Finding your chats…" : "Bringing your chats over"}
      {!counting && <span className="ml-auto font-mono text-caption tabular-nums text-muted-foreground">{percent}%</span>}
    </div>
    <div className="mx-auto mt-4 max-w-md px-2"><ImportTransfer tool={job.tool} /></div>
    <div className="mt-4 h-1.5 w-full overflow-hidden rounded-full bg-muted">
      <div
        role="progressbar"
        aria-label="Import progress"
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={counting ? undefined : percent}
        className="import-progress-sweep h-full rounded-full bg-foreground/80 transition-[width] duration-700 ease-out"
        style={{ width: counting ? "100%" : `${Math.max(percent, 2)}%`, opacity: counting ? 0.35 : 1 }}
      />
    </div>
    {!counting && <p className="mt-3 text-meta tabular-nums text-muted-foreground">
      <span className="text-foreground">{processed.toLocaleString()}</span> of {job.total.toLocaleString()}
      {" · "}<span data-import-result="imported">{imported.toLocaleString()}</span> came over
      {job.alreadyInBridge > 0 && <> · {job.alreadyInBridge.toLocaleString()} already in Bridge</>}
      <span className="float-right" data-import-time-left>{timeLeft(job, now)}</span>
    </p>}
    {job.currentProject && <p className="mt-1 truncate text-caption text-muted-foreground/80">Now reading {folderLabel(job.currentProject)}</p>}
    <p className="mt-3 text-caption text-muted-foreground">You can leave this page or close the window. Bridge lets you know when it is done.</p>
  </section>;
}

function Count({ value, tone = "muted", result }: { value: number; tone?: "muted" | "plain" | "warning"; result?: string }) {
  const color = tone === "warning" ? "text-warning" : tone === "plain" ? "text-foreground" : "text-muted-foreground";
  return <span data-import-result={result} className={`font-mono text-caption tabular-nums ${color}`}>{value.toLocaleString()}</span>;
}

function Stat({ label, children }: { label: string; children: ReactNode }) {
  return <div className="min-w-0"><dt className="truncate text-caption text-muted-foreground">{label}</dt><dd>{children}</dd></div>;
}

function ImportSummary({ job }: { job: ImportJob }) {
  const failed = job.status === "failed";
  const errors = job.errors ?? [];
  const notCarried = job.notCarried ?? [];
  const extraErrors = errors.length - ERRORS_SHOWN;
  return <div aria-label="Import result">
    <div className="flex items-baseline gap-2">
      <span className="text-ui font-medium text-foreground">{failed ? "Import stopped" : "Import finished"}</span>
      <span className="ml-auto text-caption text-muted-foreground">{finishedLabel(job)}</span>
    </div>
    {!failed && <div className="mx-auto my-3 max-w-md px-2" data-import-done><ImportTransfer tool={job.tool} done /></div>}
    <dl className="mt-2 grid grid-cols-2 gap-x-6 gap-y-2 sm:grid-cols-4">
      <Stat label="Came over"><Count value={job.imported} tone="plain" result="imported" /></Stat>
      {(job.workspaces ?? 0) > 0 && <Stat label="Workspaces"><Count value={job.workspaces ?? 0} tone="plain" result="workspaces" /></Stat>}
      <Stat label="Already in Bridge"><Count value={job.alreadyInBridge} result="skipped" /></Stat>
      {job.changed > 0 && <Stat label="Changed since the last import"><Count value={job.changed} /></Stat>}
      {job.leftOut > 0 && <Stat label={job.tool === "conductor" ? "Archived in Conductor, left out" : "Background runs left out"}><Count value={job.leftOut} /></Stat>}
      {job.failed > 0 && <Stat label="Could not be imported"><Count value={job.failed} tone="warning" /></Stat>}
    </dl>
    {(job.imported > 0 || (job.workspaces ?? 0) > 0) && <p className="mt-3 text-caption text-muted-foreground">{job.tool === "conductor"
      ? "The workspaces are in the sidebar under their project, in the same folders, so Conductor keeps working with them. Import Claude Code and Codex to bring their chats."
      : job.tool === "t3code"
        ? "The chats are in the sidebar under their workspace or project. T3 Code's worktrees are Bridge workspaces now, in the same folders, so T3 Code keeps working with them."
        : "Imported chats are in the sidebar, under their project when Bridge knows the folder."}</p>}
    {notCarried.length > 0 && <div className="mt-3">
      <p className="text-meta font-medium text-foreground">What did not come over</p>
      <ul className="mt-1 space-y-0.5 text-caption text-muted-foreground">{notCarried.map(item => <li key={item.text}>{item.count.toLocaleString()} × {item.text}</li>)}</ul>
    </div>}
    {errors.length > 0 && <ul className="mt-3 space-y-0.5 text-caption text-warning">
      {errors.slice(0, ERRORS_SHOWN).map(error => <li key={error}>{error}</li>)}
      {extraErrors > 0 && <li className="text-muted-foreground">and {extraErrors} more</li>}
    </ul>}
  </div>;
}
