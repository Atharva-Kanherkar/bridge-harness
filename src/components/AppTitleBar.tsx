import type { ReactNode } from "react";
import { PanelLeft } from "lucide-react";
import { cn } from "@/lib/utils";

// The window keeps no native titlebar (overlay style, hidden title), so this
// strip is the app's own: it carries the canvas chrome and drags the window
// from anywhere that is not a control. With the sidebar hidden this row also
// inherits the panel and history controls the sidebar header used to hold.

export type AppTitleBarProps = {
  /** Names the current view where the sidebar is hidden and cannot. */
  title: string;
  navOpen: boolean;
  onOpenNav: () => void;
  /** Leading cluster, before the title: the panel toggle and history chevrons
   *  the sidebar header owns while it is on screen. Desktop only — below `sm`
   *  the drawer's own "Open navigation" button is the way in. */
  leading?: ReactNode;
  /** Optional right-edge controls, kept clear of the window corner. */
  actions?: ReactNode;
  /** No hairline — the strip shares a surface with a flush sidebar. */
  flush?: boolean;
  /** Hide the brand word when the rail already owns the window's leading edge. */
  hideBrand?: boolean;
  /** The rail is away, so this row is the window's leading edge and has to keep
   *  clear of the traffic lights the way the rail's own header did. */
  sidebarHidden?: boolean;
};

export function AppTitleBar({ title, navOpen, onOpenNav, leading, actions, flush = false, hideBrand: _hideBrand = false, sidebarHidden = false }: AppTitleBarProps) {
  return (
    <header
      data-tauri-drag-region="deep"
      className={cn(
        "flex h-11 shrink-0 items-center gap-2",
        flush ? "bg-background pr-3" : "u-vibrancy-sidebar border-b border-border bg-sidebar pr-[var(--window-control-inset)]",
        !flush || sidebarHidden ? "u-traffic-inset pl-24" : "pl-5",
      )}
    >
      {leading && <div className="hidden shrink-0 items-center gap-0.5 sm:flex">{leading}</div>}
      <button
        type="button"
        onClick={onOpenNav}
        className="inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-lg text-muted-foreground transition-colors hover:bg-accent hover:text-foreground sm:hidden"
        aria-label="Open navigation"
        aria-expanded={navOpen}
      >
        <PanelLeft size={16} strokeWidth={1.7} aria-hidden="true" />
      </button>
      <p className="m-0 min-w-0 flex-1 truncate">
        <span className="text-[13px] font-semibold text-foreground">{title}</span>
      </p>
      {actions && <div className="flex shrink-0 items-center gap-1.5">{actions}</div>}
    </header>
  );
}
