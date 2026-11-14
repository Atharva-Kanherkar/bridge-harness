// The launcher: every app, the chats you were just in, and what is waiting on
// you, behind ⌘K and the sidebar's Apps row.
//
// One flat list of items drives the keyboard, whatever section an item is
// drawn in, so ArrowDown always means "the next thing on screen". The apps are
// a grid for the eye and still a single step for the keys.

import { useEffect, useId, useMemo, useRef, useState } from "react";
import { MessageSquare, Pin, Search, TextSearch } from "lucide-react";
import { Dialog, DialogPopup } from "@/components/ui/dialog";
import { Kbd } from "@/components/ui/kbd";
import { cn } from "@/lib/utils";
import { appMatches, badgeText, BUILTIN_APPS, type AppBadges, type AppId, type BadgeText, type BuiltinApp } from "../apps/registry";
import { PIN_LIMIT_MESSAGE, type PinResult } from "../apps/pins";
import { CHAT_SEARCH_MIN_CHARS } from "../chatSearch";
import { chordLabel } from "../keymap";
import { formatShortcut } from "../lib/platform";

export type LauncherChat = { id: string; title: string; detail?: string };

export type AppLauncherProps = {
  open: boolean;
  onClose: () => void;
  badges: AppBadges;
  pins: readonly AppId[];
  /** Top-level chats, most recent first. The launcher shows at most five. */
  chats: readonly LauncherChat[];
  /** A chat is on screen, so "Search this chat" means something. */
  chatOpen: boolean;
  onOpenApp: (id: AppId) => void;
  onOpenChat: (id: string) => void;
  onSearchThisChat: () => void;
  /** Hand the query to the sidebar's chat search, deep stage included. */
  onSearchChats: (query: string) => void;
  onTogglePin: (id: AppId) => PinResult;
};

const MAX_CHATS = 5;
const PIN_CHORD = formatShortcut({ meta: true }, "P");

type Item =
  | { key: string; kind: "needs"; app: BuiltinApp; badge: BadgeText }
  | { key: string; kind: "app"; app: BuiltinApp }
  | { key: string; kind: "chat"; chat: LauncherChat }
  | { key: string; kind: "search-chat" }
  | { key: string; kind: "search-all"; query: string };

type Section = { id: string; label: string; items: Item[] };

export function launcherSections({ query, badges, chats, chatOpen }: { query: string; badges: AppBadges; chats: readonly LauncherChat[]; chatOpen: boolean }): Section[] {
  const needle = query.trim().toLowerCase();
  const apps = BUILTIN_APPS.filter(app => appMatches(app, needle));
  const needs = apps.flatMap(app => {
    const badge = badgeText(badges[app.id]);
    return badge ? [{ key: `needs:${app.id}`, kind: "needs" as const, app, badge }] : [];
  });
  const recent = chats.filter(chat => !needle || chat.title.toLowerCase().includes(needle)).slice(0, MAX_CHATS);
  const actions: Item[] = [];
  if (chatOpen && (!needle || "search this chat".includes(needle))) actions.push({ key: "search-chat", kind: "search-chat" });
  if (needle.length >= CHAT_SEARCH_MIN_CHARS) actions.push({ key: "search-all", kind: "search-all", query: query.trim() });
  return [
    { id: "needs", label: "Needs you", items: needs },
    { id: "apps", label: "Apps", items: apps.map(app => ({ key: `app:${app.id}`, kind: "app" as const, app })) },
    { id: "chats", label: "Recent chats", items: recent.map(chat => ({ key: `chat:${chat.id}`, kind: "chat" as const, chat })) },
    { id: "actions", label: "Search", items: actions },
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

function LauncherPanel({ onClose, badges, pins, chats, chatOpen, onOpenApp, onOpenChat, onSearchThisChat, onSearchChats, onTogglePin }: AppLauncherProps) {
  const inputRef = useRef<HTMLInputElement>(null);
  const baseId = useId();
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const [status, setStatus] = useState<string | null>(null);

  const sections = useMemo(() => launcherSections({ query, badges, chats, chatOpen }), [query, badges, chats, chatOpen]);
  const items = useMemo(() => sections.flatMap(section => section.items), [sections]);
  const current = items[Math.min(active, items.length - 1)];
  const optionId = (item: Item) => `${baseId}-${item.key}`;

  // A new query starts from the top: the old index points at nothing useful.
  useEffect(() => { setActive(0); }, [query]);
  useEffect(() => {
    if (!current) return;
    const element = document.getElementById(optionId(current));
    if (typeof element?.scrollIntoView === "function") element.scrollIntoView({ block: "nearest" });
  });

  const run = (item: Item) => {
    onClose();
    if (item.kind === "needs" || item.kind === "app") onOpenApp(item.app.id);
    else if (item.kind === "chat") onOpenChat(item.chat.id);
    else if (item.kind === "search-chat") onSearchThisChat();
    else onSearchChats(item.query);
  };

  const pin = (item: Item | undefined) => {
    if (!item || (item.kind !== "needs" && item.kind !== "app")) return;
    const wasPinned = pins.includes(item.app.id);
    const result = onTogglePin(item.app.id);
    setStatus(!result.ok ? PIN_LIMIT_MESSAGE : wasPinned ? `Unpinned ${item.app.name}.` : `Pinned ${item.app.name} to the sidebar.`);
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
    const step = event.key === "ArrowDown" || event.key === "ArrowRight" && event.currentTarget.selectionStart === query.length ? 1
      : event.key === "ArrowUp" || event.key === "ArrowLeft" && event.currentTarget.selectionStart === 0 ? -1
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
        <input
          ref={inputRef}
          value={query}
          onChange={event => { setQuery(event.target.value); setStatus(null); }}
          onKeyDown={onKeyDown}
          role="combobox"
          aria-expanded="true"
          aria-controls={`${baseId}-list`}
          aria-activedescendant={current ? optionId(current) : undefined}
          aria-label="Search apps and chats"
          placeholder="Search apps and chats…"
          spellCheck={false}
          className="min-w-0 flex-1 bg-transparent text-[15px] text-foreground outline-none placeholder:text-muted-foreground/70"
        />
        <Kbd>esc</Kbd>
      </div>

      <div id={`${baseId}-list`} role="listbox" aria-label="Apps and chats" className="min-h-0 flex-1 overflow-y-auto">
        {!sections.some(section => section.id !== "actions") && <p className="px-5 pb-2 pt-5 text-center text-[13px] text-muted-foreground">No app or chat matches “{query.trim()}”.</p>}
        {sections.map(section => (
          <div key={section.id} role="group" aria-labelledby={`${baseId}-${section.id}`} data-launcher-section={section.id} className="border-b border-border px-2 py-2.5 last:border-b-0">
            <p id={`${baseId}-${section.id}`} className="px-2 pb-1.5 text-[11px] font-medium uppercase tracking-[0.06em] text-faint">{section.label}</p>
            {section.id === "apps"
              ? <div className="grid grid-cols-4 gap-1 sm:grid-cols-5">{section.items.map(item => item.kind === "app" && <AppTile key={item.key} app={item.app} pinned={pins.includes(item.app.id)} badge={badgeText(badges[item.app.id])} active={item === current} {...optionProps(item)} />)}</div>
              : section.items.map(item => <Row key={item.key} item={item} active={item === current} {...optionProps(item)} />)}
          </div>
        ))}
      </div>

      <div className="flex h-10 shrink-0 items-center gap-4 border-t border-border bg-muted/30 px-4 text-[11.5px] text-muted-foreground">
        {/* One live region for the life of the panel, so a pin result is
            announced; the key hints step aside while it speaks. */}
        <p role="status" className={status ? "min-w-0 flex-1 truncate text-foreground" : "sr-only"}>{status}</p>
        {!status && <>
          <span className="flex items-center gap-1.5"><Kbd>↵</Kbd>Open</span>
          <span className="flex items-center gap-1.5"><Kbd>{PIN_CHORD}</Kbd>Pin to sidebar</span>
        </>}
        <span className="ml-auto flex items-center gap-1.5"><Kbd>{chordLabel("open-launcher")}</Kbd>Close</span>
      </div>
    </DialogPopup>
  );
}

type OptionProps = { id: string; role: "option"; "aria-selected": boolean; "data-active"?: string; onMouseMove: () => void; onClick: () => void };

function AppTile({ app, pinned, badge, active, ...option }: { app: BuiltinApp; pinned: boolean; badge?: BadgeText; active: boolean } & OptionProps) {
  const Icon = app.icon;
  return (
    <div {...option} aria-label={`${app.name}${pinned ? ", pinned" : ""}${badge ? `, ${badge.label}` : ""}`} className={cn("relative flex cursor-default flex-col items-center gap-2 rounded-xl px-1 pb-2.5 pt-3 text-center", active ? "bg-selection text-selection-foreground" : "text-foreground/90")}>
      <span className="relative flex size-11 items-center justify-center rounded-[12px] border border-border-card bg-card shadow-control">
        <Icon size={19} strokeWidth={1.5} aria-hidden="true" />
        {badge && <span aria-hidden="true" className="absolute -right-1.5 -top-1.5 min-w-4 rounded-full bg-foreground px-1 text-[10px] font-semibold leading-4 text-background tabular-nums">{badge.count}</span>}
      </span>
      <span className="text-[12px] leading-tight">{app.name}</span>
      {pinned && <Pin size={10} aria-hidden="true" className="absolute right-2 top-2 fill-current text-muted-foreground" />}
    </div>
  );
}

function Row({ item, active, ...option }: { item: Item; active: boolean } & OptionProps) {
  const base = cn("flex h-10 cursor-default items-center gap-3 rounded-lg px-2 text-[13px]", active ? "bg-selection text-selection-foreground" : "text-foreground");
  if (item.kind === "needs") {
    const Icon = item.app.icon;
    return <div {...option} className={base}>
      <Icon size={15} strokeWidth={1.6} className="shrink-0 text-muted-foreground" aria-hidden="true" />
      <span className="font-medium">{item.app.name}</span>
      <span className="min-w-0 truncate text-muted-foreground">{item.badge.label}</span>
      <span aria-hidden="true" className="ml-auto rounded-full bg-foreground/10 px-1.5 text-[11px] tabular-nums text-muted-foreground">{item.badge.count}</span>
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
  return null;
}
