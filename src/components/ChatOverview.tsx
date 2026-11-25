import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { CircleAlert, CircleCheck, CircleDashed, CircleX, Code2, FileDiff, FolderGit2, GitBranch } from "lucide-react";
import type { LucideIcon } from "lucide-react";
import { bridgeApi } from "../api";
import { chatConversationItems, chatOverview, formatTurnDuration, type ChatOverviewModel, type OverviewRow, type OverviewTone, type TurnChanges, type TurnHistoryEntry } from "../chatOverview";
import { dockPaneChord, type DockPaneId } from "../dockLayout";
import { formatShortcut } from "@/lib/platform";
import type { AgentEvent, BaseBranchDivergence, CompletionSummary, Session, SessionEntry, Workspace } from "../types";
import { HarnessMark } from "./harnessMarks";
import { Kbd } from "./ui/kbd";
import { Tip } from "@/components/ui/tooltip";
import { Button } from "@/components/ui/button";
import { RowButton } from "@/components/ui/row";

// The head of the dock's Work pane: where this chat runs, which branch, what
// it has not committed, how its last checks went, and what each turn did. It
// takes the place of a floating details card, so nothing covers the chat.
// Rows hide when Bridge holds no fact for them. There is no Commit or Open PR
// button: Bridge has no path that commits or opens a pull request for a chat's
// worktree, and a button that only pretends would be worse than none.

const TURNS_SHOWN = 5;

const ROW_LABEL: Record<OverviewRow["kind"], string> = { where: "Where", branch: "Branch", changes: "Changes", checks: "Checks" };
const ROW_ICON: Record<Exclude<OverviewRow["kind"], "checks">, LucideIcon> = { where: FolderGit2, branch: GitBranch, changes: FileDiff };
const CHECK_ICON: Record<OverviewTone, { icon: LucideIcon; className: string }> = {
  passed: { icon: CircleCheck, className: "text-success" },
  failed: { icon: CircleX, className: "text-destructive" },
  waived: { icon: CircleAlert, className: "text-warning" },
  pending: { icon: CircleDashed, className: "text-muted-foreground" },
};

function paneChord(pane: DockPaneId): string | undefined {
  const digit = dockPaneChord(pane);
  return digit ? formatShortcut({ meta: true, alt: true }, digit) : undefined;
}

function Counts({ additions, deletions }: { additions?: number; deletions?: number }) {
  if (additions === undefined && deletions === undefined) return null;
  return <span className="inline-flex gap-1.5 font-mono text-caption tabular-nums">
    {additions !== undefined && <span className="text-success">+{additions}</span>}
    {deletions !== undefined && <span className="text-destructive">−{deletions}</span>}
  </span>;
}

function fileCount(files: number): string {
  return `${files} ${files === 1 ? "file" : "files"}`;
}

function PaneButton({ label, pane, ariaLabel, icon: Icon, onClick }: { label: string; pane: DockPaneId; ariaLabel: string; icon?: LucideIcon; onClick: () => void }) {
  const chord = paneChord(pane);
  return <Button variant="outline" size="2xs" onClick={onClick} aria-label={ariaLabel} tooltip={chord ? `${ariaLabel}  ${chord}` : ariaLabel}>
    {Icon && <Icon size={12} aria-hidden="true" />}
    {label}
    {chord && <Kbd>{chord}</Kbd>}
  </Button>;
}

function Row({ row, onReview, onOpenCode }: { row: OverviewRow; onReview?: () => void; onOpenCode?: () => void }) {
  let icon: ReactNode;
  let main: ReactNode;
  let title: string | undefined;
  let detail: string | undefined;
  let action: ReactNode = null;
  switch (row.kind) {
    case "where": {
      const Icon = ROW_ICON.where;
      icon = <Icon size={14} aria-hidden="true" />;
      main = row.display;
      title = row.path;
      detail = row.detail;
      if (onOpenCode) action = <PaneButton label="Code" pane="code" ariaLabel="Open in the Code pane" icon={Code2} onClick={onOpenCode} />;
      break;
    }
    case "branch": {
      const Icon = ROW_ICON.branch;
      icon = <Icon size={14} aria-hidden="true" />;
      main = row.name;
      title = row.name;
      detail = row.detail;
      break;
    }
    case "changes": {
      const Icon = ROW_ICON.changes;
      icon = <Icon size={14} aria-hidden="true" />;
      main = row.files > 0
        ? <span className="inline-flex items-center gap-2">{fileCount(row.files)}<Counts additions={row.additions} deletions={row.deletions} /></span>
        : "No uncommitted changes";
      detail = row.files > 0 ? "uncommitted" : undefined;
      if (row.files > 0 && onReview) action = <PaneButton label="Review" pane="changes" ariaLabel={`Review ${fileCount(row.files)}`} onClick={onReview} />;
      break;
    }
    case "checks": {
      const { icon: Icon, className } = CHECK_ICON[row.tone];
      icon = <Icon size={14} className={className} aria-hidden="true" />;
      main = row.label;
      title = row.label;
      detail = row.detail;
      break;
    }
  }
  return <div data-overview-row={row.kind} className="flex min-h-10 items-center gap-2.5 px-3 py-1.5">
    <span className="grid w-3.5 shrink-0 place-items-center text-muted-foreground">{icon}</span>
    <span className="w-14 shrink-0 text-meta text-muted-foreground">{ROW_LABEL[row.kind]}</span>
    <span className="min-w-0 flex-1">
      <Tip label={title}><span className="block truncate text-meta text-foreground">{main}</span></Tip>
      {detail && <Tip label={detail}><span className="block truncate text-caption text-faint">{detail}</span></Tip>}
    </span>
    {action}
  </div>;
}

function TurnOutcome({ changes }: { changes: TurnChanges | null }) {
  if (!changes) return <span className="shrink-0 text-faint">no changes</span>;
  return <span className="inline-flex shrink-0 items-center gap-2">{fileCount(changes.files)}<Counts additions={changes.additions} deletions={changes.deletions} /></span>;
}

function TurnRow({ turn, harness, onOpen }: { turn: TurnHistoryEntry; harness?: string | null; onOpen?: (entryId: string) => void }) {
  const body = <>
    <HarnessMark harness={turn.harness ?? harness} size={12} />
    <Tip label={turn.title}><span className="min-w-0 truncate text-foreground">{turn.title}</span></Tip>
    {turn.durationMs !== undefined && <span className="shrink-0 tabular-nums text-faint">{formatTurnDuration(turn.durationMs)}</span>}
    <span className="ml-auto pl-2" />
    {turn.live ? <span className="shrink-0 text-info">Working</span> : <TurnOutcome changes={turn.changes} />}
  </>;
  const row = "flex h-8 w-full min-w-0 items-center gap-2 rounded-md px-2 text-left text-meta text-muted-foreground";
  if (onOpen && turn.entryId) {
    const entryId = turn.entryId;
    return <li data-overview-turn={turn.turn} className="min-w-0">
      <RowButton size="bare" onClick={() => onOpen(entryId)} aria-label={`Show turn: ${turn.title}`}><span className={row}>{body}</span></RowButton>
    </li>;
  }
  return <li data-overview-turn={turn.turn} className={row}>{body}</li>;
}

export type ChatOverviewViewProps = {
  model: ChatOverviewModel;
  /** The chat's harness, for turns whose rows carry none. */
  harness?: string | null;
  /** Opens the Changes pane. Absent when this chat has no Changes pane to open. */
  onReview?: () => void;
  /** Opens the Code pane, Bridge's editor. */
  onOpenCode?: () => void;
  /** Scrolls the conversation to the prompt that opened a turn. */
  onOpenTurn?: (entryId: string) => void;
};

export function ChatOverviewView({ model, harness, onReview, onOpenCode, onOpenTurn }: ChatOverviewViewProps) {
  const [allTurns, setAllTurns] = useState(false);
  const turns = allTurns ? model.turns : model.turns.slice(0, TURNS_SHOWN);
  const hidden = model.turns.length - turns.length;
  return <section aria-label="This chat" data-chat-overview className="pb-1 pt-4">
    <div className="px-2">
      <p className="text-meta font-medium text-muted-foreground">This chat</p>
      {model.headline && <h2 className="mt-0.5 font-display text-heading text-foreground">{model.headline}</h2>}
    </div>
    {model.rows.length > 0 && <div className="mt-3 divide-y divide-border overflow-hidden rounded-lg border border-border bg-card">
      {model.rows.map(row => <Row key={row.kind} row={row} onReview={onReview} onOpenCode={onOpenCode} />)}
    </div>}
    {model.turns.length > 0 && <div className="mt-4">
      <h3 className="mb-1 flex items-center gap-2 px-2 text-caption font-medium text-faint">
        Turn history<span className="ml-auto tabular-nums">{model.turns.length} {model.turns.length === 1 ? "turn" : "turns"}</span>
      </h3>
      <ul aria-label="Turn history" className="grid grid-cols-1 gap-px">
        {turns.map(turn => <TurnRow key={turn.key} turn={turn} harness={harness} onOpen={onOpenTurn} />)}
      </ul>
      {(hidden > 0 || allTurns) && model.turns.length > TURNS_SHOWN && <Button variant="quiet" size="xs" className="mt-0.5"
        onClick={() => setAllTurns(value => !value)}
      >{allTurns ? "Show fewer" : `Show ${hidden} more`}</Button>}
    </div>}
  </section>;
}

export type ChatOverviewProps = {
  session: Session;
  /** The chat's workspace when it has a repository; absent for a chat with none. */
  workspace?: Workspace | null;
  projectName?: string;
  entries?: SessionEntry[];
  activeLeafId?: string | null;
  events: AgentEvent[];
  completion?: CompletionSummary | null;
  turnActive: boolean;
  /** The pane is on screen. Hidden, it keeps its last rows rather than re-reading every frame. */
  visible: boolean;
  onOpenPane: (pane: DockPaneId) => void;
  onOpenTurn?: (entryId: string) => void;
};

/** The overview for the open chat, reading the facts the app already holds. */
export function ChatOverview({ session, workspace, projectName, entries, activeLeafId = null, events, completion, turnActive, visible, onOpenPane, onOpenTurn }: ChatOverviewProps) {
  // Ahead/behind is a read of the base branch with no fetch: no network, and
  // only while the pane is open. It is read again when the tree changes and
  // when a turn settles, which is when a commit could have moved it.
  const [divergence, setDivergence] = useState<BaseBranchDivergence | null>(null);
  const stats = workspace ? `${workspace.dirtyFiles}:${workspace.additions}:${workspace.deletions}` : "";
  const sessionId = session.id;
  const hasRepo = !!workspace;
  useEffect(() => {
    if (!visible || !hasRepo) return;
    let cancelled = false;
    bridgeApi.workspaceBaseDivergence(sessionId, false)
      .then(next => { if (!cancelled) setDivergence(next); })
      .catch(() => { if (!cancelled) setDivergence(null); });
    return () => { cancelled = true; };
  }, [visible, hasRepo, sessionId, stats, turnActive]);

  // A hidden pane keeps what it last showed: replaying the chat on every
  // streamed frame nobody can see is work for nothing.
  const lastItems = useRef<ReturnType<typeof chatConversationItems>>([]);
  const items = useMemo(
    () => visible ? chatConversationItems(entries, activeLeafId, events) : lastItems.current,
    [visible, entries, activeLeafId, events],
  );
  lastItems.current = items;

  const model = useMemo(() => chatOverview({
    path: session.cwd || workspace?.path,
    projectName,
    branch: workspace?.branch,
    divergence: hasRepo ? divergence : null,
    changes: workspace ? { files: workspace.dirtyFiles, additions: workspace.additions, deletions: workspace.deletions } : null,
    completion,
    items,
    turnActive,
  }), [session.cwd, workspace, projectName, hasRepo, divergence, completion, items, turnActive]);

  return <ChatOverviewView
    model={model}
    harness={session.harness}
    onReview={hasRepo ? () => onOpenPane("changes") : undefined}
    onOpenCode={hasRepo ? () => onOpenPane("code") : undefined}
    onOpenTurn={onOpenTurn}
  />;
}
