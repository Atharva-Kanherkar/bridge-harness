import { useEffect, useState } from "react";
import { Check, ChevronLeft, ChevronRight, SlidersHorizontal } from "lucide-react";
import { cn } from "@/lib/utils";
import { MenuPanel, MenuSeparator, useMenuPanel } from "@/components/ui/menu-panel";
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

function groupByOptions(allowProject: boolean): Option[] {
  const ids: ChatGroupBy[] = allowProject
    ? ["date", "project", "agent", "status", "none"]
    // Nothing in Home has a project, so grouping by one would offer a single
    // "No project" bucket and call it a grouping.
    : ["date", "agent", "status", "none"];
  return ids.map(id => ({ id, label: CHAT_GROUP_BY_LABELS[id] }));
}
const SORT_BY_OPTIONS: Option[] = (["recency", "name"] as ChatSortBy[]).map(id => ({ id, label: CHAT_SORT_BY_LABELS[id] }));

const ALL_LABEL = "All";

const MENU_WIDTH = 190;
/** Enough to keep the tallest panel (Group by, five options) on screen. */
const MENU_HEIGHT_ESTIMATE = 210;

export type SidebarFilterMenuProps = {
  view: ChatView;
  /** Harnesses present in the unfiltered list, for the Agent panel. */
  agents: Option[];
  /** False in Home, where no chat has a project to group by. */
  allowProjectGrouping: boolean;
  onChange: (view: ChatView) => void;
};

export function SidebarFilterMenu({ view, agents, allowProjectGrouping, onChange }: SidebarFilterMenuProps) {
  const menu = useMenuPanel<HTMLButtonElement>({ width: MENU_WIDTH, height: MENU_HEIGHT_ESTIMATE });
  const [panel, setPanel] = useState<Panel>("root");

  // Reopening always starts at the top level rather than wherever it was left.
  useEffect(() => {
    if (!menu.open) setPanel("root");
  }, [menu.open]);

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
    groupBy: { title: "Group by", options: groupByOptions(allowProjectGrouping), current: view.groupBy, apply: id => ({ ...view, groupBy: id as ChatGroupBy }) },
    sortBy: { title: "Sort by", options: SORT_BY_OPTIONS, current: view.sortBy, apply: id => ({ ...view, sortBy: id as ChatSortBy }) },
  };

  const narrowing = view.status !== "all" || view.agent !== "all";

  return (
    <>
      <button
        ref={menu.triggerRef}
        type="button"
        onClick={menu.toggle}
        aria-expanded={menu.open}
        aria-haspopup="menu"
        aria-label="Filter and group chats"
        title="Filter and group chats"
        className={cn(
          "inline-flex h-6 w-6 items-center justify-center rounded-md transition-colors",
          menu.open || narrowing ? "bg-accent text-foreground" : "text-muted-foreground hover:bg-accent hover:text-foreground",
        )}
      >
        <SlidersHorizontal size={13} strokeWidth={1.7} aria-hidden="true" />
      </button>

      <MenuPanel controller={menu} label="Chat list options">
        {panel === "root" ? rows.map((row, index) => (
          <div key={row.panel}>
            {index > 0 && rows[index - 1].group !== row.group && <MenuSeparator />}
            <button
              type="button"
              role="menuitem"
              onClick={() => setPanel(row.panel)}
              className="flex h-7 w-full items-center gap-2 rounded-md px-2 text-left text-[13px] transition-colors hover:bg-accent"
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
            <MenuSeparator />
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
                  className="flex h-7 w-full items-center gap-2 rounded-md px-2 text-left text-[13px] transition-colors hover:bg-accent"
                >
                  <Check size={12} strokeWidth={2.2} aria-hidden="true" className={cn("shrink-0", checked ? "opacity-100" : "opacity-0")} />
                  <span className="min-w-0 flex-1 truncate">{option.label}</span>
                </button>
              );
            })}
          </>
        )}
      </MenuPanel>
    </>
  );
}
