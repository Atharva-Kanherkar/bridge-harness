import type { ReactNode } from "react";
import { ChevronLeft, ChevronRight, PanelLeft } from "lucide-react";
import { cn } from "@/lib/utils";

export type WindowNavButtonsProps = {
  collapsed: boolean;
  onToggleCollapsed: () => void;
  canBack: boolean;
  canForward: boolean;
  onBack: () => void;
  onForward: () => void;
  /** Panel on the left, chevrons pushed to the right of the parent flex row. */
  spread?: boolean;
};

function NavButton({
  label,
  disabled,
  onClick,
  children,
}: {
  label: string;
  disabled?: boolean;
  onClick: () => void;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      title={label}
      aria-label={label}
      disabled={disabled}
      onClick={onClick}
      className={cn(
        "inline-flex size-7 shrink-0 items-center justify-center rounded-md text-muted-foreground transition-colors",
        "hover:bg-accent hover:text-foreground disabled:pointer-events-none disabled:opacity-35",
      )}
    >
      {children}
    </button>
  );
}

export function WindowPanelButton({ collapsed, onToggleCollapsed }: Pick<WindowNavButtonsProps, "collapsed" | "onToggleCollapsed">) {
  return (
    <NavButton label={collapsed ? "Show sidebar" : "Hide sidebar"} onClick={onToggleCollapsed}>
      <PanelLeft size={15} strokeWidth={1.5} aria-hidden="true" />
    </NavButton>
  );
}

export function WindowHistoryChevrons({
  canBack,
  canForward,
  onBack,
  onForward,
}: Pick<WindowNavButtonsProps, "canBack" | "canForward" | "onBack" | "onForward">) {
  return (
    <div className="flex shrink-0 items-center gap-0.5">
      <NavButton label="Back" disabled={!canBack} onClick={onBack}>
        <ChevronLeft size={15} strokeWidth={1.5} aria-hidden="true" />
      </NavButton>
      <NavButton label="Forward" disabled={!canForward} onClick={onForward}>
        <ChevronRight size={15} strokeWidth={1.5} aria-hidden="true" />
      </NavButton>
    </div>
  );
}

/** Panel toggle plus history chevrons. Spread matches Cursor: panel beside the
 *  traffic lights, chevrons on the far right of the same strip. */
export function WindowNavButtons({
  collapsed,
  onToggleCollapsed,
  canBack,
  canForward,
  onBack,
  onForward,
  spread = false,
}: WindowNavButtonsProps) {
  const panel = <WindowPanelButton collapsed={collapsed} onToggleCollapsed={onToggleCollapsed} />;
  const chevrons = (
    <WindowHistoryChevrons
      canBack={canBack}
      canForward={canForward}
      onBack={onBack}
      onForward={onForward}
    />
  );
  if (spread) {
    return (
      <>
        {panel}
        <div className="ml-auto">{chevrons}</div>
      </>
    );
  }
  return (
    <div className="flex shrink-0 items-center gap-0.5">
      {panel}
      {chevrons}
    </div>
  );
}
