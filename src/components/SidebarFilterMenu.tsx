import { useCallback, useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { Check, ChevronLeft, ChevronRight, SlidersHorizontal } from "lucide-react";
import { cn } from "@/lib/utils";
import {
  CHAT_GROUP_BY_LABELS,
  CHAT_SORT_BY_LABELS,
  CHAT_STATUS_LABELS,
  type ChatGroupBy,
  type ChatSortBy,
  type ChatStatusFilter,
  type ChatView,
} from "./sidebarChats";

type Panel = "root" | "status" | "agent" | "groupBy" | "sortBy";

type Option = { id: string; label: string };

const STATUS_OPTIONS: Option[] = (["all", "active", "waiting", "failed"] as ChatStatusFilter[]).map(id => ({ id, label: CHAT_STATUS_LABELS[id] }));
const GROUP_BY_OPTIONS: Option[] = (["date", "project", "agent", "status", "none"] as ChatGroupBy[]).map(id => ({ id, label: CHAT_GROUP_BY_LABELS[id] }));
const SORT_BY_OPTIONS: Option[] = (["recency", "name"] as ChatSortBy[]).map(id => ({ id, label: CHAT_SORT_BY_LABELS[id] }));

const ALL_LABEL = "All";

const MENU_WIDTH = 190;
/** Enough to keep the tallest panel (Group by, five options) on screen. */
const MENU_HEIGHT_ESTIMATE = 210;

export type SidebarFilterMenuProps = {
  view: ChatView;
  /** Harnesses present in the unfiltered list, for the Agent panel. */
  agents: Option[];
  onChange: (view: ChatView) => void;
};

export function SidebarFilterMenu({ view, agents, onChange }: SidebarFilterMenuProps) {
  const [open, setOpen] = useState(false);
  const [panel, setPanel] = useState<Panel>("root");
  const [anchor, setAnchor] = useState({ left: 0, top: 0 });
  const triggerRef = useRef<HTMLButtonElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);

  const close = useCallback(() => {
    setOpen(false);
    setPanel("root");
  }, []);

  const place = useCallback(() => {
    const rect = triggerRef.current?.getBoundingClientRect();
    if (!rect) return;
    setAnchor({
      left: Math.max(8, Math.min(rect.right - MENU_WIDTH, window.innerWidth - MENU_WIDTH - 8)),
      top: Math.min(rect.bottom + 6, Math.max(8, window.innerHeight - MENU_HEIGHT_ESTIMATE)),
    });
  }, []);

  useEffect(() => {
    if (!open) return;
    const onPointerDown = (event: PointerEvent) => {
      const target = event.target as Node;
      if (triggerRef.current?.contains(target) || menuRef.current?.contains(target)) return;
      close();
    };
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") close();
    };
    // The menu is a fixed layer so the history scroller cannot clip it, which
    // means it has to close rather than drift when anything scrolls or resizes.
    document.addEventListener("pointerdown", onPointerDown);
    document.addEventListener("keydown", onKeyDown);
    window.addEventListener("scroll", close, true);
    window.addEventListener("resize", close);
    return () => {
      document.removeEventListener("pointerdown", onPointerDown);
      document.removeEventListener("keydown", onKeyDown);
      window.removeEventListener("scroll", close, true);
      window.removeEventListener("resize", close);
    };
  }, [open, close]);

  const toggle = () => {
    if (open) {
      close();
      return;
    }
    place();
    setOpen(true);
  };

  const agentLabel = view.agent === "all"
    ? ALL_LABEL
    : agents.find(agent => agent.id === view.agent)?.label ?? view.agent;

  const rows: { panel: Panel; label: string; value: string; group: 1 | 2 }[] = [
    { panel: "status", label: "Status", value: CHAT_STATUS_LABELS[view.status], group: 1 },
    { panel: "agent", label: "Agent", value: agentLabel, group: 1 },
    { panel: "groupBy", label: "Group by", value: CHAT_GROUP_BY_LABELS[view.groupBy], group: 2 },
    { panel: "sortBy", label: "Sort by", value: CHAT_SORT_BY_LABELS[view.sortBy], group: 2 },
  ];

  const panels: Record<Exclude<Panel, "root">, { title: string; options: Option[]; current: string; apply: (id: string) => ChatView }> = {
    status: { title: "Status", options: STATUS_OPTIONS, current: view.status, apply: id => ({ ...view, status: id as ChatStatusFilter }) },
    agent: { title: "Agent", options: [{ id: "all", label: ALL_LABEL }, ...agents], current: view.agent, apply: id => ({ ...view, agent: id }) },
    groupBy: { title: "Group by", options: GROUP_BY_OPTIONS, current: view.groupBy, apply: id => ({ ...view, groupBy: id as ChatGroupBy }) },
    sortBy: { title: "Sort by", options: SORT_BY_OPTIONS, current: view.sortBy, apply: id => ({ ...view, sortBy: id as ChatSortBy }) },
  };

  const active = view.status !== "all" || view.agent !== "all";

  return (
    <>
      <button
        ref={triggerRef}
        type="button"
        onClick={toggle}
        aria-expanded={open}
        aria-haspopup="menu"
        aria-label="Filter and group chats"
        title="Filter and group chats"
        className={cn(
          "inline-flex h-6 w-6 items-center justify-center rounded-md transition-colors",
          open || active ? "bg-accent text-foreground" : "text-muted-foreground hover:bg-accent hover:text-foreground",
        )}
      >
        <SlidersHorizontal size={13} strokeWidth={1.7} aria-hidden="true" />
      </button>

      {open && createPortal(
        <div
          ref={menuRef}
          role="menu"
          aria-label="Chat list options"
          style={{ left: anchor.left, top: anchor.top, width: MENU_WIDTH }}
          className="fixed z-50 rounded-lg border border-border bg-popover p-1 text-popover-foreground shadow-lg"
        >
          {panel === "root" ? rows.map((row, index) => (
            <div key={row.panel}>
              {index > 0 && rows[index - 1].group !== row.group && <div className="my-1 h-px bg-border" />}
              <button
                type="button"
                role="menuitem"
                onClick={() => setPanel(row.panel)}
                className="flex h-7 w-full items-center gap-2 rounded-md px-2 text-left text-[12.5px] transition-colors hover:bg-accent"
              >
                <span className="min-w-0 flex-1 truncate">{row.label}</span>
                <span className="shrink-0 truncate text-[11px] text-muted-foreground">{row.value}</span>
                <ChevronRight size={12} strokeWidth={1.7} className="shrink-0 text-muted-foreground" aria-hidden="true" />
              </button>
            </div>
          )) : (
            <>
              <button
                type="button"
                onClick={() => setPanel("root")}
                className="flex h-7 w-full items-center gap-1.5 rounded-md px-1.5 text-left text-[11px] font-medium text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
              >
                <ChevronLeft size={12} strokeWidth={1.7} aria-hidden="true" />
                {panels[panel].title}
              </button>
              <div className="my-1 h-px bg-border" />
              {panels[panel].options.map(option => {
                const checked = option.id === panels[panel].current;
                return (
                  <button
                    key={option.id}
                    type="button"
                    role="menuitemradio"
                    aria-checked={checked}
                    onClick={() => {
                      onChange(panels[panel].apply(option.id));
                      setPanel("root");
                    }}
                    className="flex h-7 w-full items-center gap-2 rounded-md px-2 text-left text-[12.5px] transition-colors hover:bg-accent"
                  >
                    <Check size={12} strokeWidth={2.2} aria-hidden="true" className={cn("shrink-0", checked ? "opacity-100" : "opacity-0")} />
                    <span className="min-w-0 flex-1 truncate">{option.label}</span>
                  </button>
                );
              })}
            </>
          )}
        </div>,
        document.body,
      )}
    </>
  );
}
