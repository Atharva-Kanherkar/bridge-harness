import { useEffect, useState, type ComponentType } from "react";
import {
  Archive, ArchiveRestore, CircleAlert, CircleDot, FileText, GitBranch, GitPullRequest, MessageSquare,
  MessagesSquare, Plus, RefreshCw, Route, Send, X,
} from "lucide-react";
import type { PreviewTrailBriefResult, TrailDetail, TrailEvent, TrailSummary } from "@/protocol/generated/protocol";
import { bridgeApi } from "@/api";
import { TRAIL_STATE_LABEL, briefKind, formatTokens, groupByDay, memberNoun, stageLabel, timeLabel, type BriefKind } from "@/trails";
import { HarnessMark } from "@/components/harnessMarks";
import { harnessLabel } from "@/utils";
import { Button } from "@/components/ui/button";
import { Chip, KIND_ICON, KIND_LABEL, RailSection, StageBar, TONE_INK, TrailScroll, riseDelay } from "./TrailParts";
import { TrailBriefSkeleton, TrailHeaderSkeleton, TrailRailSkeleton, TrailTimelineSkeleton } from "./TrailSkeletons";
import { clsx as cn } from "clsx";
import { Tip } from "@/components/ui/tooltip";

const BRIEF_ICON: Record<BriefKind, ComponentType<{ className?: string }>> = {
  issue: CircleDot,
  note: FileText,
  pullRequest: GitPullRequest,
  chat: MessagesSquare,
  message: Send,
  other: MessageSquare,
};

function EventRow({ event, last, order, onOpen, onRemove }: { event: TrailEvent; last: boolean; order: number; onOpen?: () => void; onRemove?: () => void }) {
  const Icon = KIND_ICON[event.kind];
  const title = <span className="min-w-0 flex-1 truncate text-ui font-medium text-foreground">{event.title}</span>;
  const auto = event.addedBy === "auto" && event.kind !== "checks";
  return (
    <li className="trail-reveal group relative pb-5" data-event-kind={event.kind}>
      {!last && <span aria-hidden="true" className="trail-draw absolute left-3.5 top-8 h-full w-px bg-border" style={riseDelay(order)} />}
      <div className="trail-rise relative flex gap-3" style={riseDelay(order)}>
        <span className="relative z-10 flex size-7 shrink-0 items-center justify-center rounded-full bg-card ring-4 ring-background">
          {event.harness ? <HarnessMark harness={event.harness} size={14} /> : <Icon className={cn("size-3.5", TONE_INK[event.tone])} />}
        </span>
        <div className="min-w-0 flex-1 pt-1">
          <div className="flex items-center gap-3">
            {onOpen ? (
              <Button variant="link" size="inline" onClick={onOpen} className="min-w-0 flex-1 justify-start">{title}</Button>
            ) : title}
            <time dateTime={event.at} className="w-10 shrink-0 text-right text-caption tabular-nums text-faint">{timeLabel(event.at)}</time>
            {/* Always the slot, so every time sits in one column; checks have no button in it. */}
            <span data-remove-slot="" className="flex size-6 shrink-0 items-center justify-center opacity-0 transition-opacity focus-within:opacity-100 group-hover:opacity-100">
              {onRemove && (
                <Button variant="quiet" size="icon-2xs" tooltip="Remove from trail"
                 
                  onClick={onRemove}
                  aria-label={`Remove this ${memberNoun(event.member)} from the trail`}
                 
                 
                >
                  <X className="size-3.5" aria-hidden="true" />
                </Button>
              )}
            </span>
          </div>
          <p data-event-meta="" className="mt-0.5 flex min-w-0 flex-wrap items-center gap-x-1.5 text-caption text-faint">
            <span className="font-medium text-muted-foreground">{KIND_LABEL[event.kind]}</span>
            {event.detail && <><span aria-hidden="true">·</span><span className={cn("min-w-0", TONE_INK[event.tone])}>{event.detail}</span></>}
            {auto && <><span aria-hidden="true">·</span><span>added automatically</span></>}
          </p>
          {event.excerpt && <p className="mt-2 rounded-lg bg-card px-3 py-2 text-ui leading-relaxed text-body">{event.excerpt}</p>}
        </div>
      </div>
    </li>
  );
}

export function TrailPage({ trail, summary: knownSummary, workspaceTitle, loading, refreshing = false, error, onBack, onContinue, onOpenEvent, onRemove, onRename, onArchive, onRefresh, onRetry, onAdd }: {
  trail: TrailDetail | null;
  /** What is known before the detail arrives (the list's row): the header paints from it. */
  summary?: TrailSummary | null;
  workspaceTitle?: string;
  loading: boolean;
  /** A GitHub re-read is in flight: Refresh spins and waits. */
  refreshing?: boolean;
  error?: string;
  onBack: () => void;
  onContinue: () => void;
  onOpenEvent: (event: TrailEvent) => void;
  onRemove: (event: TrailEvent) => void;
  onRename: (title: string) => Promise<void>;
  onArchive: (archived: boolean) => void;
  onRefresh: () => void;
  /** Read the trail again after its first read failed. */
  onRetry?: () => void;
  /** Open the Add picker: a new or existing note, a chat, a pull request, a draft or an issue. */
  onAdd: () => void;
}) {
  const [editing, setEditing] = useState(false);
  const [draftTitle, setDraftTitle] = useState("");
  // The brief answers for one trail; another trail's brief never shows here.
  const [brief, setBrief] = useState<{ trailId: string; result: PreviewTrailBriefResult | null } | null>(null);
  const summary = trail?.summary ?? knownSummary ?? null;
  const memberCount = trail?.summary.memberCount ?? 0;

  useEffect(() => {
    if (!trail) return;
    const trailId = trail.summary.id;
    let live = true;
    bridgeApi.previewTrailBrief(trailId)
      .then(result => { if (live) setBrief({ trailId, result }); })
      .catch(() => { if (live) setBrief({ trailId, result: null }); });
    return () => { live = false; };
  }, [trail?.summary.id, trail?.summary.updatedAt, memberCount]);

  const back = (
    <Button variant="text" size="inline" onClick={onBack}>
      <Route className="size-3.5" aria-hidden="true" />Trails
    </Button>
  );

  if (!summary) {
    return (
      <TrailScroll wide title="Trails" resetKey={null}>
        <div className="mx-auto w-full max-w-page-wide px-5 pb-12 pt-6 sm:px-8">
          <div className="mb-2 flex items-center gap-2 text-caption text-muted-foreground">{back}</div>
          {error ? <FirstReadError error={error} onRetry={onRetry} /> : loading ? (
            <>
              <div className="mb-6"><TrailHeaderSkeleton /></div>
              <div className="flex flex-col gap-8 lg:flex-row">
                <div className="min-w-0 flex-1"><TrailTimelineSkeleton /></div>
                <div className="w-full shrink-0 lg:w-72"><TrailRailSkeleton /></div>
              </div>
            </>
          ) : null}
        </div>
      </TrailScroll>
    );
  }

  const events = trail?.events ?? [];
  const newestPr = [...events].reverse().find(event => event.kind === "pullRequest");
  const chats = summary.harnesses.reduce((sum, item) => sum + item.chats, 0);
  const days = groupByDay(events);
  const shownBrief = trail && brief?.trailId === trail.summary.id ? brief : null;
  const chosenBrief = shownBrief?.result?.items.filter(item => item.defaultOn) ?? [];

  async function commitTitle() {
    const next = draftTitle.trim();
    setEditing(false);
    if (next && next !== summary?.title) await onRename(next);
  }

  // One count down the timeline, so its events cascade in order across days.
  let order = 0;

  return (
    <TrailScroll
      wide
      resetKey={summary.id}
      title={summary.title}
      action={<Button size="sm" onClick={onContinue}><Plus aria-hidden="true" />Continue</Button>}
    >
      <div className="mx-auto w-full max-w-page-wide px-5 pb-12 pt-6 sm:px-8">
        <header className="mb-7">
          <div className="mb-2 flex items-center gap-2 text-caption text-muted-foreground">
            {back}
            {workspaceTitle && <><span className="text-faint">/</span><span className="truncate">{workspaceTitle}</span></>}
          </div>
          <div className="flex flex-wrap items-center gap-x-3 gap-y-2">
            {editing ? (
              // oxlint-disable-next-line bridge/no-raw-controls -- an in-place heading editor keeps the heading's own type
              <input
                autoFocus
                value={draftTitle}
                maxLength={120}
                onChange={event => setDraftTitle(event.target.value)}
                onBlur={() => void commitTitle()}
                onKeyDown={event => { if (event.key === "Enter") void commitTitle(); if (event.key === "Escape") setEditing(false); }}
                aria-label="Trail title"
                className="min-w-0 flex-1 rounded-lg border border-input bg-background px-2 font-display text-title font-semibold tracking-tight text-foreground outline-none focus-visible:ring-2 focus-visible:ring-ring"
              />
            ) : (
              <h1 className="min-w-0 flex-1 text-balance font-display text-title font-semibold tracking-tight text-foreground">
                {/* oxlint-disable-next-line bridge/no-raw-controls -- the heading is its own rename control and keeps the heading's type */}
                <Tip label="Rename"><button type="button" onClick={() => { setDraftTitle(summary.title); setEditing(true); }} className="rounded-sm text-left focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring">{summary.title}</button></Tip>
              </h1>
            )}
            <div className="flex shrink-0 items-center gap-1">
              <Button variant="ghost" size="sm" onClick={onAdd} aria-haspopup="dialog" title="Add a note, chat, pull request, draft or issue"><Plus aria-hidden="true" />Add</Button>
              <Button
                variant="ghost"
                size="icon-sm"
                onClick={onRefresh}
                disabled={refreshing}
                aria-busy={refreshing}
                aria-label="Refresh from GitHub"
                title={refreshing ? "Refreshing from GitHub" : "Refresh from GitHub"}
              >
                <RefreshCw aria-hidden="true" className={cn(refreshing && "animate-spin")} />
              </Button>
              <Button variant="ghost" size="icon-sm" onClick={() => onArchive(!summary.archivedAt)} aria-label={summary.archivedAt ? "Unarchive trail" : "Archive trail"} title={summary.archivedAt ? "Unarchive" : "Archive"}>
                {summary.archivedAt ? <ArchiveRestore aria-hidden="true" /> : <Archive aria-hidden="true" />}
              </Button>
              <Button className="ml-1" onClick={onContinue}><Plus aria-hidden="true" />Continue this trail</Button>
            </div>
          </div>
          <div className="mt-3 flex flex-wrap items-center gap-2">
            <Chip tone={summary.state === "needsYou" ? "warning" : summary.state === "shipped" ? "success" : "plain"}>{TRAIL_STATE_LABEL[summary.state]}</Chip>
            {trail?.branch && <Chip title="Branch of the newest pull request"><GitBranch className="size-3" aria-hidden="true" /><span className="font-mono">{trail.branch}</span></Chip>}
            {newestPr && <Chip tone={newestPr.tone === "success" ? "success" : "plain"}><GitPullRequest className="size-3" aria-hidden="true" />{newestPr.title.split(" ")[0]} · {newestPr.detail}</Chip>}
            <Chip><MessagesSquare className="size-3" aria-hidden="true" />{chats} {chats === 1 ? "chat" : "chats"}</Chip>
            <span className="ml-auto flex items-center gap-2 text-caption text-faint"><StageBar stage={summary.stage} state={summary.state} />{stageLabel(summary.stage)}</span>
          </div>
          {summary.needsYouReason && (
            <p className="mt-4 flex items-center gap-2.5 rounded-lg bg-warning/10 px-3.5 py-2.5 text-ui text-foreground">
              <CircleAlert className="size-3.5 shrink-0 text-warning" aria-hidden="true" />
              <span className="flex-1">{summary.needsYouReason}</span>
            </p>
          )}
          {error && trail && <p role="alert" className="mt-3 text-caption text-destructive">{error}</p>}
        </header>

        <div className="flex flex-col gap-8 lg:flex-row">
          <div className="min-w-0 flex-1">
            {!trail && (error ? <FirstReadError error={error} onRetry={onRetry} /> : <TrailTimelineSkeleton />)}
            {trail && (
              <ol aria-label="Timeline">
                {events.length === 0 && (
                  <li className="rounded-xl bg-card px-4 py-6 text-ui text-muted-foreground">
                    Nothing on this trail yet. Add the issue or note it starts from, a chat, a pull request or a draft with Add, or start a chat with Continue this trail.
                  </li>
                )}
                {days.map((day, dayIndex) => (
                  <li key={day.key}>
                    <p className="trail-rise mb-3 pl-10 text-caption font-medium text-muted-foreground" style={riseDelay(order)}>{day.label}</p>
                    <ol>
                      {day.events.map((event, index) => (
                        <EventRow
                          key={event.id}
                          event={event}
                          last={dayIndex === days.length - 1 && index === day.events.length - 1}
                          order={order++}
                          onOpen={event.kind === "removed" ? undefined : () => onOpenEvent(event)}
                          onRemove={event.kind === "checks" ? undefined : () => onRemove(event)}
                        />
                      ))}
                    </ol>
                  </li>
                ))}
              </ol>
            )}
          </div>

          {/* The rail stays in view while a long timeline scrolls past it,
              clear of the condensed bar. */}
          <aside className="w-full shrink-0 lg:sticky lg:top-14 lg:w-72 lg:self-start">
            {!trail ? <TrailRailSkeleton /> : (
              <div className="trail-rise space-y-3" style={riseDelay(2)}>
                <RailSection title="A new chat starts with" aside={shownBrief?.result ? `${formatTokens(chosenBrief.reduce((sum, item) => sum + item.tokens, 0))} tokens` : undefined}>
                  {!shownBrief && <TrailBriefSkeleton />}
                  {shownBrief && !shownBrief.result && <p className="text-ui text-muted-foreground">Could not read the brief.</p>}
                  {shownBrief?.result && chosenBrief.length === 0 && <p className="text-ui text-muted-foreground">Nothing yet. A new chat joins the trail and starts fresh.</p>}
                  {chosenBrief.length > 0 && (
                    <ul className="space-y-2.5 text-ui leading-snug text-body">
                      {chosenBrief.map(item => {
                        const Icon = BRIEF_ICON[briefKind(item.id)];
                        return (
                          <li key={item.id} data-brief-kind={briefKind(item.id)} className="flex gap-2.5">
                            <Icon className="mt-0.5 size-3.5 shrink-0 text-faint" aria-hidden="true" />
                            <span className="min-w-0 flex-1">{item.label}</span>
                          </li>
                        );
                      })}
                    </ul>
                  )}
                </RailSection>
                <RailSection title="Open" aside={String(trail.open.length)}>
                  {trail.open.length === 0 ? <p className="text-ui text-muted-foreground">Nothing waits on you.</p> : (
                    <ul className="space-y-2 text-ui">
                      {trail.open.map(item => (
                        <li key={`${item.kind}:${item.label}`} className="flex items-center gap-2">
                          <span aria-hidden="true" className={cn("size-1.5 shrink-0 rounded-full", item.kind === "failingCheck" ? "bg-destructive" : "bg-warning")} />
                          {item.url
                            ? <a href={item.url} target="_blank" rel="noreferrer" className="min-w-0 flex-1 truncate text-body underline-offset-4 hover:underline">{item.label}</a>
                            : <span className="min-w-0 flex-1 truncate text-body">{item.label}</span>}
                        </li>
                      ))}
                    </ul>
                  )}
                </RailSection>
                {summary.harnesses.length > 0 && (
                  <RailSection title="Agents">
                    <div className="flex flex-wrap gap-1.5">
                      {summary.harnesses.map(item => (
                        <Chip key={item.harness}><HarnessMark harness={item.harness} size={12} />{harnessLabel(item.harness)} · {item.chats} {item.chats === 1 ? "chat" : "chats"}</Chip>
                      ))}
                    </div>
                  </RailSection>
                )}
              </div>
            )}
          </aside>
        </div>
      </div>
    </TrailScroll>
  );
}

/** The first read failed and nothing is on screen: say why, offer to read again. */
function FirstReadError({ error, onRetry }: { error: string; onRetry?: () => void }) {
  return (
    <div role="alert" className="flex flex-wrap items-center gap-3 rounded-xl bg-card px-4 py-3 text-ui">
      <CircleAlert className="size-3.5 shrink-0 text-destructive" aria-hidden="true" />
      <span className="min-w-0 flex-1 text-foreground">{error}</span>
      {onRetry && <Button size="sm" variant="ghost" onClick={onRetry}><RefreshCw aria-hidden="true" />Retry</Button>}
    </div>
  );
}
