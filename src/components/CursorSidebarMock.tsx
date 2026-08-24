import { useState } from "react";
import { Bot, ChevronLeft, ChevronRight, Cloud, Folder, FolderPlus, GitBranch, Home, LayoutGrid, ListFilter, PanelLeft, Search, Settings, SquarePen } from "lucide-react";
import { cn } from "@/lib/utils";

/**
 * Frontend-only look-at mock of a Cursor-style agent sidebar.
 * Swap `SHOW_CURSOR_SIDEBAR_MOCK` in App.tsx to restore the real rail.
 */

type MockChat = {
  id: string;
  title: string;
  time: string;
  git?: boolean;
  cloud?: boolean;
};

type MockRepo = {
  id: string;
  title: string;
  icon: "folder" | "home";
  chats: MockChat[];
};

const MOCK_REPOS: MockRepo[] = [
  {
    id: "bridge-harness",
    title: "bridge-harness",
    icon: "folder",
    chats: [
      { id: "health-bar", title: "Health bar redesign", time: "1m", git: true, cloud: true },
      { id: "nested-corners", title: "Nested corner radii", time: "9m" },
      { id: "usage-unknown", title: "Usage unknown rail", time: "1d", git: true },
      { id: "protocol", title: "Protocol artifacts", time: "5d" },
      { id: "sidecars", title: "Claude sidecar PATH", time: "5d" },
    ],
  },
  {
    id: "vedas",
    title: "vedas",
    icon: "folder",
    chats: [
      { id: "vedas-1", title: "Index the hymn corpus", time: "2d" },
      { id: "vedas-2", title: "Sandbox the parser", time: "1w" },
    ],
  },
  {
    id: "none",
    title: "No Repo",
    icon: "home",
    chats: [
      { id: "plain-1", title: "What should Bridge remember?", time: "3d" },
    ],
  },
];

export type CursorSidebarMockProps = {
  mobileOpen?: boolean;
  onCloseMobile?: () => void;
  workBoardActive: boolean;
  workNeedsYouCount: number;
  onOpenWorkBoard: () => void;
  onOpenNewChat: () => void;
  onOpenProjects: () => void;
  onOpenMarketplace: () => void;
  onOpenSettings: () => void;
};

function IconBtn({ label, onClick, children }: { label: string; onClick?: () => void; children: React.ReactNode }) {
  return (
    <button
      type="button"
      title={label}
      aria-label={label}
      onClick={onClick}
      className="inline-flex size-6 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
    >
      {children}
    </button>
  );
}

function ActionRow({ icon: Icon, label, onClick }: { icon: typeof Search; label: string; onClick?: () => void }) {
  return (
    <button
      type="button"
      onClick={onClick}
      className="flex h-7 w-full items-center gap-2.5 rounded-md px-2 text-[13px] tracking-[-0.008em] text-foreground/85 transition-colors hover:bg-accent"
    >
      <Icon size={15} strokeWidth={1.5} className="shrink-0 text-muted-foreground" aria-hidden="true" />
      {label}
    </button>
  );
}

export function CursorSidebarMock({
  mobileOpen = false,
  onCloseMobile,
  onOpenNewChat,
  onOpenProjects,
  onOpenMarketplace,
  onOpenSettings,
}: CursorSidebarMockProps) {
  const [selectedId, setSelectedId] = useState("health-bar");
  const [openRepos, setOpenRepos] = useState<Set<string>>(() => new Set(["bridge-harness"]));
  const [shownInFull, setShownInFull] = useState<Set<string>>(new Set());
  const [searchOpen, setSearchOpen] = useState(false);
  const [query, setQuery] = useState("");

  const toggleRepo = (id: string) => {
    setOpenRepos(current => {
      const next = new Set(current);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  const needle = query.trim().toLowerCase();

  return (
    <>
      {mobileOpen && (
        <button type="button" className="fixed inset-0 z-30 bg-scrim sm:hidden" onClick={onCloseMobile} aria-label="Close navigation" />
      )}
      <aside
        className="u-vibrancy-sidebar relative z-20 flex h-full w-[16.5rem] shrink-0 flex-col overflow-hidden bg-sidebar font-sans antialiased"
      >
        <div className="flex h-full min-h-0 flex-col px-2 py-2">
          <p className="mb-1.5 px-2 text-[9px] font-medium uppercase tracking-[0.12em] text-muted-foreground/55">Sidebar mock</p>

          <div className="mb-1 flex h-7 shrink-0 items-center gap-0.5 px-0.5" data-tauri-drag-region="deep">
            <IconBtn label="Collapse sidebar">
              <PanelLeft size={15} strokeWidth={1.5} aria-hidden="true" />
            </IconBtn>
            <IconBtn label="Back">
              <ChevronLeft size={15} strokeWidth={1.5} aria-hidden="true" />
            </IconBtn>
            <IconBtn label="Forward">
              <ChevronRight size={15} strokeWidth={1.5} aria-hidden="true" />
            </IconBtn>
          </div>

          <div className="shrink-0">
            <ActionRow icon={SquarePen} label="New Chat" onClick={onOpenNewChat} />
            <ActionRow icon={Search} label="Search" onClick={() => setSearchOpen(open => !open)} />
            <ActionRow icon={Bot} label="Automations" onClick={onOpenMarketplace} />
            <ActionRow icon={LayoutGrid} label="Customize" onClick={onOpenProjects} />
          </div>

          {searchOpen && (
            <div className="relative mt-1.5 shrink-0 px-0.5">
              <Search size={12} strokeWidth={1.5} aria-hidden="true" className="pointer-events-none absolute left-2.5 top-2 text-muted-foreground" />
              <input
                type="text"
                value={query}
                autoFocus
                onChange={event => setQuery(event.target.value)}
                onKeyDown={event => { if (event.key === "Escape") { setSearchOpen(false); setQuery(""); } }}
                placeholder="Filter chats…"
                aria-label="Filter mock chats"
                className="h-7 w-full rounded-md border border-border bg-background pl-7 pr-2 text-[12.5px] text-foreground outline-none placeholder:text-muted-foreground/70 focus:border-ring"
              />
            </div>
          )}

          <div className="mt-3 mb-0.5 flex h-6 shrink-0 items-center px-2">
            <span className="text-[12px] tracking-[-0.006em] text-muted-foreground">Repositories</span>
            <span className="ml-auto flex items-center gap-0.5">
              <IconBtn label="Filter repositories">
                <ListFilter size={13} strokeWidth={1.5} aria-hidden="true" />
              </IconBtn>
              <IconBtn label="New folder" onClick={onOpenProjects}>
                <FolderPlus size={13} strokeWidth={1.5} aria-hidden="true" />
              </IconBtn>
            </span>
          </div>

          <div className="min-h-0 flex-1 overflow-y-auto">
            {MOCK_REPOS.map(repo => {
              const open = openRepos.has(repo.id);
              const chats = needle
                ? repo.chats.filter(chat => chat.title.toLowerCase().includes(needle) || repo.title.toLowerCase().includes(needle))
                : repo.chats;
              const expanded = shownInFull.has(repo.id) || !!needle;
              const visible = expanded ? chats : chats.slice(0, 4);
              const hidden = expanded ? 0 : Math.max(0, chats.length - 4);
              if (needle && chats.length === 0) return null;
              return (
                <div key={repo.id} className="mb-0.5">
                  <button
                    type="button"
                    onClick={() => toggleRepo(repo.id)}
                    className="flex h-7 w-full items-center gap-2 rounded-md px-2 text-left text-[13px] tracking-[-0.008em] text-foreground/90 transition-colors hover:bg-accent"
                  >
                    {repo.icon === "home"
                      ? <Home size={14} strokeWidth={1.5} className="shrink-0 text-muted-foreground" aria-hidden="true" />
                      : <Folder size={14} strokeWidth={1.5} className="shrink-0 text-muted-foreground" aria-hidden="true" />}
                    <span className="min-w-0 truncate">{repo.title}</span>
                  </button>
                  {open && visible.map(chat => {
                    const selected = chat.id === selectedId;
                    return (
                      <button
                        key={chat.id}
                        type="button"
                        onClick={() => setSelectedId(chat.id)}
                        className={cn(
                          "flex h-[26px] w-full items-center gap-1.5 rounded-md pr-2 pl-7 text-left text-[13px] tracking-[-0.008em] transition-colors",
                          selected ? "bg-accent font-medium text-foreground" : "text-foreground/80 hover:bg-accent/70",
                        )}
                      >
                        <span className="min-w-0 flex-1 truncate">{chat.title}</span>
                        {chat.git && <GitBranch size={11} strokeWidth={1.7} className="shrink-0 text-ring" aria-label="On a git branch" />}
                        {chat.cloud && <Cloud size={11} strokeWidth={1.7} className="shrink-0 text-muted-foreground/70" aria-label="Synced" />}
                        <span className="shrink-0 text-[11px] tabular-nums text-muted-foreground/65">{chat.time}</span>
                      </button>
                    );
                  })}
                  {open && hidden > 0 && (
                    <button
                      type="button"
                      onClick={() => setShownInFull(current => new Set(current).add(repo.id))}
                      className="flex h-6 w-full items-center rounded-md pl-7 text-[12px] text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
                    >
                      More
                    </button>
                  )}
                </div>
              );
            })}
          </div>

          <div className="mt-1 shrink-0 pt-1.5">
            <div className="flex h-9 items-center gap-2 rounded-md px-1.5">
              <span
                className="size-6 shrink-0 rounded-full bg-[conic-gradient(from_200deg,#7a8fd4,#d7a0c2,#7eb7c9,#7a8fd4)]"
                aria-hidden="true"
              />
              <span className="min-w-0 flex-1 truncate text-[13px] tracking-[-0.008em] text-foreground">Yashaswi Kumar</span>
              <button
                type="button"
                onClick={onOpenSettings}
                title="Settings"
                aria-label="Settings"
                className="inline-flex size-6 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
              >
                <Settings size={14} strokeWidth={1.5} aria-hidden="true" />
              </button>
            </div>
          </div>
        </div>
      </aside>
    </>
  );
}
