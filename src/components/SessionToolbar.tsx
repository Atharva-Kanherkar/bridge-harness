import type { ReactNode } from "react";
import { Check, Maximize2, Minimize2, Monitor, MoreHorizontal, PanelLeft, PanelRight, Search, Settings2, Square } from "lucide-react";
import { cn } from "@/lib/utils";
import { MenuItem, MenuPanel, MenuSeparator, useMenuPanel } from "@/components/ui/menu-panel";
import type { DockPaneId } from "../dockLayout";
import type { DockPaneDescriptor } from "./SessionDock";

// One strip for the whole session: what it is, where its panes are, and the
// window actions. The dock's collapsed rail is gone, so this row is now the
// only way into a pane while the dock is shut — GitHub keeps a button of its
// own and the rest fold into the overflow menu.
//
// This is also the *only* chrome row on a session view — AppTitleBar steps
// aside there — so it carries the app-title-bar responsibilities it absorbed:
// the mobile nav toggle and the bypass actions that used to live in a second
// strip above it.

export type SessionToolbarProps = {
  title: string;
  /** The workspace/project this session belongs to. Renders as the muted first
   * crumb of the breadcrumb (`project / title`). Optional: a session without a
   * known project just shows the folder mark and its title. */
  projectName?: string;
  /** Immutable provenance marker for imported historical sessions. */
  sourceBadge?: string;
  /** Quiet context at the right edge. The branch and dirty count are deliberately
   * absent: the count rides on the dock's Changes tab, and the branch name in a
   * header was the noise this strip exists to remove. Direct chats pass nothing
   * here — the composer already owns model choice for them. */
  modelControl?: ReactNode;
  /** Persistent tier + effort + runtime label for the active session. Shown as a
   *  muted secondary line next to the model picker for repo sessions, or as the
   *  sole context indicator for direct chats that omit `modelControl`. */
  tierLabel?: string | null;
  /** The standing bypass-approvals warning. Sits immediately before the
   * dock/recall/menu cluster, same spot AppTitleBar gave it. */
  bypassBadge?: ReactNode;
  /** Right-edge cluster beyond the window controls. */
  actions?: ReactNode;
  /** Leading cluster, before the title: the panel toggle and history chevrons
   * the sidebar header owns while it is on screen. Desktop only — below `sm`
   * the drawer's own "Open navigation" button is the way in. */
  leading?: ReactNode;
  /** The rail is away, so this row is the window's leading edge and has to keep
   * clear of the traffic lights the way the rail's own header did. */
  sidebarHidden?: boolean;
  /** Mobile-only sidebar toggle, carried over from AppTitleBar now that it no
   * longer renders alongside this row. */
  navOpen?: boolean;
  onOpenNav?: () => void;
  dockOpen: boolean;
  onToggleDock: () => void;
  /** The dock's own pane list, so the strip and its menu stay one switcher
   * instead of a second source of truth about what a session can open. */
  dockPanes?: DockPaneDescriptor[];
  activePane?: DockPaneId;
  onOpenPane?: (pane: DockPaneId) => void;
  browserOpen: boolean;
  onToggleBrowser: () => void;
  fullscreen: boolean;
  onToggleFullscreen: () => void;
  /** Repo sessions only. */
  onOpenRouterSettings?: () => void;
  /** Search this chat's forest. Direct chats included. */
  onToggleRecall?: () => void;
  recallOpen?: boolean;
  /** Live sessions only. */
  onEnd?: () => void;
  busy?: boolean;
};

const MENU_WIDTH = 208;
const MENU_HEIGHT_ESTIMATE = 288;
// Panes that live behind the overflow menu. Anything left off this list keeps
// a button on the strip — GitHub does, because a review is a place a session
// returns to rather than a panel it peeks at.
const OVERFLOW_PANE_IDS: readonly DockPaneId[] = ["changes", "code", "terminal", "browser", "transcript", "tasks"];

function toolButtonClass(active: boolean) {
  return cn(
    "relative inline-flex h-7 w-7 shrink-0 items-center justify-center rounded-md transition-colors",
    active ? "bg-accent text-foreground" : "text-muted-foreground hover:bg-accent hover:text-foreground",
  );
}

export function SessionToolbar({
  title,
  projectName,
  sourceBadge,
  modelControl,
  tierLabel,
  bypassBadge,
  actions,
  leading,
  sidebarHidden = false,
  navOpen = false,
  onOpenNav,
  dockOpen,
  onToggleDock,
  dockPanes,
  activePane,
  onOpenPane,
  browserOpen,
  onToggleBrowser,
  fullscreen,
  onToggleFullscreen,
  onOpenRouterSettings,
  onToggleRecall,
  recallOpen = false,
  onEnd,
  busy = false,
}: SessionToolbarProps) {
  const menu = useMenuPanel<HTMLButtonElement>({ width: MENU_WIDTH, height: MENU_HEIGHT_ESTIMATE });
  const overflowPanes = dockPanes?.filter(pane => OVERFLOW_PANE_IDS.includes(pane.id)) ?? [];
  const toolbarPanes = dockPanes?.filter(pane => !OVERFLOW_PANE_IDS.includes(pane.id)) ?? [];
  const overflowAlert = overflowPanes.some(pane => pane.available && pane.alert);
  const overflowBadge = overflowPanes.some(pane => pane.available && !!pane.badge && !pane.alert);

  return (
    <div
      className={cn(
        "flex h-11 shrink-0 select-none items-center gap-2 border-b border-border pr-4 sm:pr-6",
        sidebarHidden ? "u-traffic-inset pl-24" : "pl-4 sm:pl-6",
      )}
      // The window has no native titlebar, so this strip is the grab handle:
      // "deep" makes the whole row draggable while buttons keep their clicks.
      data-tauri-drag-region="deep"
    >
      {leading && <div className="hidden shrink-0 items-center gap-0.5 sm:flex">{leading}</div>}

      {onOpenNav && (
        <button
          type="button"
          onClick={onOpenNav}
          className="inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-lg text-muted-foreground transition-colors hover:bg-accent hover:text-foreground sm:hidden"
          aria-label="Open navigation"
          aria-expanded={navOpen}
        >
          <PanelLeft size={16} strokeWidth={1.7} aria-hidden="true" />
        </button>
      )}

      {/* Keep the task visible when the sidebar is hidden or a pane is open. */}
      <div className="min-w-0 flex-1" title={projectName ? `${projectName} / ${title}` : title}>
        <h1 className="truncate text-[13px] font-semibold leading-4 text-foreground">{title}</h1>
        {projectName && <p className="truncate text-[11px] leading-4 text-muted-foreground">{projectName}</p>}
      </div>

      {sourceBadge && <span data-testid="import-source-badge" title={sourceBadge} className="hidden max-w-[260px] shrink-0 truncate rounded-full border border-border px-2 py-0.5 font-mono text-[9px] font-medium uppercase tracking-[0.04em] text-muted-foreground sm:inline">{sourceBadge}</span>}

      {(modelControl || tierLabel) && (
        <div className="hidden shrink-0 items-center gap-2 lg:flex">
          {modelControl}
          {tierLabel && (
            <span
              className="max-w-[260px] truncate text-[11px] text-muted-foreground"
              title={tierLabel}
              data-testid="tier-label"
            >
              {tierLabel}
            </span>
          )}
        </div>
      )}

      {bypassBadge}

      <span className="mx-0.5 h-4 w-px shrink-0 bg-border" aria-hidden="true" />

      <div className="flex shrink-0 items-center gap-0.5 rounded-lg p-0.5">
        <button
          type="button"
          onClick={onToggleDock}
          aria-pressed={dockOpen}
          aria-label="Toggle dock"
          title="Dock  ⌥⌘0"
          className={toolButtonClass(dockOpen)}
        >
          <PanelRight size={15} strokeWidth={1.8} aria-hidden="true" />
        </button>

        {onToggleRecall && (
          <button
            type="button"
            onClick={onToggleRecall}
            aria-pressed={recallOpen}
            aria-label="Search this chat"
            title="Search this chat"
            className={toolButtonClass(recallOpen)}
          >
            <Search size={15} strokeWidth={1.8} aria-hidden="true" />
          </button>
        )}

        {toolbarPanes.map((pane, index) => {
          const Icon = pane.icon;
          const chordIndex = (dockPanes?.findIndex(item => item.id === pane.id) ?? index) + 1;
          const active = pane.id === activePane;
          return (
            <button
              key={pane.id}
              type="button"
              onClick={() => onOpenPane?.(pane.id)}
              aria-pressed={dockOpen && active}
              aria-label={pane.label}
              title={`${pane.label}  ⌥⌘${chordIndex}${pane.available ? "" : ` — ${pane.unavailableReason ?? "unavailable"}`}`}
              className={cn(toolButtonClass(dockOpen && active), !pane.available && "opacity-40")}
            >
              <Icon size={15} strokeWidth={1.7} aria-hidden="true" />
              {!!pane.badge && pane.available && !pane.alert && (
                <span className="pointer-events-none absolute right-1 top-1 h-[5px] w-[5px] rounded-full bg-muted-foreground/70" />
              )}
              {pane.alert && pane.available && (
                <span data-testid={`dock-alert-rail-${pane.id}`} className="mission-live-accent pointer-events-none absolute right-1 top-1 h-[5px] w-[5px] rounded-full bg-warning" />
              )}
            </button>
          );
        })}

        <button
          ref={menu.triggerRef}
          type="button"
          onClick={menu.toggle}
          aria-expanded={menu.open}
          aria-haspopup="menu"
          aria-label="Session actions"
          title="Session actions"
          className={toolButtonClass(menu.open)}
        >
          <MoreHorizontal size={15} strokeWidth={1.8} aria-hidden="true" />
          {overflowAlert && (
            <span data-testid="dock-alert-overflow" className="mission-live-accent pointer-events-none absolute right-1 top-1 h-[5px] w-[5px] rounded-full bg-warning" />
          )}
          {!overflowAlert && overflowBadge && (
            <span className="pointer-events-none absolute right-1 top-1 h-[5px] w-[5px] rounded-full bg-muted-foreground/70" />
          )}
        </button>
      </div>

      {actions && <div className="flex shrink-0 items-center gap-1.5">{actions}</div>}

      <MenuPanel controller={menu} label="Session actions">
        {overflowPanes.map(pane => {
          const Icon = pane.icon;
          const chordIndex = (dockPanes?.findIndex(item => item.id === pane.id) ?? 0) + 1;
          const shortcut = <span className="shrink-0 font-mono text-[10px] text-muted-foreground">⌥⌘{chordIndex}</span>;
          const mark = (
            <span className="relative inline-flex shrink-0">
              <Icon size={13} strokeWidth={1.7} className={cn("text-muted-foreground", !pane.available && "opacity-40")} aria-hidden="true" />
              {pane.alert && pane.available && (
                <span data-testid={`dock-alert-rail-${pane.id}`} className="mission-live-accent pointer-events-none absolute -right-0.5 -top-0.5 h-[5px] w-[5px] rounded-full bg-warning" />
              )}
            </span>
          );
          if (pane.id === "browser") {
            return (
              <MenuItem
                key={pane.id}
                role="menuitemcheckbox"
                checked={browserOpen}
                label={pane.label}
                leading={mark}
                trailing={browserOpen ? <Check size={12} strokeWidth={2.2} aria-hidden="true" /> : shortcut}
                onClick={() => { onToggleBrowser(); menu.close(); }}
              />
            );
          }
          return (
            <MenuItem
              key={pane.id}
              role="menuitemcheckbox"
              checked={dockOpen && pane.id === activePane}
              label={pane.label}
              leading={mark}
              trailing={dockOpen && pane.id === activePane ? <Check size={12} strokeWidth={2.2} aria-hidden="true" /> : shortcut}
              onClick={() => { onOpenPane?.(pane.id); menu.close(); }}
            />
          );
        })}
        {overflowPanes.length === 0 && (
          <MenuItem
            role="menuitemcheckbox"
            checked={browserOpen}
            label="Browser"
            leading={<Monitor size={13} strokeWidth={1.7} className="shrink-0 text-muted-foreground" aria-hidden="true" />}
            trailing={browserOpen ? <Check size={12} strokeWidth={2.2} aria-hidden="true" /> : undefined}
            onClick={() => { onToggleBrowser(); menu.close(); }}
          />
        )}
        {overflowPanes.length > 0 && <MenuSeparator />}
        <MenuItem
          role="menuitemcheckbox"
          checked={fullscreen}
          label={fullscreen ? "Exit fullscreen" : "Fullscreen"}
          leading={fullscreen
            ? <Minimize2 size={13} strokeWidth={1.7} className="shrink-0 text-muted-foreground" aria-hidden="true" />
            : <Maximize2 size={13} strokeWidth={1.7} className="shrink-0 text-muted-foreground" aria-hidden="true" />}
          trailing={<span className="shrink-0 font-mono text-[10px] text-muted-foreground">⌥⌘F</span>}
          onClick={() => { onToggleFullscreen(); menu.close(); }}
        />
        {onOpenRouterSettings && (
          <MenuItem
            label="Learning router"
            leading={<Settings2 size={13} strokeWidth={1.7} className="shrink-0 text-muted-foreground" aria-hidden="true" />}
            onClick={() => { onOpenRouterSettings(); menu.close(); }}
          />
        )}
        {onEnd && (
          <>
            <MenuSeparator />
            <MenuItem
              label="End session"
              destructive
              disabled={busy}
              leading={<Square size={12} strokeWidth={1.9} className="shrink-0" aria-hidden="true" />}
              onClick={() => { onEnd(); menu.close(); }}
            />
          </>
        )}
      </MenuPanel>
    </div>
  );
}
