import { useEffect, useRef, useState } from "react";
import { LoaderCircle as CircleNotch } from "lucide-react";
import { bridgeApi } from "../api";
import type { DetectedImportSource, ImportJob } from "../protocol/generated/protocol";
import { finishedLabel, folderLabel, importPercent, newerJob, sourceSummary } from "../moveToBridge";
import { PrimaryButton, SettingsGroup, SettingsRow, StatusPill } from "./settings/kit";

// "Move to Bridge": the tools found on this Mac and one Import button. The
// import runs in Bridge's own process, a small batch at a time, so this page
// can close while it works; Bridge says when it is done.

const ERRORS_SHOWN = 5;

export function MoveToBridge({ onError }: { onError: (message: string) => void }) {
  const [sources, setSources] = useState<DetectedImportSource[] | null>(null);
  const [job, setJob] = useState<ImportJob | null>(null);
  const [starting, setStarting] = useState(false);
  const report = (error: unknown) => onError(error instanceof Error ? error.message : String(error));

  useEffect(() => {
    let cancelled = false;
    let stop: (() => void) | undefined;
    bridgeApi.detectImportSources()
      .then(found => { if (!cancelled) setSources(found.sources); })
      .catch(error => { if (!cancelled) { setSources([]); report(error); } });
    // The page may open in the middle of an import that started earlier.
    bridgeApi.importStatus()
      .then(status => { if (!cancelled && status.job) setJob(current => newerJob(current, status.job!)); })
      .catch(() => undefined);
    void bridgeApi.onImportProgress(next => { if (!cancelled) setJob(current => newerJob(current, next)); })
      .then(unlisten => { if (cancelled) unlisten(); else stop = unlisten; });
    return () => { cancelled = true; stop?.(); };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  async function start(tool: string) {
    setStarting(true);
    try {
      const started = await bridgeApi.startImport(tool);
      setJob(current => newerJob(current, started));
    } catch (error) { report(error); }
    finally { setStarting(false); }
  }

  const running = job?.status === "running";
  return <>
    {job && running && <ImportProgress job={job} />}
    {job && !running && <ImportSummary job={job} />}
    <SettingsGroup label="Found on this Mac" note="Import brings over your chats and setup. Setup arrives switched off.">
      {sources === null && <SettingsRow label={<span className="flex items-center gap-2 text-muted-foreground"><CircleNotch size={12} strokeWidth={1.7} className="animate-spin" aria-hidden="true" />Looking for other tools…</span>} />}
      {sources?.map(item => <SettingsRow
        key={item.tool}
        label={item.label}
        disabled={!item.installed}
        description={<span className="block">
          <span className="block">{sourceSummary(item)}</span>
          {item.installed && item.note && <span className="block text-muted-foreground">{item.note}</span>}
        </span>}
        control={item.supported && item.installed
          ? <PrimaryButton disabled={running || starting} onClick={() => void start(item.tool)}>
              {(running || starting) && <CircleNotch size={12} strokeWidth={1.7} className="animate-spin" aria-hidden="true" />}
              {running ? "Importing…" : job?.tool === item.tool ? "Import again" : "Import"}
            </PrimaryButton>
          : <StatusPill tone={item.installed ? "warning" : "neutral"}>{item.installed ? "Coming" : "Not found"}</StatusPill>}
      />)}
    </SettingsGroup>
  </>;
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

function ImportProgress({ job }: { job: ImportJob }) {
  const counting = job.total === 0;
  const percent = importPercent(job);
  const processed = useGliding(job.processed);
  const imported = useGliding(job.imported);
  return <section aria-label="Import progress" data-import-job={job.status} className="mb-6 rounded-xl border border-border-card bg-card p-4">
    <div className="flex items-center gap-2 text-[13px] font-medium text-foreground">
      <CircleNotch size={13} strokeWidth={1.8} className="animate-spin text-muted-foreground" aria-hidden="true" />
      {counting ? "Finding your chats…" : "Bringing your chats over"}
      {!counting && <span className="ml-auto font-mono text-[11px] tabular-nums text-muted-foreground">{percent}%</span>}
    </div>
    <div className="mt-3 h-1.5 w-full overflow-hidden rounded-full bg-muted">
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
    {!counting && <p className="mt-3 text-[12px] tabular-nums text-muted-foreground">
      <span className="text-foreground">{processed.toLocaleString()}</span> of {job.total.toLocaleString()}
      {" · "}<span data-import-result="imported">{imported.toLocaleString()}</span> came over
      {job.alreadyInBridge > 0 && <> · {job.alreadyInBridge.toLocaleString()} already in Bridge</>}
    </p>}
    {job.currentProject && <p className="mt-1 truncate text-[11.5px] text-muted-foreground/80">Now reading {folderLabel(job.currentProject)}</p>}
    <p className="mt-3 text-[11.5px] text-muted-foreground">You can leave this page or close the window. Bridge lets you know when it is done.</p>
  </section>;
}

function Count({ value, tone = "muted", result }: { value: number; tone?: "muted" | "plain" | "warning"; result?: string }) {
  const color = tone === "warning" ? "text-warning" : tone === "plain" ? "text-foreground" : "text-muted-foreground";
  return <span data-import-result={result} className={`font-mono text-[11px] tabular-nums ${color}`}>{value.toLocaleString()}</span>;
}

function ImportSummary({ job }: { job: ImportJob }) {
  const failed = job.status === "failed";
  const errors = job.errors ?? [];
  const notCarried = job.notCarried ?? [];
  const extraErrors = errors.length - ERRORS_SHOWN;
  return <SettingsGroup
    label={failed ? "Import stopped" : "Import finished"}
    note={finishedLabel(job)}
  >
    <SettingsRow label="Came over" description="Imported chats are in the sidebar, under their project when Bridge knows the folder." control={<Count value={job.imported} tone="plain" result="imported" />} />
    <SettingsRow label="Already in Bridge" description="Imported before, or chats Bridge ran itself." control={<Count value={job.alreadyInBridge} result="skipped" />} />
    {job.changed > 0 && <SettingsRow label="Changed since the last import" description="Kept as they were imported." control={<Count value={job.changed} />} />}
    {job.leftOut > 0 && <SettingsRow label="Background runs left out" description="Sessions Claude Code ran in a temporary folder, such as SDK probes. Not chats you had." control={<Count value={job.leftOut} />} />}
    {job.failed > 0 && <SettingsRow label="Could not be imported" control={<Count value={job.failed} tone="warning" />} />}
    {notCarried.length > 0 && <SettingsRow
      label="What did not come over"
      description={<span className="block">{notCarried.map(item => <span key={item.text} className="block">{item.count.toLocaleString()} × {item.text}</span>)}</span>}
    />}
    {errors.slice(0, ERRORS_SHOWN).map(error => <SettingsRow key={error} label={<span className="text-warning">{error}</span>} />)}
    {extraErrors > 0 && <SettingsRow label={<span className="text-muted-foreground">and {extraErrors} more</span>} />}
  </SettingsGroup>;
}
