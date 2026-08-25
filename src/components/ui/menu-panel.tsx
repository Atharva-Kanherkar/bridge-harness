import { useCallback, useEffect, useRef, useState, type ReactNode, type RefObject } from "react";
import { createPortal } from "react-dom";
import { cn } from "@/lib/utils";

// A small floating panel anchored under its trigger. It is portalled to the body
// on fixed coordinates because both callers sit inside a scrolling container that
// would otherwise clip an absolutely positioned panel — and for the same reason it
// closes on scroll rather than drifting away from the control that opened it.

export type MenuPanelController<T extends HTMLElement> = {
  open: boolean;
  anchor: { left: number; top: number };
  triggerRef: RefObject<T>;
  panelRef: RefObject<HTMLDivElement>;
  width: number;
  toggle: () => void;
  close: () => void;
};

export function useMenuPanel<T extends HTMLElement>({ width, height }: { width: number; height: number }): MenuPanelController<T> {
  const [open, setOpen] = useState(false);
  const [anchor, setAnchor] = useState({ left: 0, top: 0 });
  const triggerRef = useRef<T>(null);
  const panelRef = useRef<HTMLDivElement>(null);

  const close = useCallback(() => setOpen(false), []);

  const toggle = useCallback(() => {
    setOpen(current => {
      if (current) return false;
      const rect = triggerRef.current?.getBoundingClientRect();
      if (rect) {
        setAnchor({
          left: Math.max(8, Math.min(rect.right - width, window.innerWidth - width - 8)),
          top: Math.min(rect.bottom + 6, Math.max(8, window.innerHeight - height)),
        });
      }
      return true;
    });
  }, [width, height]);

  useEffect(() => {
    if (!open) return;
    const onPointerDown = (event: Event) => {
      const target = event.target as Node;
      if (triggerRef.current?.contains(target) || panelRef.current?.contains(target)) return;
      close();
    };
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        close();
        window.requestAnimationFrame(() => triggerRef.current?.focus());
      }
    };
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

  return { open, anchor, triggerRef, panelRef, width, toggle, close };
}

export function MenuPanel<T extends HTMLElement>({
  controller,
  label,
  className,
  children,
}: {
  controller: MenuPanelController<T>;
  label: string;
  className?: string;
  children: ReactNode;
}) {
  useEffect(() => {
    if (!controller.open) return;
    const panel = controller.panelRef.current;
    if (!panel) return;
    let frame: number | undefined;
    const focusFirstItem = () => {
      if (panel.contains(document.activeElement)) return;
      if (document.activeElement !== controller.triggerRef.current && document.activeElement !== document.body) return;
      panel.querySelector<HTMLElement>('[role^="menuitem"]:not([disabled])')?.focus();
    };
    const scheduleFocus = () => {
      if (frame !== undefined) window.cancelAnimationFrame(frame);
      frame = window.requestAnimationFrame(focusFirstItem);
    };
    const observer = new MutationObserver(scheduleFocus);
    observer.observe(panel, { childList: true, subtree: true });
    scheduleFocus();
    return () => {
      observer.disconnect();
      if (frame !== undefined) window.cancelAnimationFrame(frame);
    };
  }, [controller.open, controller.panelRef, controller.triggerRef]);

  if (!controller.open) return null;
  return createPortal(
    <div
      ref={controller.panelRef}
      role="menu"
      aria-label={label}
      onClickCapture={event => {
        const item = (event.target as Element).closest<HTMLElement>('[role^="menuitem"]');
        if (!item || item.hasAttribute("disabled")) return;
        window.requestAnimationFrame(() => controller.triggerRef.current?.focus());
      }}
      onKeyDown={event => {
        if (event.key === "Escape") {
          event.preventDefault();
          event.stopPropagation();
          controller.close();
          window.requestAnimationFrame(() => controller.triggerRef.current?.focus());
          return;
        }
        const items = [...event.currentTarget.querySelectorAll<HTMLElement>('[role^="menuitem"]:not([disabled])')];
        if (!items.length) return;
        const current = Math.max(0, items.indexOf(document.activeElement as HTMLElement));
        const next = event.key === "ArrowDown"
          ? (current + 1) % items.length
          : event.key === "ArrowUp"
          ? (current - 1 + items.length) % items.length
          : event.key === "Home"
          ? 0
          : event.key === "End"
          ? items.length - 1
          : null;
        if (next === null) return;
        event.preventDefault();
        items[next]?.focus();
      }}
      style={{ left: controller.anchor.left, top: controller.anchor.top, width: controller.width }}
      className={cn("fixed z-50 rounded-lg border border-border bg-popover p-1 text-popover-foreground shadow-lg", className)}
    >
      {children}
    </div>,
    document.body,
  );
}

/** One row in a menu: an optional leading mark, a label, and optional trailing text. */
export function MenuItem({
  label,
  onClick,
  leading,
  trailing,
  role = "menuitem",
  checked,
  destructive,
  disabled,
}: {
  label: string;
  onClick: () => void;
  leading?: ReactNode;
  trailing?: ReactNode;
  role?: "menuitem" | "menuitemradio" | "menuitemcheckbox";
  checked?: boolean;
  destructive?: boolean;
  disabled?: boolean;
}) {
  return (
    <button
      type="button"
      role={role}
      aria-checked={role === "menuitem" ? undefined : !!checked}
      disabled={disabled}
      tabIndex={-1}
      onClick={onClick}
      className={cn(
        "flex h-7 w-full items-center gap-2 rounded-md px-2 text-left text-[13px] transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:opacity-40",
        destructive ? "text-destructive hover:bg-destructive/10" : "hover:bg-accent",
      )}
    >
      {leading}
      <span className="min-w-0 flex-1 truncate">{label}</span>
      {trailing}
    </button>
  );
}

export function MenuSeparator() {
  return <div className="my-1 h-px bg-border" />;
}
