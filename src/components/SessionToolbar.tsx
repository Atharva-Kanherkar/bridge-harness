import { Check, Maximize2, Minimize2, Monitor, MoreHorizontal, PanelRight, Search, Settings2, Square } from "lucide-react";
import { cn } from "@/lib/utils";
import { MenuItem, MenuPanel, MenuSeparator, useMenuPanel } from "@/components/ui/menu-panel";

// One strip for the whole session: what it is and the window actions. The
// panel tabs that used to live here moved into the dock's own switcher — the
// dock is beside the conversation now, not instead of it, so the toolbar only
// carries the affordance that shows or hides it.

export type SessionToolbarProps = {
  title: string;
  /** Quiet context at the right edge. The branch and dirty count are deliberately
   * absent: the count rides on the dock's Changes tab, and the branch name in a
   * header was the noise this strip exists to remove. */
  model?: string;
  dockOpen: boolean;
  onToggleDock: () => void;
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
const MENU_HEIGHT_ESTIMATE = 160;

export function SessionToolbar({
  title,
  model,
  dockOpen,
  onToggleDock,
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

  return (
    <div
      className="flex h-11 shrink-0 items-center gap-2 border-b border-border px-4 sm:px-6"
      // The window has no native titlebar, so this strip is the grab handle:
      // "deep" makes the whole row draggable while buttons keep their clicks.
      data-tauri-drag-region="deep"
    >
      <h1 className="m-0 min-w-0 flex-1 truncate font-display text-[14px] font-semibold leading-none tracking-[-0.014em] text-foreground">
        {title}
      </h1>

      {model && (
        <p className="hidden shrink-0 truncate text-[11px] text-muted-foreground lg:block">{model}</p>
      )}

      <button
        type="button"
        onClick={onToggleDock}
        aria-pressed={dockOpen}
        aria-label="Toggle dock"
        title="Dock  ⌥⌘0"
        className={cn(
          "inline-flex h-7 w-7 shrink-0 items-center justify-center rounded-md transition-colors",
          dockOpen ? "bg-accent text-foreground" : "text-muted-foreground hover:bg-accent hover:text-foreground",
        )}
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
          className={cn(
            "inline-flex h-7 w-7 shrink-0 items-center justify-center rounded-md transition-colors",
            recallOpen ? "bg-accent text-foreground" : "text-muted-foreground hover:bg-accent hover:text-foreground",
          )}
        >
          <Search size={15} strokeWidth={1.8} aria-hidden="true" />
        </button>
      )}

      <button
        ref={menu.triggerRef}
        type="button"
        onClick={menu.toggle}
        aria-expanded={menu.open}
        aria-haspopup="menu"
        aria-label="Session actions"
        title="Session actions"
        className={cn(
          "inline-flex h-7 w-7 shrink-0 items-center justify-center rounded-md transition-colors",
          menu.open ? "bg-accent text-foreground" : "text-muted-foreground hover:bg-accent hover:text-foreground",
        )}
      >
        <MoreHorizontal size={15} strokeWidth={1.8} aria-hidden="true" />
      </button>

      <MenuPanel controller={menu} label="Session actions">
        <MenuItem
          role="menuitemcheckbox"
          checked={browserOpen}
          label="Browser"
          leading={<Monitor size={13} strokeWidth={1.7} className="shrink-0 text-muted-foreground" aria-hidden="true" />}
          trailing={browserOpen ? <Check size={12} strokeWidth={2.2} aria-hidden="true" /> : undefined}
          onClick={() => { onToggleBrowser(); menu.close(); }}
        />
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
