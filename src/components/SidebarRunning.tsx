import { useEffect, useState } from "react";
import clsx from "clsx";
import { ChevronRight, LayoutGrid } from "lucide-react";
import { HarnessMark } from "./harnessMarks";
import { chatName } from "./sidebarChats";
import { formatElapsed, runningDetail, type RunningChat } from "../sidebarRunning";
import { harnessLabel } from "../utils";
import type { TrailSummary } from "../protocol/generated/protocol";
import { SidebarTrailButton } from "./trails/SidebarTrailButton";

/**
 * One clock for the whole group, ticking once a second: every row's timer
 * reads it, so ten running chats cost one interval, not ten. A hidden window's
 * timers are throttled, so it also catches up the moment the window shows.
 */
function useSecondClock(): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const tick = () => setNow(Date.now());
    const timer = window.setInterval(tick, 1000);
    document.addEventListener("visibilitychange", tick);
    window.addEventListener("focus", tick);
    return () => {
      window.clearInterval(timer);
      document.removeEventListener("visibilitychange", tick);
      window.removeEventListener("focus", tick);
    };
  }, []);
  return now;
}

function RunningRow({ row, now, active, onOpen, trail, onOpenTrail }: {
  row: RunningChat;
  now: number;
  active: boolean;
  onOpen: () => void;
  trail?: TrailSummary;
  onOpenTrail?: (trailId: string) => void;
}) {
  const name = chatName(row.chat);
  const detail = runningDetail(row);
  return (
    <div
      data-running-row-frame={row.chat.id}
      data-active={active || undefined}
      className={clsx(
        "group/row flex shrink-0 items-center rounded-md transition-colors",
        active ? "bg-selection text-selection-foreground" : "hover:bg-accent",
      )}
    >
      <button
        type="button"
        onClick={onOpen}
        data-running-row={row.chat.id}
        data-active={active || undefined}
        className="flex h-11 min-w-0 flex-1 items-center gap-2 rounded-md pl-2 text-left font-sans transition-transform active:scale-[0.99]"
      >
        <span className="flex min-w-0 flex-1 flex-col justify-center gap-0.5">
          {/* The hover names the chat in full on the name itself: the row's own
              title stays with the chat's row in its project folder. */}
          <span title={`${row.chat.titleLadder?.at(-1) ?? name} · ${harnessLabel(row.chat.harness)}`} className={clsx("min-w-0 truncate text-ui leading-4 tracking-snug text-foreground", active && "font-medium")}>{name}</span>
          <span className="flex min-w-0 items-center gap-1.5 text-caption leading-3.5 tracking-snug text-muted-foreground">
            <span data-active-dot aria-hidden="true" className="size-1.5 shrink-0 rounded-full bg-info motion-safe:animate-pulse" />
            <span data-running-elapsed className="shrink-0 tabular-nums text-info">{formatElapsed(now - row.since)}</span>
            <span aria-hidden="true">·</span>
            <span data-running-detail={detail.tone} className={clsx("min-w-0 truncate", detail.tone === "needsYou" && "text-warning")}>{detail.text}</span>
          </span>
        </span>
      </button>
      {trail && onOpenTrail && <SidebarTrailButton trail={trail} onOpenTrail={onOpenTrail} />}
      <HarnessMark harness={row.chat.harness} size={13} className={clsx("mx-2 shrink-0", !active && "text-muted-foreground")} />
    </div>
  );
}

const COLLAPSED_KEY = "bridge.sidebar.active.collapsed";

/**
 * The sidebar's Active group: every chat with a turn or a worker in flight,
 * across projects, above the project tree. A view, not a move: each chat keeps
 * its row in its project folder too. The host renders it only when something
 * runs, so an idle rail shows no empty heading.
 *
 * It is also the way into Mission Control, the same chats on one board: the
 * grid button beside the heading opens it. Folded, the group is its heading
 * and a count; the choice holds across launches.
 */
export function SidebarRunning({ rows, activeSessionId, onOpenSession, onOpenMissionControl, missionControlActive = false, trailBySession, onOpenTrail }: {
  rows: readonly RunningChat[];
  activeSessionId?: string;
  onOpenSession: (id: string) => void;
  /** Each chat's trail: a running chat on one shows the trail button on hover. */
  trailBySession?: ReadonlyMap<string, TrailSummary>;
  onOpenTrail?: (trailId: string) => void;
  onOpenMissionControl?: () => void;
  missionControlActive?: boolean;
}) {
  const now = useSecondClock();
  const [collapsed, setCollapsed] = useState(() => localStorage.getItem(COLLAPSED_KEY) === "1");
  const toggle = () => setCollapsed(value => {
    localStorage.setItem(COLLAPSED_KEY, value ? "0" : "1");
    return !value;
  });
  const listId = "sidebar-active-list";
  return (
    <section aria-label="Active" data-sidebar-running data-collapsed={collapsed || undefined} className="mb-3 shrink-0">
      <div className="flex h-7 items-center gap-1 pr-1">
        <button
          type="button"
          onClick={toggle}
          aria-expanded={!collapsed}
          aria-controls={listId}
          title={collapsed ? "Show active chats" : "Hide active chats"}
          data-active-toggle
          className="flex h-7 min-w-0 flex-1 items-center gap-1.5 rounded-md px-2 text-left transition-colors hover:bg-accent"
        >
          {/* A live dot, not a spinner: the group should read as calm presence. */}
          <span aria-hidden="true" className="relative grid size-3.25 shrink-0 place-items-center">
            <span className="absolute size-1.5 rounded-full bg-info/40 motion-safe:animate-ping" />
            <span className="relative size-1.5 rounded-full bg-info" />
          </span>
          <span className="text-meta font-medium tracking-snug text-muted-foreground">Active</span>
          <span data-active-count className="text-caption tabular-nums text-muted-foreground/60">{rows.length}</span>
          <ChevronRight size={12} strokeWidth={1.7} aria-hidden="true" className={clsx("ml-auto shrink-0 text-muted-foreground/70 transition-transform", !collapsed && "rotate-90")} />
        </button>
        {onOpenMissionControl && (
          <button
            type="button"
            onClick={onOpenMissionControl}
            title="Open Mission Control"
            aria-label="Open Mission Control"
            aria-current={missionControlActive ? "page" : undefined}
            data-open-mission-control
            className={clsx(
              "grid size-6 shrink-0 place-items-center rounded-md transition-colors",
              missionControlActive ? "bg-selection text-selection-foreground" : "text-muted-foreground hover:bg-accent hover:text-foreground",
            )}
          >
            <LayoutGrid size={13} strokeWidth={1.6} aria-hidden="true" />
          </button>
        )}
      </div>
      {/* Capped, so a busy afternoon cannot push the project tree off the rail. */}
      {!collapsed && (
        <div id={listId} className="-mr-2 flex max-h-64 flex-col gap-0.5 overflow-y-auto pr-2">
          {rows.map(row => (
            <RunningRow
              key={row.chat.id}
              row={row}
              now={now}
              active={row.chat.id === activeSessionId}
              onOpen={() => onOpenSession(row.chat.id)}
              trail={trailBySession?.get(row.chat.id)}
              onOpenTrail={onOpenTrail}
            />
          ))}
        </div>
      )}
    </section>
  );
}
