import { useRef } from "react";
import { FolderGit2, Maximize2, Minimize2, X } from "lucide-react";
import type { LucideIcon } from "lucide-react";
import { cn } from "@/lib/utils";
import {
  DEFAULT_DOCK_WIDTH,
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
};

const RESIZE_STEP = 16;

export function SessionDock({ state, panes, availableWidth, sheet, concealed = false, onAction, onConnectFolder, children }: SessionDockProps) {
  const dragging = useRef(false);

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
          "flex min-w-0 flex-col border-l border-border bg-sidebar",
          concealed && "hidden",
          !concealed && state.open && state.expanded && "flex-1",
          !concealed && state.open && !state.expanded && sheet && "absolute inset-y-0 right-0 z-30 shadow-2xl",
          !concealed && state.open && !state.expanded && !sheet && "shrink-0",
          !concealed && !state.open && "w-11 shrink-0 items-stretch",
        )}
        style={!concealed && state.open && !state.expanded ? { width: state.width } : undefined}
      >
        {state.open && !concealed && (
          <div className="flex h-11 shrink-0 items-center gap-2 border-b border-border px-2">
            <div role="tablist" aria-label="Dock panes" className="flex h-7 min-w-0 items-center gap-0.5 rounded-lg border border-border bg-muted p-0.5">
              {panes.map((pane, index) => {
                const active = pane.id === state.pane;
                const Icon = pane.icon;
                return (
                  <button
                    key={pane.id}
                    type="button"
                    role="tab"
                    aria-selected={active}
                    aria-label={pane.label}
                    title={`${pane.label}  ⌥⌘${index + 1}${pane.available ? "" : ` — ${pane.unavailableReason ?? "unavailable"}`}`}
                    onClick={() => onAction({ type: "open-pane", pane: pane.id })}
                    className={cn(
                      "flex h-6 shrink-0 items-center gap-1.5 rounded-md px-1.5 text-[11px] font-medium transition-colors",
                      active ? "bg-card text-foreground" : "text-muted-foreground hover:text-foreground",
                      !pane.available && !active && "opacity-40",
                    )}
                  >
                    <Icon size={13} strokeWidth={1.7} aria-hidden="true" />
                    {active && <span className="pr-0.5">{pane.label}</span>}
                    {!!pane.badge && pane.available && (
                      <span className="rounded-full bg-accent px-1 font-mono text-[10px] leading-4 text-muted-foreground">{pane.badge}</span>
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

        {!state.open && !concealed && (
          <div className="flex flex-col items-center gap-1 py-2">
            {panes.map((pane, index) => {
              const Icon = pane.icon;
              return (
                <button
                  key={pane.id}
                  type="button"
                  onClick={() => onAction({ type: "open-pane", pane: pane.id })}
                  aria-label={pane.label}
                  title={`${pane.label}  ⌥⌘${index + 1}`}
                  className={cn(
                    "relative inline-flex h-8 w-8 items-center justify-center rounded-md transition-colors",
                    pane.id === state.pane ? "bg-accent text-foreground" : "text-muted-foreground hover:bg-accent hover:text-foreground",
                    !pane.available && "opacity-40",
                  )}
                >
                  <Icon size={15} strokeWidth={1.7} aria-hidden="true" />
                  {!!pane.badge && pane.available && (
                    <span className="pointer-events-none absolute right-1 top-1 h-[5px] w-[5px] rounded-full bg-muted-foreground/70" />
                  )}
                </button>
              );
            })}
          </div>
        )}

        {/* The pane container outlives every mode switch. Hidden while the dock
            is collapsed or concealed, but the visited panes inside keep their
            nodes — a UI toggle never tears one down. */}
        <div className={cn("min-h-0 flex-1 overflow-hidden", (!state.open || concealed) && "hidden")}>
          {panes.map(pane =>
            state.visited.includes(pane.id) ? (
              <div key={pane.id} className={cn("h-full", pane.id !== state.pane && "hidden")}>
                {pane.available ? children(pane.id) : (
                  <div className="flex h-full flex-col items-center justify-center gap-3 px-8 text-center">
                    <FolderGit2 size={20} strokeWidth={1.5} className="text-muted-foreground/50" aria-hidden="true" />
                    <p className="max-w-[34ch] text-[12.5px] leading-relaxed text-muted-foreground">
                      {pane.unavailableReason ?? `${pane.label} is unavailable here.`}
                    </p>
                    {onConnectFolder && (
                      <button
                        type="button"
                        onClick={onConnectFolder}
                        className="u-surface rounded-lg px-2.5 py-1 text-[11.5px] text-foreground transition-colors hover:bg-accent"
                      >
                        Connect a folder
                      </button>
                    )}
                  </div>
                )}
              </div>
            ) : null,
          )}
        </div>
      </aside>
    </>
  );
}
