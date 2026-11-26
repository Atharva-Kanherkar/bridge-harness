import { useState } from "react";
import { GitMerge, GitPullRequest, GitPullRequestClosed, CircleDashed } from "lucide-react";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { RowButton } from "@/components/ui/row";
import type { ScreenTimeHarness, ScreenTimeHour, ScreenTimeProject, ScreenTimePullRequest, ScreenTimePullRequests } from "../types";
import { formatDayShort, niceScale } from "../usageReport";
import { formatDuration } from "../screenTime";
import { harnessChartFill } from "./harnessMarks";
import { HarnessName, LAYOUT_CARD, useScrub } from "./UsageLayoutParts";

// Where the Time tab's range went: projects, pull requests, hours of the day
// and harnesses. Rules 13 to 17 of testing/feat-screen-time-breakdown.md.

/** Rows the Projects card shows before `Show all`. */
export const PROJECT_ROWS = 6;
const YOU = "bg-foreground/80";
const AGENTS = "bg-foreground/25";

function share(value: number, of: number): string {
  return `${of > 0 ? Math.max(0, Math.min(1, value / of)) * 100 : 0}%`;
}

function plural(count: number, one: string, many = `${one}s`): string {
  return `${count.toLocaleString("en-US")} ${count === 1 ? one : many}`;
}

/** `12a`, `6a`, `12p`, `6p`. */
function hourTick(hour: number): string {
  return `${hour % 12 === 0 ? 12 : hour % 12}${hour < 12 ? "a" : "p"}`;
}

/** `9 am`, `12 pm`, `11 pm`. */
export function hourName(hour: number): string {
  return `${hour % 12 === 0 ? 12 : hour % 12} ${hour < 12 ? "am" : "pm"}`;
}

function CardHeading({ title, detail }: { title: string; detail?: string }) {
  return <div className="mb-3 flex flex-wrap items-baseline gap-x-3 gap-y-1">
    <h2 className="text-ui font-medium text-foreground">{title}</h2>
    {detail && <span className="text-caption text-muted-foreground">{detail}</span>}
  </div>;
}

function Empty({ children }: { children: string }) {
  return <p className="py-8 text-center text-caption text-muted-foreground">{children}</p>;
}

export function ProjectsCard({ projects }: { projects: ScreenTimeProject[] }) {
  const [all, setAll] = useState(false);
  const shown = all ? projects : projects.slice(0, PROJECT_ROWS);
  const largest = Math.max(0, ...projects.map(project => project.humanSeconds + project.agentSeconds));
  return <section aria-label="Projects" className={LAYOUT_CARD}>
    <CardHeading title="Projects" detail={projects.length > 0 ? plural(projects.length, "project") : undefined} />
    {projects.length === 0 ? <Empty>No project time in this range.</Empty> : <>
      <table className="w-full table-fixed text-caption tabular-nums">
        <thead>
          <tr className="text-left text-muted-foreground">
            <th scope="col" className="pb-2 font-normal">Project</th>
            <th scope="col" className="w-16 pb-2 text-right font-normal">You</th>
            <th scope="col" className="w-16 pb-2 text-right font-normal">Agents</th>
            <th scope="col" className="w-12 pb-2 text-right font-normal">Chats</th>
            <th scope="col" className="w-10 pb-2 text-right font-normal">PRs</th>
          </tr>
        </thead>
        <tbody>
          {shown.map(project => <tr key={project.projectId ?? "none"} data-project={project.projectId ?? "none"} className="border-t border-border">
            <th scope="row" className="py-2 pr-3 text-left font-normal">
              <span className={cn("block truncate text-ui", project.projectId ? "text-foreground" : "text-muted-foreground")}>{project.name}</span>
              <span className="mt-1.5 flex h-1 overflow-hidden rounded-full bg-foreground/5" aria-hidden="true">
                <span className={cn("h-full", YOU)} style={{ width: share(project.humanSeconds, largest) }} />
                <span className={cn("h-full", AGENTS)} style={{ width: share(project.agentSeconds, largest) }} />
              </span>
            </th>
            <td className="py-2 text-right text-foreground">{formatDuration(project.humanSeconds)}</td>
            <td className="py-2 text-right text-foreground">{formatDuration(project.agentSeconds)}</td>
            <td className="py-2 text-right text-muted-foreground">{project.chats.toLocaleString("en-US")}</td>
            <td className="py-2 text-right text-muted-foreground">{project.pullRequests.toLocaleString("en-US")}</td>
          </tr>)}
        </tbody>
      </table>
      <div className="mt-3 flex flex-wrap items-center justify-between gap-2">
        <p className="text-caption text-muted-foreground">Your time per project counts minutes a chat of it was open, from this update on.</p>
        {projects.length > PROJECT_ROWS && <Button variant="quiet" size="xs" aria-expanded={all} onClick={() => setAll(value => !value)}>
          {all ? "Show fewer" : `Show all ${projects.length}`}
        </Button>}
      </div>
    </>}
  </section>;
}

const STATE_LABEL: Record<ScreenTimePullRequest["state"], string> = { open: "Open", merged: "Merged", closed: "Closed", unknown: "State unknown" };

function StateGlyph({ state }: { state: ScreenTimePullRequest["state"] }) {
  const props = { size: 14, strokeWidth: 2, "aria-hidden": true as const };
  if (state === "merged") return <GitMerge {...props} className="shrink-0 text-merged" />;
  if (state === "closed") return <GitPullRequestClosed {...props} className="shrink-0 text-muted-foreground" />;
  if (state === "open") return <GitPullRequest {...props} className="shrink-0 text-foreground" />;
  return <CircleDashed {...props} className="shrink-0 text-muted-foreground" />;
}

export function PullRequestsCard({ pullRequests }: { pullRequests: ScreenTimePullRequests }) {
  const most = Math.max(0, ...pullRequests.repos.map(repo => repo.opened));
  return <section aria-label="Pull requests" className={LAYOUT_CARD}>
    <CardHeading title="Pull requests" detail="opened by agents in your chats" />
    {pullRequests.opened === 0 ? <Empty>No pull requests opened from Bridge in this range.</Empty> : <div className="space-y-4">
      <div className="flex flex-wrap items-baseline gap-x-4 gap-y-1">
        <span data-testid="screen-time-prs-opened" className="text-3xl font-light leading-none tracking-display tabular-nums text-foreground">{pullRequests.opened.toLocaleString("en-US")}</span>
        <span className="text-caption tabular-nums text-muted-foreground">
          opened · <span className="text-merged">{pullRequests.merged.toLocaleString("en-US")} merged</span> · {pullRequests.open.toLocaleString("en-US")} open · {pullRequests.closed.toLocaleString("en-US")} closed
        </span>
      </div>
      <ul aria-label="By repository" className="space-y-2">
        {pullRequests.repos.map(repo => <li key={repo.repo} className="grid grid-cols-[minmax(0,1fr)_auto] items-center gap-x-3 gap-y-1 text-caption tabular-nums">
          <span className="truncate text-foreground">{repo.repo}</span>
          <span className="text-muted-foreground">{plural(repo.opened, "PR")} · {repo.merged.toLocaleString("en-US")} merged</span>
          <span className="col-span-2 flex h-1 overflow-hidden rounded-full bg-foreground/5" aria-hidden="true">
            <span className="h-full bg-merged/70" style={{ width: share(repo.merged, most) }} />
            <span className="h-full bg-foreground/25" style={{ width: share(repo.opened - repo.merged, most) }} />
          </span>
        </li>)}
      </ul>
      {pullRequests.recent.length > 0 && <div>
        <h3 className="mb-1 text-caption font-medium text-muted-foreground">Latest</h3>
        <ul aria-label="Latest pull requests" className="-mx-2">
          {pullRequests.recent.map(pr => <li key={`${pr.repo}#${pr.number}`}>
            <RowButton size="sm" render={<a href={pr.url} target="_blank" rel="noreferrer" data-system-browser aria-label={`${pr.title}, ${pr.repo} #${pr.number}, ${STATE_LABEL[pr.state]}`} />}>
              <StateGlyph state={pr.state} />
              <span className="min-w-0 flex-1 truncate text-foreground">{pr.title}</span>
              <span className="shrink-0 tabular-nums text-muted-foreground">{pr.repo.split("/").pop()} #{pr.number} · {formatDayShort(pr.openedDay)}</span>
            </RowButton>
          </li>)}
        </ul>
      </div>}
    </div>}
  </section>;
}

export function HoursCard({ hours }: { hours: ScreenTimeHour[] }) {
  const scrub = useScrub(hours.length);
  const peak = Math.max(0, ...hours.map(hour => Math.max(hour.humanSeconds, hour.agentSeconds)));
  const scale = niceScale(peak / 3_600, 2);
  const maxSeconds = scale.max * 3_600;
  const yours = hours.reduce((best, hour) => hour.humanSeconds > (best?.humanSeconds ?? 0) ? hour : best, null as ScreenTimeHour | null);
  const theirs = hours.reduce((best, hour) => hour.agentSeconds > (best?.agentSeconds ?? 0) ? hour : best, null as ScreenTimeHour | null);
  const active = scrub.index !== null ? hours[scrub.index] : null;
  return <section aria-label="When you work" className={LAYOUT_CARD}>
    <div className="mb-3 flex flex-wrap items-baseline gap-x-3 gap-y-1">
      <h2 className="text-ui font-medium text-foreground">When you work</h2>
      <p role="status" aria-live="polite" className="ml-auto text-caption tabular-nums text-muted-foreground">
        {active
          ? <>{hourName(active.hour)}: you <span className="text-foreground">{formatDuration(active.humanSeconds)}</span> · agents <span className="text-foreground">{formatDuration(active.agentSeconds)}</span></>
          : yours ? <>Your busiest hour: <span className="text-foreground">{hourName(yours.hour)}</span></>
          : theirs ? <>Agents' busiest hour: <span className="text-foreground">{hourName(theirs.hour)}</span></> : null}
      </p>
    </div>
    {peak === 0 ? <Empty>No time recorded in this range.</Empty> : <div
      role="group"
      tabIndex={0}
      aria-label="Your time and agent time per hour of the day. Use the arrow keys to read one hour."
      onKeyDown={scrub.onKeyDown}
      onBlur={scrub.clear}
      onPointerLeave={scrub.clear}
      className="rounded-lg outline-none focus-visible:ring-2 focus-visible:ring-ring"
    >
      <div className="relative flex h-28 items-end border-b border-border">
        {hours.map((hour, index) => <div
          key={hour.hour}
          data-hour={hour.hour}
          onPointerEnter={() => scrub.setIndex(index)}
          className={cn("flex h-full min-w-0 flex-1 items-end justify-center gap-px rounded-sm px-px", scrub.index === index && "bg-accent/70")}
        >
          <span className={cn("w-full max-w-2 rounded-t-xs motion-safe:animate-chart-rise origin-bottom", YOU)} style={{ height: share(hour.humanSeconds, maxSeconds) }} />
          <span className={cn("w-full max-w-2 rounded-t-xs motion-safe:animate-chart-rise origin-bottom", AGENTS)} style={{ height: share(hour.agentSeconds, maxSeconds) }} />
        </div>)}
      </div>
      <div className="relative mt-1.5 h-4 text-caption tabular-nums text-muted-foreground" aria-hidden="true">
        {[0, 6, 12, 18].map(hour => <span key={hour} className="absolute top-0" style={{ left: `${(hour / 24) * 100}%` }}>{hourTick(hour)}</span>)}
      </div>
    </div>}
  </section>;
}

export function HarnessesCard({ harnesses }: { harnesses: ScreenTimeHarness[] }) {
  const total = harnesses.reduce((sum, harness) => sum + harness.agentSeconds, 0);
  return <section aria-label="Agents by harness" className={LAYOUT_CARD}>
    <CardHeading title="Agents by harness" />
    {total === 0 ? <Empty>No agent time in this range.</Empty> : <ul className="space-y-3">
      {harnesses.map(harness => <li key={harness.harness} data-harness={harness.harness} className="space-y-1.5">
        <div className="flex items-center gap-3 text-caption tabular-nums">
          <HarnessName harness={harness.harness} className="min-w-0 flex-1 text-ui text-foreground" />
          <span className="text-muted-foreground">{plural(harness.chats, "chat")}</span>
          <span className="w-16 text-right text-foreground">{formatDuration(harness.agentSeconds)}</span>
          <span className="w-10 text-right text-muted-foreground">{Math.round((harness.agentSeconds / total) * 100)}%</span>
        </div>
        <svg viewBox="0 0 100 4" preserveAspectRatio="none" className="block h-1 w-full overflow-hidden rounded-full" aria-hidden="true">
          <rect width="100" height="4" className="fill-foreground/5" />
          <rect width={(harness.agentSeconds / total) * 100} height="4" className={harnessChartFill(harness.harness)} />
        </svg>
      </li>)}
    </ul>}
  </section>;
}
