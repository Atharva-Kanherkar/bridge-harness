import type { ReactNode } from "react";
import { PanelLeft } from "lucide-react";
import { cn } from "@/lib/utils";

// The window keeps no native titlebar (overlay style, hidden title), so this
// strip is the app's own: it owns the traffic-light corner, carries the brand,
// and drags the window from anywhere that is not a control. Fullscreen renders
// no strip at all — the session toolbar takes over as the topmost row there.

export type AppTitleBarProps = {
  /** Names the current view where the sidebar is hidden and cannot. */
  title: string;
  navOpen: boolean;
  onOpenNav: () => void;
  /** Right-edge cluster: view toggles, usage. Trailing inset matches the usage chip
   *  so its top-right corner is concentric with the window. Controls block dragging. */
  actions?: ReactNode;
  /** No hairline — the strip shares a surface with a flush sidebar. */
  flush?: boolean;
  /** Hide the brand word when the rail already owns the window's leading edge. */
  hideBrand?: boolean;
};

export function AppTitleBar({ title, navOpen, onOpenNav, actions, flush = false, hideBrand = false }: AppTitleBarProps) {
  return (
    <header
      data-tauri-drag-region="deep"
      className={cn(
        "flex h-11 shrink-0 items-center gap-2 pr-[var(--window-control-inset)]",
        flush ? "bg-transparent pl-2" : "u-vibrancy-sidebar border-b border-border bg-sidebar pl-24",
      )}
    >
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
        <span className="text-[12.5px] font-medium text-foreground sm:hidden">{title}</span>
        {!hideBrand && <span className="hidden font-display text-[14px] font-semibold tracking-[-0.012em] text-foreground sm:inline">bridge</span>}
      </p>
      {actions && <div className="flex shrink-0 items-center gap-1.5">{actions}</div>}
    </header>
  );
}
