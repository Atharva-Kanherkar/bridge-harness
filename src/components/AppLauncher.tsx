// The launcher: every app, the chats you were just in, and what is waiting on
// you, behind ⌘K and the sidebar's Apps row. Typing also finds actions: the
// keymap's commands, each with its chord, run through the same dispatcher the
// chord does.
//
// One flat list of items drives the keyboard, whatever section an item is
// drawn in, so ArrowDown always means "the next thing on screen". The apps are
// a grid for the eye and still a single step for the keys.

import { useEffect, useId, useMemo, useRef, useState } from "react";
import { ArrowDown, ArrowUp, ChevronRight, CircleStop, FolderPlus, Fullscreen, Keyboard, Maximize2, MessageSquare, NotebookPen, PanelLeft, PanelRight, PanelsRightBottom, Pin, PinOff, Search, Settings, SquarePen, TextSearch, type LucideIcon } from "lucide-react";
import { Dialog, DialogPopup } from "@/components/ui/dialog";
import { Kbd } from "@/components/ui/kbd";
import { cn } from "@/lib/utils";
import { appMatches, badgeText, builtinEntry, isAppId, type AppBadges, type AppKey, type BadgeText, type BuiltinApp, type LaunchEntry } from "../apps/registry";
import type { PinResult } from "../apps/pins";
import { CHAT_SEARCH_MIN_CHARS } from "../chatSearch";
import { DOCK_PANES, dockPaneChord, type DockPaneId } from "../dockLayout";
import { chordLabel, shortcutFor, type CommandId } from "../keymap";
import { formatShortcut } from "../lib/platform";
import { Button } from "@/components/ui/button";

export type LauncherChat = { id: string; title: string; detail?: string };
export type LauncherDockPane = { id: DockPaneId; label: string; icon: LucideIcon };

export type AppLauncherProps = {
  open: boolean;
  onClose: () => void;
  /** The apps that are on, in launcher order. An app that is off is not here. */
  apps: readonly BuiltinApp[];
  /** The person's own apps that are on ("My apps"). */
  personal?: readonly LaunchEntry[];
  badges: AppBadges;
  pins: readonly AppKey[];
  /** Top-level chats, most recent first. The launcher shows at most five. */
  chats: readonly LauncherChat[];
  /** A chat is on screen, so "Search this chat" means something. */
  chatOpen: boolean;
  onOpenApp: (key: AppKey) => void;
  onOpenChat: (id: string) => void;
  onSearchThisChat: () => void;
  /** Hand the query to the sidebar's chat search, deep stage included. */
  onSearchChats: (query: string) => void;
  onTogglePin: (key: AppKey) => Promise<PinResult>;
  /** Run a keymap command through the dispatcher its chord uses. Without it
   *  the launcher lists no actions. `index` picks a member of a digit family. */
  onRunCommand?: (id: CommandId, index?: number) => void;
  /** The dock panes the app offers, in strip order, for Open dock pane. */
  dockPanes?: readonly LauncherDockPane[];
  /** The chats ⌘1-9 open, in that order, for Jump to chat. */
  numberedChats?: readonly LauncherChat[];
};

const MAX_CHATS = 5;
const PIN_CHORD = formatShortcut({ meta: true }, "P");
const NONE: readonly never[] = [];

// ── Actions ──────────────────────────────────────────────────────────────────
// The keymap says what a command is called and which chord runs it; this table
// only adds what the launcher needs to draw and find it. A command lands here or
// in LAUNCHER_SKIPPED with a reason, and the types refuse anything in between.

export type SubmenuId = "dock-panes" | "chats";

type LauncherCommand = {
  id: CommandId;
  icon: LucideIcon;
  /** More words that find it. Its label always does. */
  keywords?: readonly string[];
  /** Listed only while a chat is on screen: the dock and the turn live there. */
  needsChat?: boolean;
  /** Listed only when there is a chat to move to. */
  needsChats?: boolean;
  /** A digit family: opens a list of its members rather than running. */
  submenu?: SubmenuId;
};

export const LAUNCHER_COMMANDS = [
  { id: "new-chat", icon: SquarePen, keywords: ["start", "draft"] },
  { id: "new-project", icon: FolderPlus, keywords: ["add", "folder", "repository"] },
  { id: "capture-note", icon: NotebookPen, keywords: ["jot", "quick"] },
  { id: "interrupt-turn", icon: CircleStop, keywords: ["stop", "cancel"], needsChat: true },
  { id: "jump-to-chat", icon: MessageSquare, keywords: ["go", "switch", "open"], submenu: "chats" },
  { id: "next-chat", icon: ArrowDown, needsChats: true },
  { id: "previous-chat", icon: ArrowUp, needsChats: true },
  { id: "toggle-dock", icon: PanelRight, keywords: ["show", "hide", "close"], needsChat: true },
  { id: "expand-dock", icon: Maximize2, keywords: ["maximize"], needsChat: true },
  { id: "open-dock-pane", icon: PanelsRightBottom, keywords: ["show"], needsChat: true, submenu: "dock-panes" },
  { id: "toggle-sidebar", icon: PanelLeft, keywords: ["show", "hide", "collapse"] },
  { id: "toggle-fullscreen", icon: Fullscreen, keywords: ["focus"] },
  { id: "open-settings", icon: Settings, keywords: ["preferences"] },
  { id: "show-shortcuts", icon: Keyboard, keywords: ["help", "keys", "chords", "hotkeys"] },
] as const satisfies readonly LauncherCommand[];

export type LauncherCommandId = (typeof LAUNCHER_COMMANDS)[number]["id"];

/** Every keymap command the launcher leaves out, and why. */
export const LAUNCHER_SKIPPED: Record<Exclude<CommandId, LauncherCommandId>, string> = {
  "open-launcher": "It is the launcher.",
  "open-recall": "The Search section already offers it as Search this chat.",
  "open-projects": "Projects is an app in the grid, under the same name.",
  "open-notes": "Notes is an app in the grid, under the same name.",
  "toggle-notes-list": "Only the Notes view answers it, and that view has its own button for it.",
  // Zoom is a repeat gesture: one step per trip through the launcher is slower
  // than the chord. Outside the desktop shell the browser owns zoom, and that
  // check sits in the window listener, ahead of the dispatcher this would call.
  "zoom-in": "A repeat gesture the chord does better; the browser owns it outside the shell.",
  "zoom-out": "A repeat gesture the chord does better; the browser owns it outside the shell.",
  "zoom-reset": "Belongs with zoom in and out, which the launcher leaves to their chords.",
};

// The header already names the command, so the list under it names its members.
const SUBMENUS: Record<SubmenuId, { command: LauncherCommandId; section: string; placeholder: string; empty: string }> = {
  "dock-panes": { command: "open-dock-pane", section: "Dock panes", placeholder: "Search panes…", empty: "No pane matches" },
  chats: { command: "jump-to-chat", section: "Chats", placeholder: "Search chats…", empty: "No chat matches" },
};

/** What a row runs. A member of a digit family carries its index. */
export type LauncherAction = {
  id: CommandId;
  index?: number;
  label: string;
  detail?: string;
  chord?: string;
  icon: LucideIcon;
  submenu?: SubmenuId;
};

type ActionContext = { dockPanes: readonly LauncherDockPane[]; numberedChats: readonly LauncherChat[] };

/** The sheet's wording minus the digit range the chord column already shows:
 *  "Open dock pane 1-9" reads "Open dock pane", next to ⌥⌘1-9. */
function commandLabel(id: CommandId): string {
  const shortcut = shortcutFor(id);
  return shortcut.digits && shortcut.displayKey ? shortcut.label.replace(` ${shortcut.displayKey}`, "") : shortcut.label;
}

/** A command is found from the start of any of its words, so two letters do
 *  not list every command that happens to contain them. */
function wordMatch(text: string, needle: string): boolean {
  return ` ${text.toLowerCase()}`.includes(` ${needle}`);
}

function submenuItems(id: SubmenuId, context: ActionContext): LauncherAction[] {
  if (id === "dock-panes") {
    return context.dockPanes.flatMap(pane => {
      const index = DOCK_PANES.indexOf(pane.id);
      if (index < 0) return [];
      const digit = dockPaneChord(pane.id);
      return [{ id: "open-dock-pane" as const, index, label: pane.label, icon: pane.icon, chord: digit ? chordLabel("open-dock-pane", Number(digit)) : undefined }];
    });
  }
  return context.numberedChats.slice(0, 9).map((chat, index) => ({
    id: "jump-to-chat" as const,
    index,
    label: chat.title,
    detail: chat.detail,
    icon: MessageSquare,
    chord: chordLabel("jump-to-chat", index + 1),
  }));
}

type Item =
  | { key: string; kind: "needs"; app: LaunchEntry; badge: BadgeText }
  | { key: string; kind: "app"; app: LaunchEntry }
  | { key: string; kind: "chat"; chat: LauncherChat }
  | { key: string; kind: "search-chat" }
  | { key: string; kind: "search-all"; query: string }
  | { key: string; kind: "command"; action: LauncherAction };

type Section = { id: string; label: string; items: Item[] };

const commandItem = (action: LauncherAction): Item => ({ key: `command:${action.id}${action.index === undefined ? "" : `:${action.index}`}`, kind: "command", action });

/** Actions and Help for a typed query. Empty when nothing is typed: the
 *  launcher's resting layout is apps and chats. */
function actionSections(needle: string, chatOpen: boolean, context: ActionContext): Section[] {
  if (!needle) return [];
  const actions: LauncherAction[] = [];
  const help: { action: LauncherAction; own: boolean }[] = [];
  for (const command of LAUNCHER_COMMANDS as readonly LauncherCommand[]) {
    if (command.needsChat && !chatOpen) continue;
    if (command.needsChats && !context.numberedChats.length) continue;
    const label = commandLabel(command.id);
    const own = wordMatch(label, needle) || (command.keywords ?? []).some(word => wordMatch(word, needle));
    const action: LauncherAction = { id: command.id, label, icon: command.icon, chord: chordLabel(command.id) };
    if (command.submenu) {
      const children = submenuItems(command.submenu, context);
      if (!children.length) continue;
      // A pane or chat name finds its parent, and the detail says which.
      const named = children.filter(child => child.label.toLowerCase().includes(needle));
      if (!own && !named.length) continue;
      actions.push({ ...action, submenu: command.submenu, detail: (own ? children : named).map(child => child.label).join(", ") });
      continue;
    }
    if (shortcutFor(command.id).menu === "Help") help.push({ action, own });
    else if (own) actions.push(action);
  }
  // The sheet lists every chord, so it answers any query that found an action;
  // the detail says where in it to look.
  const first = actions[0];
  const shown = help.flatMap(({ action, own }) => {
    if (own) return [action];
    return first ? [{ ...action, detail: `${shortcutFor(first.id).group} › ${first.label}` }] : [];
  });
  return [
    { id: "commands", label: "Actions", items: actions.map(commandItem) },
    { id: "help", label: "Help", items: shown.map(commandItem) },
  ];
}

export function launcherSections({ query, apps: available, personal = [], badges, chats, chatOpen, actions: context, submenu }: { query: string; apps: readonly BuiltinApp[]; personal?: readonly LaunchEntry[]; badges: AppBadges; chats: readonly LauncherChat[]; chatOpen: boolean; actions?: ActionContext; submenu?: SubmenuId }): Section[] {
  const needle = query.trim().toLowerCase();
  if (submenu && context) {
    const items = submenuItems(submenu, context).filter(item => !needle || item.label.toLowerCase().includes(needle));
    return [{ id: "submenu", label: SUBMENUS[submenu].section, items: items.map(commandItem) }];
  }
  const apps = available.map(builtinEntry).filter(app => appMatches(app, needle));
  const mine = personal.filter(app => appMatches(app, needle));
  const needs = apps.flatMap(app => {
    const badge = isAppId(app.key) ? badgeText(badges[app.key]) : undefined;
    return badge ? [{ key: `needs:${app.key}`, kind: "needs" as const, app, badge }] : [];
  });
  const recent = chats.filter(chat => !needle || chat.title.toLowerCase().includes(needle)).slice(0, MAX_CHATS);
  const actions: Item[] = [];
  if (chatOpen && (!needle || "search this chat".includes(needle))) actions.push({ key: "search-chat", kind: "search-chat" });
  if (needle.length >= CHAT_SEARCH_MIN_CHARS) actions.push({ key: "search-all", kind: "search-all", query: query.trim() });
  // Typed actions sit after the apps, so ⌘K, an app's name and ↵ still opens
  // that app, and before the chats, which a name only narrows.
  const [commands, help] = context ? actionSections(needle, chatOpen, context) : [];
  return [
    { id: "needs", label: "Needs you", items: needs },
    { id: "apps", label: "Apps", items: apps.map(app => ({ key: `app:${app.key}`, kind: "app" as const, app })) },
    { id: "mine", label: "My apps", items: mine.map(app => ({ key: `app:${app.key}`, kind: "app" as const, app })) },
    ...(commands ? [commands] : []),
    { id: "chats", label: "Recent chats", items: recent.map(chat => ({ key: `chat:${chat.id}`, kind: "chat" as const, chat })) },
    { id: "actions", label: "Search", items: actions },
    ...(help ? [help] : []),
  ].filter(section => section.items.length > 0);
}

export function AppLauncher(props: AppLauncherProps) {
  const { open, onClose } = props;
  return (
    <Dialog open={open} onOpenChange={next => { if (!next) onClose(); }}>
      {open && <LauncherPanel {...props} />}
    </Dialog>
  );
}

function LauncherPanel({ onClose, apps, personal, badges, pins, chats, chatOpen, onOpenApp, onOpenChat, onSearchThisChat, onSearchChats, onTogglePin, onRunCommand, dockPanes = NONE, numberedChats = NONE }: AppLauncherProps) {
  const inputRef = useRef<HTMLInputElement>(null);
  const baseId = useId();
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const [status, setStatus] = useState<string | null>(null);
  // An open submenu remembers the list it came from, to go back to it.
  const [submenu, setSubmenu] = useState<{ id: SubmenuId; query: string; active: number } | null>(null);

  const context = useMemo(() => (onRunCommand ? { dockPanes, numberedChats } : undefined), [onRunCommand, dockPanes, numberedChats]);
  const sections = useMemo(() => launcherSections({ query, apps, personal, badges, chats, chatOpen, actions: context, submenu: submenu?.id }), [query, apps, personal, badges, chats, chatOpen, context, submenu?.id]);
  const items = useMemo(() => sections.flatMap(section => section.items), [sections]);
  const current = items[Math.min(active, items.length - 1)];
  const optionId = (item: Item) => `${baseId}-${item.key}`;
  const submenuLabel = submenu ? commandLabel(SUBMENUS[submenu.id].command) : undefined;

  useEffect(() => {
    if (!current) return;
    const element = document.getElementById(optionId(current));
    if (typeof element?.scrollIntoView === "function") element.scrollIntoView({ block: "nearest" });
  });

  // A new query starts from the top: the old index points at nothing useful.
  const search = (value: string) => {
    setQuery(value);
    setActive(0);
    setStatus(null);
  };

  const enter = (action: LauncherAction & { submenu: SubmenuId }) => {
    // Found by a pane or chat name: land on that one, with the list cleared.
    const needle = query.trim().toLowerCase();
    const children = context ? submenuItems(action.submenu, context) : [];
    const named = needle ? children.findIndex(child => child.label.toLowerCase().includes(needle)) : -1;
    setSubmenu({ id: action.submenu, query, active: Math.min(active, items.length - 1) });
    search("");
    setActive(Math.max(0, named));
    inputRef.current?.focus();
  };

  const back = () => {
    if (!submenu) return;
    setSubmenu(null);
    setQuery(submenu.query);
    setActive(submenu.active);
    setStatus(null);
    inputRef.current?.focus();
  };

  const run = (item: Item) => {
    if (item.kind === "command" && item.action.submenu) {
      enter({ ...item.action, submenu: item.action.submenu });
      return;
    }
    onClose();
    if (item.kind === "needs" || item.kind === "app") onOpenApp(item.app.key);
    else if (item.kind === "chat") onOpenChat(item.chat.id);
    else if (item.kind === "search-chat") onSearchThisChat();
    else if (item.kind === "command") onRunCommand?.(item.action.id, item.action.index);
    else onSearchChats(item.query);
  };

  const pin = (item: Item | undefined) => {
    if (!item || (item.kind !== "needs" && item.kind !== "app")) return;
    pinApp(item.app);
  };

  const pinApp = (app: LaunchEntry) => {
    const wasPinned = pins.includes(app.key);
    void onTogglePin(app.key).then(result => {
      setStatus(!result.ok ? result.message : wasPinned ? `Unpinned ${app.name}.` : `Pinned ${app.name} to the sidebar.`);
    });
  };

  // A key the launcher acts on is the launcher's alone. Window listeners
  // underneath (the Code pane answers ⌘P with its file palette) must not
  // also act on it, and some never check defaultPrevented.
  const take = (event: React.KeyboardEvent) => {
    event.preventDefault();
    event.stopPropagation();
  };

  const onKeyDown = (event: React.KeyboardEvent<HTMLInputElement>) => {
    if (event.nativeEvent.isComposing) return;
    const atEnd = event.currentTarget.selectionStart === query.length;
    const atStart = event.currentTarget.selectionStart === 0;
    const modified = event.metaKey || event.ctrlKey || event.altKey || event.shiftKey;
    // → on a row with a submenu opens it; ← from the start of the box, or ⌫ in
    // an empty one, comes back. Anywhere else the arrows keep moving.
    if (event.key === "ArrowRight" && atEnd && !modified && current?.kind === "command" && current.action.submenu) {
      take(event);
      run(current);
      return;
    }
    if (submenu && !modified && (event.key === "ArrowLeft" && atStart || event.key === "Backspace" && query === "")) {
      take(event);
      back();
      return;
    }
    const step = event.key === "ArrowDown" || event.key === "ArrowRight" && atEnd ? 1
      : event.key === "ArrowUp" || event.key === "ArrowLeft" && atStart ? -1
      : 0;
    if (step && items.length) {
      take(event);
      setActive(index => (Math.min(index, items.length - 1) + step + items.length) % items.length);
      return;
    }
    if (event.key === "Enter" && current) {
      take(event);
      run(current);
      return;
    }
    if (event.key.toLowerCase() === "p" && (event.metaKey || event.ctrlKey) && !event.altKey && !event.shiftKey) {
      take(event);
      pin(current);
    }
  };

  const optionProps = (item: Item) => ({
    id: optionId(item),
    role: "option" as const,
    "aria-selected": item === current,
    "data-active": item === current ? "" : undefined,
    onMouseMove: () => { const index = items.indexOf(item); if (index !== active) setActive(index); },
    onClick: () => run(item),
  });

  return (
    <DialogPopup showCloseButton={false} initialFocus={inputRef} aria-label="Apps" className="max-h-[min(640px,80dvh)] max-w-[680px] overflow-hidden">
      <div className="flex h-14 shrink-0 items-center gap-3 border-b border-border px-4">
        <Search size={17} strokeWidth={1.6} className="shrink-0 text-muted-foreground" aria-hidden="true" />
        {/* Where the submenu came from, and the pointer's way back to it. The
            box keeps focus: ⌫ is the keyboard's way. */}
        {submenu && <Button variant="quiet" size="xs" className="-mx-1 shrink-0" tooltip="Back"
         
          tabIndex={-1}
          aria-label={`Back from ${submenuLabel}`}
         
          onMouseDown={event => event.preventDefault()}
          onClick={back}
         
        >
          {submenuLabel}
          <ChevronRight size={15} strokeWidth={1.6} aria-hidden="true" />
        </Button>}
        <input
          ref={inputRef}
          value={query}
          onChange={event => search(event.target.value)}
          onKeyDown={onKeyDown}
          role="combobox"
          aria-expanded="true"
          aria-controls={`${baseId}-list`}
          aria-activedescendant={current ? optionId(current) : undefined}
          aria-label={submenu ? `Search ${submenuLabel}` : "Search apps and chats"}
          placeholder={submenu ? SUBMENUS[submenu.id].placeholder : context ? "Search apps, chats and actions…" : "Search apps and chats…"}
          spellCheck={false}
          className="min-w-0 flex-1 bg-transparent text-message text-foreground outline-none placeholder:text-muted-foreground/70"
        />
        <Kbd>esc</Kbd>
      </div>

      <div id={`${baseId}-list`} role="listbox" aria-label={submenu ? submenuLabel : "Apps and chats"} className="min-h-0 flex-1 overflow-y-auto">
        {submenu
          ? !items.length && <p className="px-5 pb-2 pt-5 text-center text-ui text-muted-foreground">{SUBMENUS[submenu.id].empty} “{query.trim()}”.</p>
          : !sections.some(section => section.id !== "actions") && <p className="px-5 pb-2 pt-5 text-center text-ui text-muted-foreground">No app or chat matches “{query.trim()}”.</p>}
        {sections.map(section => (
          <div key={section.id} role="group" aria-labelledby={`${baseId}-${section.id}`} data-launcher-section={section.id} className="border-b border-border px-2 py-2.5 last:border-b-0">
            <p id={`${baseId}-${section.id}`} className="px-2 pb-1.5 text-caption font-medium text-faint">{section.label}</p>
            {section.id === "apps" || section.id === "mine"
              ? <div className="grid grid-cols-4 gap-1 sm:grid-cols-5">{section.items.map(item => item.kind === "app" && <AppTile key={item.key} app={item.app} pinned={pins.includes(item.app.key)} badge={isAppId(item.app.key) ? badgeText(badges[item.app.key]) : undefined} active={item === current} onPin={() => pinApp(item.app)} {...optionProps(item)} />)}</div>
              : section.items.map(item => <Row key={item.key} item={item} active={item === current} {...optionProps(item)} />)}
          </div>
        ))}
      </div>

      <div className="flex h-10 shrink-0 items-center gap-4 border-t border-border bg-muted/30 px-4 text-caption text-muted-foreground">
        {/* One live region for the life of the panel, so a pin result is
            announced; the key hints step aside while it speaks. */}
        <p role="status" className={status ? "min-w-0 flex-1 truncate text-foreground" : "sr-only"}>{status}</p>
        {!status && <>
          <span className="flex items-center gap-1.5"><Kbd>↑↓</Kbd>Move</span>
          <span className="flex items-center gap-1.5"><Kbd>↵</Kbd>Run</span>
          <span className="hidden items-center gap-1.5 sm:flex"><Kbd>→</Kbd>Open submenu</span>
          <span className="hidden items-center gap-1.5 sm:flex"><Kbd>⌫</Kbd>Back</span>
          {items.some(item => item.kind === "app" || item.kind === "needs") && <span className="flex items-center gap-1.5"><Kbd>{PIN_CHORD}</Kbd>Pin to sidebar</span>}
        </>}
        <span className="ml-auto flex items-center gap-1.5"><Kbd>{chordLabel("open-launcher")}</Kbd>Close</span>
      </div>
    </DialogPopup>
  );
}

type OptionProps = { id: string; role: "option"; "aria-selected": boolean; "data-active"?: string; onMouseMove: () => void; onClick: () => void };

function AppTile({ app, pinned, badge, active, onPin, ...option }: { app: LaunchEntry; pinned: boolean; badge?: BadgeText; active: boolean; onPin: () => void } & OptionProps) {
  const Icon = app.icon;
  return (
    <div {...option} aria-label={`${app.name}${pinned ? ", pinned" : ""}${badge ? `, ${badge.label}` : ""}`} className={cn("group/tile relative flex cursor-default flex-col items-center gap-2 rounded-xl px-1 pb-2.5 pt-3 text-center", active ? "bg-selection text-selection-foreground" : "text-foreground/90")}>
      <span className="relative flex size-11 items-center justify-center rounded-xl border border-border-card bg-card shadow-control">
        <Icon size={19} strokeWidth={1.5} aria-hidden="true" />
        {badge && <span aria-hidden="true" className="absolute -right-1.5 -top-1.5 min-w-4 rounded-full bg-foreground px-1 text-micro font-semibold leading-4 text-background tabular-nums">{badge.count}</span>}
      </span>
      <span className="text-meta leading-tight">{app.name}</span>
      {/* The pin for a pointer. The tile stays the listbox option and ⌘P the
          keyboard path, so the button is no tab stop, keeps focus in the
          search box, and never opens the app it sits on. */}
      <button
        type="button"
        tabIndex={-1}
        aria-label={pinned ? `Unpin ${app.name}` : `Pin ${app.name} to the sidebar`}
        title={pinned ? `Unpin ${app.name}` : `Pin to sidebar  ${PIN_CHORD}`}
        onMouseDown={event => event.preventDefault()}
        onClick={event => { event.stopPropagation(); onPin(); }}
        className={cn(
          "group/pin absolute right-1.5 top-1.5 grid size-6 place-items-center rounded-md text-muted-foreground transition-opacity hover:bg-foreground/10 hover:text-foreground",
          pinned || active ? "opacity-100" : "opacity-0 group-hover/tile:opacity-100",
        )}
      >
        {pinned
          ? <>
              <Pin size={11} aria-hidden="true" className="fill-current group-hover/pin:hidden" />
              <PinOff size={11} aria-hidden="true" className="hidden group-hover/pin:block" />
            </>
          : <Pin size={11} aria-hidden="true" />}
      </button>
    </div>
  );
}

function Row({ item, active, ...option }: { item: Item; active: boolean } & OptionProps) {
  const base = cn("flex h-10 cursor-default items-center gap-3 rounded-lg px-2 text-ui", active ? "bg-selection text-selection-foreground" : "text-foreground");
  if (item.kind === "needs") {
    const Icon = item.app.icon;
    return <div {...option} className={base}>
      <Icon size={15} strokeWidth={1.6} className="shrink-0 text-muted-foreground" aria-hidden="true" />
      <span className="font-medium">{item.app.name}</span>
      <span className="min-w-0 truncate text-muted-foreground">{item.badge.label}</span>
      <span aria-hidden="true" className="ml-auto rounded-full bg-foreground/10 px-1.5 text-caption tabular-nums text-muted-foreground">{item.badge.count}</span>
    </div>;
  }
  if (item.kind === "chat") {
    return <div {...option} className={base}>
      <MessageSquare size={14} strokeWidth={1.6} className="shrink-0 text-muted-foreground" aria-hidden="true" />
      <span className="min-w-0 truncate">{item.chat.title}</span>
      {item.chat.detail && <span className="min-w-0 truncate text-muted-foreground">{item.chat.detail}</span>}
    </div>;
  }
  if (item.kind === "search-chat") {
    return <div {...option} className={base}>
      <TextSearch size={14} strokeWidth={1.6} className="shrink-0 text-muted-foreground" aria-hidden="true" />
      <span>Search this chat</span>
      <Kbd className="ml-auto">{chordLabel("open-recall")}</Kbd>
    </div>;
  }
  if (item.kind === "search-all") {
    return <div {...option} className={base}>
      <Search size={14} strokeWidth={1.6} className="shrink-0 text-muted-foreground" aria-hidden="true" />
      <span className="min-w-0 truncate">Search all chats for “{item.query}”</span>
    </div>;
  }
  if (item.kind === "command") {
    const { action } = item;
    const Icon = action.icon;
    return <div {...option} data-command={action.id} className={base}>
      <Icon size={14} strokeWidth={1.6} className="shrink-0 text-muted-foreground" aria-hidden="true" />
      <span className="min-w-0 truncate">{action.label}</span>
      {/* Zero basis: the detail takes what the label leaves and truncates first. */}
      {action.detail && <span className="min-w-0 flex-1 truncate text-meta text-faint">{action.detail}</span>}
      {(action.chord || action.submenu) && <span className="ml-auto flex shrink-0 items-center gap-1.5">
        {action.chord && <Kbd>{action.chord}</Kbd>}
        {action.submenu && <ChevronRight size={14} strokeWidth={1.6} className="text-muted-foreground" aria-hidden="true" />}
      </span>}
    </div>;
  }
  return null;
}
