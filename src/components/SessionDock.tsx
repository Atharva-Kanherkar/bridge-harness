import { useId, useRef } from "react";
import { FolderGit2, Maximize2, Minimize2, X } from "lucide-react";
import type { LucideIcon } from "lucide-react";
import { cn } from "@/lib/utils";
import { PaneState } from "./ui/pane";
import {
  DEFAULT_DOCK_WIDTH,
  MIN_DOCK_WIDTH,
  MAX_DOCK_WIDTH,
  clampDockWidth,
  type DockAction,
  type DockPaneId,
  type DockState,
} from "../dockLayout";

// The trailing dock: a resizable pane that sits beside the conversation and
// never on top of it. This component is presentational — layout state comes in
// through props and every transition leaves through one action callback — and
// its DOM is deliberately stable: the pane container is the same node whether
// the dock is open, collapsed, expanded, or hidden, because a switch of mode
// must never unmount a shell, an editor buffer, or a scroll position.

export type DockPaneDescriptor = {
  id: DockPaneId;
  label: string;
  icon: LucideIcon;
  available: boolean;
  /** Why the pane cannot apply here — rendered instead of an empty frame. */
  unavailableReason?: string;
  /** Count badge (e.g. dirty files). Rendered whether or not the pane is active. */
  badge?: number;
  /** A state that needs the human — waiting_for_you, a pending approval, a
   *  failure. Rendered as a pulsing warning dot on the tab and the toolbar. */
  alert?: boolean;
};

export type SessionDockProps = {
  state: DockState;
  panes: DockPaneDescriptor[];
  /** Width available to the whole split, for clamping divider drags. */
  availableWidth: number;
  /** Below the sheet threshold the dock overlays the conversation instead of
   * splitting with it; the scrim click collapses. */
  sheet: boolean;
  /** Fullscreen hides the dock without unmounting or mutating it. */
  concealed?: boolean;
  onAction: (action: DockAction) => void;
  onConnectFolder?: () => void;
  children: (pane: DockPaneId) => React.ReactNode;
  /**
   * A strip below the active pane, inside the dock and above its bottom edge.
   *
   * It lives here rather than in each pane because it has to survive a pane
   * switch: a pinned agent the human cannot see from the GitHub tab is not
   * pinned. Rendered on every pane, so whatever it holds decides for itself
   * whether it applies (the pinned-agents tray returns nothing on Agents).
   */
  tray?: React.ReactNode;
};

const RESIZE_STEP = 16;

/**
 * Narrowest dock that still fits the active tab's inline label.
 *
 * Room arithmetic for the header row: each icon-only tab is a 13px icon plus
 * 2×6px of padding ≈ 25px, so seven of them with six 2px gaps and the
 * tablist's own 2×2px padding and 2×1px border come to ≈ 190px. Beside it the
 * header keeps two 28px buttons, the 8px gaps between them and 2×8px of
 * `px-2` ≈ 90px. The active label adds its 6px gap plus roughly 6px per
 * character of the longest label ("Transcript") ≈ 66px, which only clears at
 * ≈ 346px — so this sits above that with margin: at the 440px default width
 * the label sits comfortably inside the strip, and at the 320px minimum the
 * strip stays icon-only rather than spilling past its own border.
 */
export const DOCK_TAB_LABEL_MIN_WIDTH = 400;

export function SessionDock({ state, panes, availableWidth, sheet, concealed = false, onAction, onConnectFolder, children, tray }: SessionDockProps) {
  const dockId = useId();
  const dragging = useRef(false);
  // Expanded means the dock is `flex-1` — the whole split, always wider than
  // the threshold — so the label rides along with it.
  const showActiveLabel = state.expanded || state.width >= DOCK_TAB_LABEL_MIN_WIDTH;

  const startResize = (event: React.PointerEvent<HTMLDivElement>) => {
    event.preventDefault();
    const originX = event.clientX;
    const originWidth = state.width;
    dragging.current = true;
    event.currentTarget.setPointerCapture(event.pointerId);
    const move = (pointer: PointerEvent) =>
      onAction({ type: "set-width", width: originWidth + (originX - pointer.clientX), available: availableWidth });
    const release = () => {
      dragging.current = false;
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", release);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", release);
  };

  const onDividerKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
    event.preventDefault();
    const delta = event.key === "ArrowLeft" ? RESIZE_STEP : -RESIZE_STEP;
    onAction({ type: "set-width", width: state.width + delta, available: availableWidth });
  };

  return (
    <>
      {state.open && !state.expanded && !sheet && !concealed && (
        <div
          role="separator"
          aria-orientation="vertical"
          aria-label="Resize dock"
          aria-valuenow={state.width}
          aria-valuemin={MIN_DOCK_WIDTH}
          aria-valuemax={clampDockWidth(MAX_DOCK_WIDTH, availableWidth)}
          tabIndex={0}
          onPointerDown={startResize}
          onKeyDown={onDividerKeyDown}
          onDoubleClick={() => onAction({ type: "set-width", width: DEFAULT_DOCK_WIDTH, available: availableWidth })}
          title="Drag to resize · double-click to reset"
          className="group relative z-20 -mr-1.5 w-3 shrink-0 cursor-col-resize"
        >
          <span className="absolute inset-y-0 left-1.5 w-px bg-border transition-colors group-hover:bg-ring/60 group-focus-visible:bg-ring" />
        </div>
      )}

      {state.open && sheet && !state.expanded && !concealed && (
        <button
          type="button"
          aria-label="Close dock"
          onClick={() => onAction({ type: "toggle" })}
          className="absolute inset-0 z-20 bg-scrim"
        />
      )}

      <aside
        aria-label="Dock"
        className={cn(
          "flex min-w-0 max-w-full flex-col border-l border-border bg-background",
          (concealed || !state.open) && "hidden",
          !concealed && state.open && state.expanded && "flex-1",
          !concealed && state.open && !state.expanded && sheet && "absolute inset-y-0 right-0 z-30 shadow-2xl",
          !concealed && state.open && !state.expanded && !sheet && "shrink-0",
        )}
        style={!concealed && state.open && !state.expanded ? { width: state.width } : undefined}
      >
        {state.open && !concealed && (
          <div className="flex min-h-11 shrink-0 items-center gap-1 border-b border-border bg-muted/30 px-2 py-1">
            <div role="tablist" aria-label="Dock panes" className="flex min-h-8 min-w-0 flex-1 items-center gap-0.5 overflow-x-auto rounded-lg p-0.5">
              {panes.map((pane, index) => {
                const active = pane.id === state.pane;
                const Icon = pane.icon;
                return (
                  <button
                    key={pane.id}
                    type="button"
                    role="tab"
                    id={`${dockId}-tab-${pane.id}`}
                    aria-controls={`${dockId}-pane-${pane.id}`}
                    tabIndex={active ? 0 : -1}
                    aria-selected={active}
                    aria-label={pane.label}
                    title={`${pane.label}  ⌥⌘${index + 1}${pane.available ? "" : ` — ${pane.unavailableReason ?? "unavailable"}`}`}
                    onClick={() => onAction({ type: "open-pane", pane: pane.id })}
                    onKeyDown={event => {
                      let next = index;
                      if (event.key === "ArrowRight") next = (index + 1) % panes.length;
                      else if (event.key === "ArrowLeft") next = (index - 1 + panes.length) % panes.length;
                      else if (event.key === "Home") next = 0;
                      else if (event.key === "End") next = panes.length - 1;
                      else return;
                      event.preventDefault();
                      onAction({ type: "open-pane", pane: panes[next].id });
                      document.getElementById(`${dockId}-tab-${panes[next].id}`)?.focus();
                    }}
                    className={cn(
                      "relative flex h-7 min-w-7 shrink-0 items-center justify-center gap-1.5 rounded-md text-[12px] font-medium transition-colors",
                      // Icon-only mode is the narrow one, and the badges still
                      // ride along, so the tabs give up a little padding there
                      // to keep the last one inside the strip at the minimum width.
                      showActiveLabel ? "px-1.5" : "px-1",
                      active ? "bg-selection text-selection-foreground" : "text-muted-foreground hover:bg-accent hover:text-foreground",
                      !pane.available && !active && "opacity-40",
                    )}
                  >
                    <Icon size={13} strokeWidth={1.7} aria-hidden="true" />
                    {active && showActiveLabel && <span className="min-w-0 truncate pr-0.5">{pane.label}</span>}
                    {!!pane.badge && pane.available && (
                      <span className="rounded-full bg-accent px-1 font-mono text-[11px] leading-4 text-muted-foreground">{pane.badge}</span>
                    )}
                    {pane.alert && pane.available && (
                      <span data-testid={`dock-alert-${pane.id}`} className="mission-live-accent pointer-events-none absolute right-0 top-0 h-[5px] w-[5px] rounded-full bg-warning" />
                    )}
                  </button>
                );
              })}
            </div>

            <span className="ml-auto" />

            <button
              type="button"
              onClick={() => onAction({ type: "toggle-expanded" })}
              aria-pressed={state.expanded}
              aria-label={state.expanded ? "Restore dock" : "Expand dock"}
              title={state.expanded ? "Restore  ⌥⌘↩" : "Expand  ⌥⌘↩"}
              className="inline-flex h-7 w-7 shrink-0 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
            >
              {state.expanded
                ? <Minimize2 size={14} strokeWidth={1.8} aria-hidden="true" />
                : <Maximize2 size={14} strokeWidth={1.8} aria-hidden="true" />}
            </button>
            <button
              type="button"
              onClick={() => onAction({ type: "toggle" })}
              aria-label="Close dock"
              title="Close dock  ⌥⌘0"
              className="inline-flex h-7 w-7 shrink-0 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
            >
              <X size={14} strokeWidth={1.8} aria-hidden="true" />
            </button>
          </div>
        )}

        {/* The pane container outlives every mode switch. Hidden while the dock
            is collapsed or concealed, but the visited panes inside keep their
            nodes — a UI toggle never tears one down. */}
        <div className={cn("min-h-0 flex-1 overflow-hidden", (!state.open || concealed) && "hidden")}>
          {panes.map(pane =>
            state.visited.includes(pane.id) ? (
              <div key={pane.id} id={`${dockId}-pane-${pane.id}`} role="tabpanel" aria-label={pane.label} className={cn("h-full", pane.id !== state.pane && "hidden")}>
                {pane.available ? children(pane.id) : (
                  <PaneState icon={FolderGit2} title={`${pane.label} is unavailable`} action={onConnectFolder && (
                      <button
                        type="button"
                        onClick={onConnectFolder}
                        className="u-glass-soft min-h-8 rounded-lg px-3 text-[13px] text-foreground transition-colors hover:bg-accent"
                      >
                        Connect a folder
                      </button>
                    )}>{pane.unavailableReason ?? "Connect a project folder to use this inspector."}</PaneState>
                )}
              </div>
            ) : null,
          )}
        </div>

        {tray && <div className={cn((!state.open || concealed) && "hidden")}>{tray}</div>}
      </aside>
    </>
  );
}
