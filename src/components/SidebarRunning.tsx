import { useEffect, useState } from "react";
import clsx from "clsx";
import { Activity, LoaderCircle } from "lucide-react";
import { HarnessMark } from "./harnessMarks";
import { chatName } from "./sidebarChats";
import { formatElapsed, runningDetail, type RunningChat } from "../sidebarRunning";
import { harnessLabel } from "../utils";

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

function RunningRow({ row, now, active, onOpen }: { row: RunningChat; now: number; active: boolean; onOpen: () => void }) {
  const name = chatName(row.chat);
  const detail = runningDetail(row);
  return (
    <button
      type="button"
      onClick={onOpen}
      data-running-row={row.chat.id}
      data-active={active || undefined}
      className={clsx(
        "flex h-11 w-full min-w-0 shrink-0 items-center gap-2 rounded-md pl-2 text-left font-sans transition-colors active:scale-[0.99]",
        active ? "bg-selection text-selection-foreground" : "hover:bg-accent",
      )}
    >
      <span className="flex min-w-0 flex-1 flex-col justify-center gap-0.5">
        {/* The hover names the chat in full on the name itself: the row's own
            title stays with the chat's row in its project folder. */}
        <span title={`${row.chat.titleLadder?.at(-1) ?? name} · ${harnessLabel(row.chat.harness)}`} className={clsx("min-w-0 truncate text-ui leading-4 tracking-[-0.008em] text-foreground", active && "font-medium")}>{name}</span>
        <span className="flex min-w-0 items-center gap-1.5 text-caption leading-3.5 tracking-[-0.004em] text-muted-foreground">
          <LoaderCircle size={10} strokeWidth={2} className="shrink-0 animate-spin text-info" aria-hidden="true" />
          <span data-running-elapsed className="shrink-0 tabular-nums text-info">{formatElapsed(now - row.since)}</span>
          <span aria-hidden="true">·</span>
          <span data-running-detail={detail.tone} className={clsx("min-w-0 truncate", detail.tone === "needsYou" && "text-warning")}>{detail.text}</span>
        </span>
      </span>
      <HarnessMark harness={row.chat.harness} size={13} className={clsx("mr-2 shrink-0", !active && "text-muted-foreground")} />
    </button>
  );
}

/**
 * The sidebar's Running group: every chat with a turn or a worker in flight,
 * across projects, above the project tree. A view, not a move: each chat keeps
 * its row in its project folder too. The host renders it only when something
 * runs, so an idle rail shows no empty heading.
 */
export function SidebarRunning({ rows, activeSessionId, onOpenSession }: {
  rows: readonly RunningChat[];
  activeSessionId?: string;
  onOpenSession: (id: string) => void;
}) {
  const now = useSecondClock();
  return (
    <section aria-label="Running" data-sidebar-running className="mb-3 shrink-0">
      <div className="flex h-7 items-center gap-1.5 px-2">
        <Activity size={13} strokeWidth={1.6} className="shrink-0 text-muted-foreground" aria-hidden="true" />
        <span className="text-meta font-medium tracking-[-0.004em] text-muted-foreground">Running</span>
        <span className="ml-auto text-caption tabular-nums text-muted-foreground/60">{rows.length}</span>
      </div>
      {/* Capped, so a busy afternoon cannot push the project tree off the rail. */}
      <div className="-mr-2 flex max-h-64 flex-col gap-0.5 overflow-y-auto pr-2">
        {rows.map(row => (
          <RunningRow
            key={row.chat.id}
            row={row}
            now={now}
            active={row.chat.id === activeSessionId}
            onOpen={() => onOpenSession(row.chat.id)}
          />
        ))}
      </div>
    </section>
  );
}
