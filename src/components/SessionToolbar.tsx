import type { ReactNode } from "react";
import { Check, Maximize2, Minimize2, Monitor, MoreHorizontal, PanelLeft, PanelRight, Route, Search, Settings2, Square } from "lucide-react";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { Tip } from "@/components/ui/tooltip";
import { MenuItem, MenuPanel, MenuSeparator, useMenuPanel } from "@/components/ui/menu-panel";
import type { DockPaneId } from "../dockLayout";
import type { DockPaneDescriptor } from "./SessionDock";
import { formatShortcut } from "@/lib/platform";
import type { TrailSummary } from "@/protocol/generated/protocol";
import { TrailPeek } from "./trails/TrailPeek";

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
  /** The trail this chat sits on: a chip that opens it, and shows it on hover. */
  trail?: TrailSummary;
  onOpenTrail?: (trailId: string) => void;
  /** "Add to trail…" in the overflow menu. */
  onAddToTrail?: () => void;
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
const MENU_HEIGHT_ESTIMATE = 316;
// Panes that live behind the overflow menu. Anything left off this list keeps
// a button on the strip — GitHub does, because a review is a place a session
// returns to rather than a panel it peeks at.
const OVERFLOW_PANE_IDS: readonly DockPaneId[] = ["changes", "code", "terminal", "browser", "transcript", "tasks"];

export function SessionToolbar({
  title,
  trail,
  onOpenTrail,
  onAddToTrail,
  projectName,
  sourceBadge,
  modelControl,
  tierLabel,
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
        // A container, because the strip's room is the window minus the
        // sidebar: viewport breakpoints kept the trail chip on screen while
        // the title beside it had already collapsed to a single letter.
        "@container/toolbar flex h-11 shrink-0 select-none items-center gap-2 border-b border-border pr-4 sm:pr-6",
        sidebarHidden ? "u-traffic-inset pl-24" : "pl-4 sm:pl-6",
      )}
      // The window has no native titlebar, so this strip is the grab handle:
      // "deep" makes the whole row draggable while buttons keep their clicks.
      data-tauri-drag-region="deep"
    >
      {leading && <div className="hidden shrink-0 items-center gap-0.5 sm:flex">{leading}</div>}

      {onOpenNav && (
        <Button
          variant="quiet"
          size="icon-sm"
          onClick={onOpenNav}
          className="sm:hidden"
          aria-label="Open navigation"
          aria-expanded={navOpen}
        >
          <PanelLeft size={16} strokeWidth={1.7} aria-hidden="true" />
        </Button>
      )}

      {/* Keep the task visible when the sidebar is hidden or a pane is open. */}
      <Tip label={projectName ? `${projectName} / ${title}` : title} side="bottom"><div className="min-w-12 flex-1">
        <h1 className="truncate text-ui font-semibold leading-4 text-foreground">{title}</h1>
        {projectName && <p className="truncate text-caption leading-4 text-muted-foreground">{projectName}</p>}
      </div></Tip>

      {trail && onOpenTrail && (
        <TrailPeek
          trail={trail}
          onOpen={onOpenTrail}
          side="bottom"
          align="end"
          trigger={<Button
            variant="secondary"
            size="2xs"
            onClick={() => onOpenTrail(trail.id)}
            aria-label={`Open trail ${trail.title}`}
            data-testid="trail-chip"
            // Gives way before the title does, and steps aside entirely
            // once the strip cannot hold both.
            className="hidden min-w-0 max-w-[220px] shrink @min-[620px]/toolbar:inline-flex"
          >
            <Route aria-hidden="true" />
            <span className="truncate">{trail.title}</span>
          </Button>}
        />
      )}

      {sourceBadge && <Tip label={sourceBadge}><span data-testid="import-source-badge" className="hidden max-w-[260px] min-w-0 truncate rounded-full border border-border px-2 py-0.5 font-mono text-caption font-medium text-muted-foreground @min-[620px]/toolbar:inline">{sourceBadge}</span></Tip>}

      {(modelControl || tierLabel) && (
        <div className="hidden shrink-0 items-center gap-2 @min-[900px]/toolbar:flex">
          {modelControl}
          {tierLabel && (
            <Tip label={tierLabel}><span
              className="max-w-[260px] truncate text-caption text-muted-foreground"
              data-testid="tier-label"
            >
              {tierLabel}
            </span></Tip>
          )}
        </div>
      )}

      <span className="mx-0.5 h-4 w-px shrink-0 bg-border" aria-hidden="true" />

      <div className="flex shrink-0 items-center gap-0.5 rounded-lg p-0.5">
        <Tip label="Dock" shortcut={formatShortcut({ meta: true, alt: true }, "0")} side="bottom"><Button
          variant="quiet"
          size="icon-xs"
          onClick={onToggleDock}
          aria-pressed={dockOpen}
          aria-label="Toggle dock"
        >
          <PanelRight size={15} strokeWidth={1.8} aria-hidden="true" />
        </Button></Tip>

        {onToggleRecall && (
          <Button
            variant="quiet"
            size="icon-xs"
            onClick={onToggleRecall}
            aria-pressed={recallOpen}
            aria-label="Search this chat"
            tooltip="Search this chat"
            tooltipSide="bottom"
          >
            <Search size={15} strokeWidth={1.8} aria-hidden="true" />
          </Button>
        )}

        {toolbarPanes.map((pane, index) => {
          const Icon = pane.icon;
          const chordIndex = (dockPanes?.findIndex(item => item.id === pane.id) ?? index) + 1;
          const active = pane.id === activePane;
          return (
            <Tip key={pane.id} label={`${pane.label}${pane.available ? "" : ` — ${pane.unavailableReason ?? "unavailable"}`}`} shortcut={formatShortcut({ meta: true, alt: true }, String(chordIndex))} side="bottom"><Button
              variant="quiet"
              size="icon-xs"
              onClick={() => onOpenPane?.(pane.id)}
              aria-pressed={dockOpen && active}
              aria-label={pane.label}
            >
              <Icon size={15} strokeWidth={1.7} className={cn(!pane.available && "opacity-40")} aria-hidden="true" />
              {!!pane.badge && pane.available && !pane.alert && (
                <span className="pointer-events-none absolute right-1 top-1 h-[5px] w-[5px] rounded-full bg-muted-foreground/70" />
              )}
              {pane.alert && pane.available && (
                <span data-testid={`dock-alert-rail-${pane.id}`} className="mission-live-accent pointer-events-none absolute right-1 top-1 h-[5px] w-[5px] rounded-full bg-warning" />
              )}
            </Button></Tip>
          );
        })}

        <Button
          ref={menu.triggerRef}
          variant="quiet"
          size="icon-xs"
          onClick={menu.toggle}
          aria-expanded={menu.open}
          aria-haspopup="menu"
          aria-label="Session actions"
          tooltip="Session actions"
          tooltipSide="bottom"
        >
          <MoreHorizontal size={15} strokeWidth={1.8} aria-hidden="true" />
          {overflowAlert && (
            <span data-testid="dock-alert-overflow" className="mission-live-accent pointer-events-none absolute right-1 top-1 h-[5px] w-[5px] rounded-full bg-warning" />
          )}
          {!overflowAlert && overflowBadge && (
            <span className="pointer-events-none absolute right-1 top-1 h-[5px] w-[5px] rounded-full bg-muted-foreground/70" />
          )}
        </Button>
      </div>

      {actions && <div className="flex shrink-0 items-center gap-1.5">{actions}</div>}

      <MenuPanel controller={menu} label="Session actions">
        {overflowPanes.map(pane => {
          const Icon = pane.icon;
          const chordIndex = (dockPanes?.findIndex(item => item.id === pane.id) ?? 0) + 1;
          const shortcut = <span className="shrink-0 font-mono text-micro text-muted-foreground">{formatShortcut({ meta: true, alt: true }, String(chordIndex))}</span>;
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
        {onAddToTrail && !trail && (
          <MenuItem
            label="Add to trail…"
            leading={<Route size={13} strokeWidth={1.7} className="shrink-0 text-muted-foreground" aria-hidden="true" />}
            onClick={() => { onAddToTrail(); menu.close(); }}
          />
        )}
        <MenuItem
          role="menuitemcheckbox"
          checked={fullscreen}
          label={fullscreen ? "Exit fullscreen" : "Fullscreen"}
          leading={fullscreen
            ? <Minimize2 size={13} strokeWidth={1.7} className="shrink-0 text-muted-foreground" aria-hidden="true" />
            : <Maximize2 size={13} strokeWidth={1.7} className="shrink-0 text-muted-foreground" aria-hidden="true" />}
          trailing={<span className="shrink-0 font-mono text-micro text-muted-foreground">{formatShortcut({ meta: true, alt: true }, "F")}</span>}
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
