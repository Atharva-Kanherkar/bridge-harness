import type { ReactNode } from "react";
import { PanelLeft } from "lucide-react";

// The window keeps no native titlebar (overlay style, hidden title), so this
// strip is the app's own: it owns the traffic-light corner, carries the brand,
// and drags the window from anywhere that is not a control. Fullscreen renders
// no strip at all — the session toolbar takes over as the topmost row there.

export type AppTitleBarProps = {
  /** Names the current view where the sidebar is hidden and cannot. */
  title: string;
  navOpen: boolean;
  onOpenNav: () => void;
  /** Right-edge cluster: view toggles, usage. Controls block dragging on their own. */
  actions?: ReactNode;
};

export function AppTitleBar({ title, navOpen, onOpenNav, actions }: AppTitleBarProps) {
  return (
    <header
      data-tauri-drag-region="deep"
      className="flex h-11 shrink-0 items-center gap-2 border-b border-border bg-sidebar pl-24 pr-2 sm:pr-3"
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
        <span className="hidden font-display text-[14px] font-semibold tracking-[-0.012em] text-foreground sm:inline">bridge</span>
      </p>
      {actions && <div className="flex shrink-0 items-center gap-1.5">{actions}</div>}
    </header>
  );
}
