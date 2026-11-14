import { useEffect, useState, type ComponentType } from "react";
import {
  Archive, ArchiveRestore, ChevronLeft, CircleAlert, FileText, GitBranch, GitPullRequest, ListChecks, MessageSquare,
  MessagesSquare, Plus, RefreshCw, Route, Send, Trash2, X,
} from "lucide-react";
import type { PreviewTrailBriefResult, TrailDetail, TrailEvent, TrailEventKind } from "@/protocol/generated/protocol";
import { bridgeApi } from "@/api";
import { TRAIL_STATE_LABEL, formatTokens, groupByDay, memberNoun, stageLabel, timeLabel } from "@/trails";
import { HarnessMark } from "@/components/harnessMarks";
import { harnessLabel } from "@/utils";
import { Button } from "@/components/ui/button";
import { SkeletonText } from "@/components/ui/skeleton";
import { Chip, RailSection, StageBar } from "./TrailParts";
import { clsx as cn } from "clsx";

const KIND_LABEL: Record<TrailEventKind, string> = {
  note: "Note",
  chat: "Chat",
  pullRequest: "Pull request",
  checks: "Checks",
  message: "Message",
  removed: "Removed",
};

const KIND_ICON: Record<TrailEventKind, ComponentType<{ className?: string }>> = {
  note: FileText,
  chat: MessagesSquare,
  pullRequest: GitPullRequest,
  checks: ListChecks,
  message: Send,
  removed: Trash2,
};

const TONE_INK = { plain: "text-muted-foreground", success: "text-success", warning: "text-warning", danger: "text-destructive" } as const;

function EventRow({ event, last, onOpen, onRemove }: { event: TrailEvent; last: boolean; onOpen?: () => void; onRemove?: () => void }) {
  const Icon = KIND_ICON[event.kind];
  const title = <span className="min-w-0 flex-1 truncate text-ui font-medium text-foreground">{event.title}</span>;
  return (
    <li className="group relative flex gap-3 pb-5" data-event-kind={event.kind}>
      {!last && <span aria-hidden="true" className="absolute left-3.5 top-7 h-full w-px bg-border" />}
      <span className="relative z-10 flex size-7 shrink-0 items-center justify-center rounded-full bg-card ring-4 ring-background">
        {event.harness ? <HarnessMark harness={event.harness} size={14} /> : <Icon className={cn("size-3.5", TONE_INK[event.tone])} />}
      </span>
      <div className="min-w-0 flex-1 pt-0.5">
        <div className="flex items-baseline gap-2">
          <span className="w-24 shrink-0 text-caption uppercase tracking-wide text-faint">{KIND_LABEL[event.kind]}</span>
          {onOpen ? (
            <button type="button" onClick={onOpen} className="flex min-w-0 flex-1 text-left hover:underline focus-visible:underline focus-visible:outline-none">{title}</button>
          ) : title}
          <span className="shrink-0 font-mono text-caption text-faint">{timeLabel(event.at)}</span>
          {onRemove && (
            <button
              type="button"
              onClick={onRemove}
              aria-label={`Remove this ${memberNoun(event.member)} from the trail`}
              title="Remove from trail"
              className="shrink-0 rounded p-0.5 text-faint opacity-0 transition-opacity hover:text-foreground focus-visible:opacity-100 group-hover:opacity-100"
            >
              <X className="size-3.5" aria-hidden="true" />
            </button>
          )}
        </div>
        {(event.detail || event.addedBy === "auto") && (
          <div className={cn("mt-0.5 flex flex-wrap items-center gap-x-2 text-caption", TONE_INK[event.tone === "plain" ? "plain" : event.tone])}>
            {event.detail && <span>{event.detail}</span>}
            {event.addedBy === "auto" && event.kind !== "checks" && <span className="text-faint">· added automatically</span>}
          </div>
        )}
        {event.excerpt && <p className="mt-2 rounded-lg bg-card px-3 py-2 text-ui leading-relaxed text-body">{event.excerpt}</p>}
      </div>
    </li>
  );
}

export function TrailPage({ trail, workspaceTitle, loading, error, onBack, onContinue, onOpenEvent, onRemove, onRename, onArchive, onRefresh }: {
  trail: TrailDetail | null;
  workspaceTitle?: string;
  loading: boolean;
  error?: string;
  onBack: () => void;
  onContinue: () => void;
  onOpenEvent: (event: TrailEvent) => void;
  onRemove: (event: TrailEvent) => void;
  onRename: (title: string) => Promise<void>;
  onArchive: (archived: boolean) => void;
  onRefresh: () => void;
}) {
  const [editing, setEditing] = useState(false);
  const [draftTitle, setDraftTitle] = useState("");
  const [brief, setBrief] = useState<PreviewTrailBriefResult | null>(null);
  const memberCount = trail?.summary.memberCount ?? 0;

  useEffect(() => {
    if (!trail) return;
    let live = true;
    bridgeApi.previewTrailBrief(trail.summary.id).then(result => { if (live) setBrief(result); }).catch(() => { if (live) setBrief(null); });
    return () => { live = false; };
  }, [trail?.summary.id, trail?.summary.updatedAt, memberCount]);

  if (!trail) {
    return (
      <div className="mx-auto w-full max-w-page-wide px-5 pb-12 pt-6 sm:px-8">
        <button type="button" onClick={onBack} className="mb-4 inline-flex items-center gap-1 text-caption text-muted-foreground hover:text-foreground"><ChevronLeft className="size-3.5" aria-hidden="true" />Trails</button>
        {error ? <p role="alert" className="text-ui text-destructive">{error}</p> : loading ? <SkeletonText lines={6} /> : null}
      </div>
    );
  }

  const { summary } = trail;
  const newestPr = [...trail.events].reverse().find(event => event.kind === "pullRequest");
  const chats = summary.harnesses.reduce((sum, item) => sum + item.chats, 0);
  const days = groupByDay(trail.events);
  const chosenBrief = brief?.items.filter(item => item.defaultOn) ?? [];

  async function commitTitle() {
    const next = draftTitle.trim();
    setEditing(false);
    if (next && next !== summary.title) await onRename(next);
  }

  return (
    <div className="mx-auto w-full max-w-page-wide px-5 pb-12 pt-6 sm:px-8">
      <header className="mb-6">
        <div className="mb-2 flex items-center gap-2 text-caption text-muted-foreground">
          <button type="button" onClick={onBack} className="inline-flex items-center gap-1 hover:text-foreground"><Route className="size-3.5" aria-hidden="true" />Trails</button>
          {workspaceTitle && <><span className="text-faint">/</span><span className="truncate">{workspaceTitle}</span></>}
        </div>
        <div className="flex flex-wrap items-start gap-3">
          {editing ? (
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
            <h1 className="min-w-0 flex-1 font-display text-title font-semibold tracking-tight text-foreground">
              <button type="button" onClick={() => { setDraftTitle(summary.title); setEditing(true); }} title="Rename" className="text-left">{summary.title}</button>
            </h1>
          )}
          <Button variant="ghost" size="icon-sm" onClick={onRefresh} aria-label="Refresh from GitHub" title="Refresh from GitHub"><RefreshCw aria-hidden="true" /></Button>
          <Button variant="ghost" size="icon-sm" onClick={() => onArchive(!summary.archivedAt)} aria-label={summary.archivedAt ? "Unarchive trail" : "Archive trail"} title={summary.archivedAt ? "Unarchive" : "Archive"}>
            {summary.archivedAt ? <ArchiveRestore aria-hidden="true" /> : <Archive aria-hidden="true" />}
          </Button>
          <Button onClick={onContinue}><Plus aria-hidden="true" />Continue this trail</Button>
        </div>
        <div className="mt-3 flex flex-wrap items-center gap-2">
          <Chip tone={summary.state === "needsYou" ? "warning" : summary.state === "shipped" ? "success" : "plain"}>{TRAIL_STATE_LABEL[summary.state]}</Chip>
          {trail.branch && <Chip title="Branch of the newest pull request"><GitBranch className="size-3" aria-hidden="true" /><span className="font-mono">{trail.branch}</span></Chip>}
          {newestPr && <Chip tone={newestPr.tone === "success" ? "success" : "plain"}><GitPullRequest className="size-3" aria-hidden="true" />{newestPr.title.split(" ")[0]} · {newestPr.detail}</Chip>}
          <Chip><MessagesSquare className="size-3" aria-hidden="true" />{chats} {chats === 1 ? "chat" : "chats"}</Chip>
          <span className="ml-auto flex items-center gap-2 text-caption text-faint"><StageBar stage={summary.stage} state={summary.state} />{stageLabel(summary.stage)}</span>
        </div>
        {summary.needsYouReason && (
          <p className="mt-3 flex items-center gap-2 rounded-lg bg-warning/10 px-3 py-2 text-ui text-foreground">
            <CircleAlert className="size-3.5 shrink-0 text-warning" aria-hidden="true" />
            <span className="flex-1">{summary.needsYouReason}</span>
          </p>
        )}
        {error && <p role="alert" className="mt-3 text-caption text-destructive">{error}</p>}
      </header>

      <div className="flex flex-col gap-8 lg:flex-row">
        <ol className="min-w-0 flex-1" aria-label="Timeline">
          {trail.events.length === 0 && (
            <li className="rounded-xl bg-card px-4 py-6 text-ui text-muted-foreground">
              Nothing on this trail yet. Add a chat from its menu (Add to trail), or start one with Continue this trail.
            </li>
          )}
          {days.map((day, dayIndex) => (
            <li key={day.key}>
              <p className="mb-3 pl-10 text-caption font-medium text-muted-foreground">{day.label}</p>
              <ol>
                {day.events.map((event, index) => (
                  <EventRow
                    key={event.id}
                    event={event}
                    last={dayIndex === days.length - 1 && index === day.events.length - 1}
                    onOpen={event.kind === "removed" ? undefined : () => onOpenEvent(event)}
                    onRemove={event.kind === "checks" ? undefined : () => onRemove(event)}
                  />
                ))}
              </ol>
            </li>
          ))}
        </ol>

        <aside className="w-full shrink-0 space-y-3 lg:w-72">
          <RailSection title="A new chat starts with" aside={brief ? `${formatTokens(chosenBrief.reduce((sum, item) => sum + item.tokens, 0))} tokens` : undefined}>
            {!brief && <SkeletonText lines={3} />}
            {brief && chosenBrief.length === 0 && <p className="text-ui text-muted-foreground">Nothing yet. A new chat joins the trail and starts fresh.</p>}
            {brief && chosenBrief.length > 0 && (
              <ul className="space-y-2 text-ui text-body">
                {chosenBrief.map(item => (
                  <li key={item.id} className="flex gap-2">
                    <MessageSquare className="mt-1 size-3 shrink-0 text-muted-foreground" aria-hidden="true" />
                    <span className="min-w-0 flex-1">{item.label}</span>
                  </li>
                ))}
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
                      ? <a href={item.url} target="_blank" rel="noreferrer" className="min-w-0 flex-1 truncate text-body hover:underline">{item.label}</a>
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
        </aside>
      </div>
    </div>
  );
}
