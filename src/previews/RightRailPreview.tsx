import { useMemo, useState, type ReactNode } from "react";
import {
  Bot,
  ChevronDown,
  Cloud,
  Folder,
  FolderGit2,
  FolderPlus,
  GitBranch,
  GitFork,
  Laptop,
  LayoutGrid,
  Mic,
  PanelRight,
  Pin,
  Plus,
  Search,
  Settings2,
  SquarePen,
  Terminal,
  type LucideIcon,
} from "lucide-react";
import { WindowHistoryChevrons } from "../components/WindowNavButtons";
import { cn } from "@/lib/utils";

type HostId = "local" | "cloud" | "ssh";
type Surface = "new-chat" | "chat" | "automations" | "mission-control" | "settings" | "projects" | "memory";
type MenuId = "repo" | "branch" | "host" | null;
type Frame = "windowed" | "fullscreen";

type Repo = {
  id: string;
  name: string;
  branches: string[];
  branch: string;
  chats: { id: string; title: string; time: string; status: "idle" | "working" | "waiting" }[];
};

const HOSTS: { id: HostId; label: string; icon: LucideIcon; hint: string }[] = [
  { id: "local", label: "This Mac", icon: Laptop, hint: "Run the agent on this machine" },
  { id: "cloud", label: "Cloud", icon: Cloud, hint: "Draft — not wired yet" },
  { id: "ssh", label: "SSH", icon: Terminal, hint: "Draft — terminal remote, not wired yet" },
];

const INITIAL_REPOS: Repo[] = [
  {
    id: "bridge-harness",
    name: "bridge-harness",
    branch: "feat/cursor-sidebar-dev",
    branches: ["feat/cursor-sidebar-dev", "main"],
    chats: [
      { id: "c1", title: "Right-rail layout", time: "4m", status: "working" },
      { id: "c2", title: "Composer host chips", time: "2h", status: "idle" },
      { id: "c3", title: "Automations entry", time: "1d", status: "waiting" },
    ],
  },
  {
    id: "travel-assistance",
    name: "travel-assistance",
    branch: "main",
    branches: ["main", "feat/itinerary"],
    chats: [
      { id: "c4", title: "Fix auth redirect", time: "3d", status: "idle" },
      { id: "c5", title: "New chat", time: "5d", status: "idle" },
    ],
  },
];

function StatusDot({ status }: { status: Repo["chats"][number]["status"] }) {
  const color = status === "working" ? "bg-success" : status === "waiting" ? "bg-warning" : "bg-muted-foreground/25";
  return <span className={cn("h-1.5 w-1.5 shrink-0 rounded-full", color)} />;
}

function Chip({
  icon: Icon,
  label,
  pressed,
  expanded,
  onClick,
}: {
  icon: LucideIcon;
  label: string;
  pressed?: boolean;
  expanded?: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      aria-pressed={pressed}
      aria-expanded={expanded}
      onClick={onClick}
      className={cn(
        "inline-flex h-7 max-w-[16rem] items-center gap-1.5 rounded-md px-2 text-[12.5px] tracking-[-0.01em] transition-colors",
        pressed || expanded ? "bg-accent text-foreground" : "text-muted-foreground hover:bg-accent hover:text-foreground",
      )}
    >
      <Icon size={13} strokeWidth={1.7} className="shrink-0" aria-hidden="true" />
      <span className="min-w-0 truncate">{label}</span>
      {expanded !== undefined && <ChevronDown size={12} strokeWidth={1.7} className="shrink-0 opacity-70" aria-hidden="true" />}
    </button>
  );
}

function ActionRow({
  icon: Icon,
  label,
  collapsed,
  active,
  onClick,
}: {
  icon: LucideIcon;
  label: string;
  collapsed: boolean;
  active?: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      title={label}
      aria-label={label}
      aria-current={active ? "page" : undefined}
      className={cn(
        "flex items-center rounded-md text-[13px] tracking-[-0.008em] transition-colors",
        collapsed ? "mx-auto size-9 justify-center" : "h-7 w-full gap-2.5 px-2",
        active ? "bg-accent font-medium text-foreground" : "text-foreground/85 hover:bg-accent hover:text-foreground",
      )}
    >
      <Icon size={15} strokeWidth={1.5} className="shrink-0 text-muted-foreground" aria-hidden="true" />
      {!collapsed && label}
    </button>
  );
}

function Menu({
  title,
  children,
}: {
  title: string;
  children: ReactNode;
}) {
  return (
    <div className="u-glass-popover absolute top-9 z-20 min-w-52 overflow-hidden rounded-xl py-1" role="menu" aria-label={title}>
      {children}
    </div>
  );
}

function MenuItem({
  active,
  onClick,
  children,
}: {
  active?: boolean;
  onClick: () => void;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      role="menuitem"
      onClick={onClick}
      className={cn(
        "flex w-full items-center gap-2 px-3 py-2 text-left text-[13px] transition-colors",
        active ? "bg-accent text-foreground" : "text-foreground/85 hover:bg-accent",
      )}
    >
      {children}
    </button>
  );
}

export function RightRailPreview() {
  const [frame, setFrame] = useState<Frame>("windowed");
  const [collapsed, setCollapsed] = useState(false);
  const [surface, setSurface] = useState<Surface>("new-chat");
  const [repos, setRepos] = useState(INITIAL_REPOS);
  const [repoId, setRepoId] = useState(INITIAL_REPOS[0].id);
  const [host, setHost] = useState<HostId>("local");
  const [worktree, setWorktree] = useState(false);
  const [draft, setDraft] = useState("");
  const [query, setQuery] = useState("");
  const [searchOpen, setSearchOpen] = useState(false);
  const [menu, setMenu] = useState<MenuId>(null);
  const [activeChatId, setActiveChatId] = useState<string | null>(null);

  const repo = repos.find(item => item.id === repoId) ?? repos[0];
  const hostMeta = HOSTS.find(item => item.id === host) ?? HOSTS[0];
  const HostIcon = hostMeta.icon;
  const filtered = useMemo(() => {
    const needle = query.trim().toLowerCase();
    if (!needle) return repos;
    return repos
      .map(item => ({
        ...item,
        chats: item.chats.filter(chat => chat.title.toLowerCase().includes(needle) || item.name.toLowerCase().includes(needle)),
      }))
      .filter(item => item.chats.length > 0 || item.name.toLowerCase().includes(needle));
  }, [query, repos]);

  const openNewChat = () => {
    const id = `new-${Date.now()}`;
    setRepos(current => current.map(item => item.id === repoId
      ? { ...item, chats: [{ id, title: "New chat", time: "now", status: "idle" as const }, ...item.chats] }
      : item));
    setActiveChatId(id);
    setSurface("new-chat");
    setDraft("");
    setMenu(null);
  };

  const openChat = (nextRepoId: string, chatId: string) => {
    setRepoId(nextRepoId);
    setActiveChatId(chatId);
    setSurface("chat");
    setMenu(null);
  };

  const setBranch = (branch: string) => {
    setRepos(current => current.map(item => item.id === repoId ? { ...item, branch } : item));
    setMenu(null);
  };

  const fullscreen = frame === "fullscreen";

  return (
    <div className="flex h-[100dvh] flex-col bg-muted text-foreground">
      <header className="flex h-10 shrink-0 items-center gap-3 border-b border-border bg-background px-3">
        <p className="min-w-0 flex-1 truncate text-[12px] text-muted-foreground">
          Look-at preview — not the real app. Click through, then tell me what to change.
        </p>
        <div className="u-segmented" aria-label="Window frame">
          <button type="button" className="u-segmented-item" data-active={frame === "windowed"} onClick={() => setFrame("windowed")}>Windowed</button>
          <button type="button" className="u-segmented-item" data-active={frame === "fullscreen"} onClick={() => setFrame("fullscreen")}>Fullscreen</button>
        </div>
      </header>

      <div className={cn("flex min-h-0 flex-1", fullscreen ? "p-0" : "items-center justify-center p-5")}>
        <div
          data-preview-frame={frame}
          className={cn(
            "flex min-h-0 overflow-hidden",
            fullscreen
              ? "h-full w-full rounded-none"
              : "h-[min(52rem,100%)] w-[min(72rem,100%)] rounded-window border border-border",
          )}
        >
          <section
            aria-label="Canvas"
            className="relative flex min-h-0 min-w-0 flex-1 flex-col bg-background"
          >
            <div className="flex h-11 shrink-0 items-center gap-2 pl-24 pr-3" data-tauri-drag-region="deep">
              <div className="absolute left-3.5 flex items-center gap-1.5" aria-hidden="true">
                <span className="size-3 rounded-full bg-destructive" />
                <span className="size-3 rounded-full bg-warning" />
                <span className="size-3 rounded-full bg-success" />
              </div>
              <p className="min-w-0 flex-1 truncate text-[12.5px] font-medium text-foreground">
                {surface === "automations" ? "Automations" : surface === "mission-control" ? "Mission Control" : surface === "settings" ? "Settings" : surface === "projects" ? "Projects" : surface === "memory" ? "Memory" : repo.name}
              </p>
            </div>

            {surface === "automations" ? <AutomationsCanvas /> : surface === "settings" ? (
              <Placeholder title="Settings" body="Account and app customization live behind the user row." />
            ) : surface === "mission-control" ? (
              <Placeholder title="Mission Control" body="See every active agent and workspace at a glance." />
            ) : surface === "projects" ? (
              <Placeholder title="Projects" body="Projects stays near the primary navigation." />
            ) : surface === "memory" ? (
              <Placeholder title="Memory" body="Account memory stays available without a workspace." />
            ) : (
              <ComposerCanvas
                surface={surface}
                repo={repo}
                repos={repos}
                host={host}
                hostMeta={hostMeta}
                HostIcon={HostIcon}
                worktree={worktree}
                draft={draft}
                menu={menu}
                activeChatId={activeChatId}
                onDraft={setDraft}
                onMenu={setMenu}
                onRepo={id => { setRepoId(id); setMenu(null); }}
                onBranch={setBranch}
                onHost={id => { setHost(id); setMenu(null); }}
                onWorktree={() => setWorktree(value => !value)}
                onCloudShortcut={() => { setHost("cloud"); setMenu(null); }}
              />
            )}
          </section>

          <aside
            aria-label="Sidebar"
            data-preview-rail="right"
            className={cn(
              "relative z-20 flex shrink-0 flex-col overflow-hidden border-l border-sidebar-border bg-sidebar font-sans antialiased",
              collapsed ? "w-[68px]" : "w-[248px]",
            )}
          >
            <div className={cn("flex h-11 shrink-0 items-center gap-0.5", collapsed ? "justify-center" : "px-1.5")}>
              <button
                type="button"
                aria-label={collapsed ? "Show sidebar" : "Hide sidebar"}
                onClick={() => setCollapsed(value => !value)}
                className="inline-flex size-7 shrink-0 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
              >
                <PanelRight size={15} strokeWidth={1.5} aria-hidden="true" />
              </button>
              {!collapsed && (
                <div className="ml-auto">
                  <WindowHistoryChevrons canBack={false} canForward={false} onBack={() => {}} onForward={() => {}} />
                </div>
              )}
            </div>

            <div className={cn("flex min-h-0 flex-1 flex-col px-2 pb-3", collapsed && "items-center")}>
              <div className={cn("mb-2 shrink-0", collapsed && "flex flex-col items-center")}>
                <ActionRow icon={SquarePen} label="New Chat" collapsed={collapsed} onClick={openNewChat} active={surface === "new-chat"} />
                <ActionRow icon={Search} label="Search" collapsed={collapsed} onClick={() => setSearchOpen(open => !open)} />
                <ActionRow icon={Bot} label="Automations" collapsed={collapsed} onClick={() => setSurface("automations")} active={surface === "automations"} />
                <ActionRow icon={LayoutGrid} label="Mission Control" collapsed={collapsed} onClick={() => setSurface("mission-control")} active={surface === "mission-control"} />
                <ActionRow icon={FolderGit2} label="Projects" collapsed={collapsed} onClick={() => setSurface("projects")} active={surface === "projects"} />
                <ActionRow icon={Pin} label="Memory" collapsed={collapsed} onClick={() => setSurface("memory")} active={surface === "memory"} />
              </div>

              {!collapsed && searchOpen && (
                <div className="relative mb-2 shrink-0">
                  <Search size={13} strokeWidth={1.7} aria-hidden="true" className="pointer-events-none absolute left-2 top-2 text-muted-foreground" />
                  <input
                    type="text"
                    value={query}
                    onChange={event => setQuery(event.target.value)}
                    placeholder="Filter repositories…"
                    aria-label="Filter repositories"
                    className="h-7 w-full rounded-lg border border-border bg-background pl-7 pr-2 text-[13px] text-foreground outline-none placeholder:text-muted-foreground/70 focus:border-ring"
                  />
                </div>
              )}

              <div className="min-h-0 flex-1 overflow-y-auto">
                {!collapsed && (
                  <div className="mb-1 flex h-7 items-center justify-between px-2">
                    <span className="text-[11px] font-medium uppercase tracking-[0.08em] text-muted-foreground">Repositories</span>
                    <button type="button" aria-label="New folder" onClick={() => setSurface("projects")} className="inline-flex size-6 items-center justify-center rounded-md text-muted-foreground hover:bg-accent hover:text-foreground">
                      <FolderPlus size={13} strokeWidth={1.5} aria-hidden="true" />
                    </button>
                  </div>
                )}
                {filtered.map(item => (
                  <div key={item.id}>
                    {!collapsed && (
                      <div className="flex h-6 items-center gap-1.5 px-2 text-[11px] text-muted-foreground">
                        <Folder size={12} strokeWidth={1.6} aria-hidden="true" />
                        <span className="min-w-0 flex-1 truncate">{item.name}</span>
                        <span className="tabular-nums">{item.chats.length}</span>
                      </div>
                    )}
                    {item.chats.map(chat => (
                      <button
                        key={chat.id}
                        type="button"
                        onClick={() => openChat(item.id, chat.id)}
                        className={cn(
                          "flex w-full items-center text-left text-[13px] tracking-[-0.008em] transition-colors",
                          collapsed ? "h-9 justify-center" : "h-[26px] gap-1.5 rounded-md py-0 pr-2 pl-7",
                          activeChatId === chat.id && surface === "chat" ? "bg-accent font-medium text-foreground" : "text-foreground/80 hover:bg-accent/70",
                        )}
                      >
                        <StatusDot status={chat.status} />
                        {!collapsed && (
                          <>
                            <span className="min-w-0 flex-1 truncate">{chat.title}</span>
                            <GitBranch size={11} strokeWidth={1.7} className="shrink-0 text-ring" aria-hidden="true" />
                            <span className="shrink-0 text-[11px] tabular-nums text-muted-foreground/65">{chat.time}</span>
                          </>
                        )}
                      </button>
                    ))}
                  </div>
                ))}
              </div>

              <button
                type="button"
                onClick={() => setSurface("settings")}
                aria-label="Open settings for cestercian"
                className={cn(
                  "mt-1 flex shrink-0 items-center border-t border-sidebar-border pt-1.5 text-foreground/85 transition-colors hover:text-foreground",
                  collapsed ? "size-10 justify-center" : "h-12 w-full gap-2.5 px-2",
                )}
              >
                <span className="grid size-7 shrink-0 place-items-center rounded-full bg-foreground text-[11px] font-semibold text-background">C</span>
                {!collapsed && <>
                  <span className="min-w-0 flex-1 truncate text-left text-[13px] font-medium">cestercian</span>
                  <Settings2 size={16} strokeWidth={1.6} className="text-muted-foreground" aria-hidden="true" />
                </>}
              </button>
            </div>
          </aside>
        </div>
      </div>
    </div>
  );
}

function Placeholder({ title, body }: { title: string; body: string }) {
  return (
    <div className="flex flex-1 flex-col items-center justify-center px-8 text-center">
      <h1 className="font-display text-[1.6rem] font-medium tracking-[-0.03em] text-foreground">{title}</h1>
      <p className="mt-2 max-w-sm text-[13px] leading-relaxed text-muted-foreground">{body}</p>
    </div>
  );
}

function AutomationsCanvas() {
  const rows = [
    { name: "Nightly briefing", provider: "Claude Code", when: "in 6h", status: "active" },
    { name: "Weekly review", provider: "Codex", when: "paused", status: "paused" },
  ];
  return (
    <main className="mx-auto w-full max-w-3xl flex-1 overflow-y-auto px-6 pb-16 pt-10">
      <h1 className="font-display text-[28px] font-semibold tracking-[-0.03em] text-foreground">Automations</h1>
      <p className="mt-1.5 text-[13.5px] leading-relaxed text-muted-foreground">
        Scheduled jobs only. The catalog tabs are not part of this view.
      </p>
      <div className="mt-6 grid gap-3">
        {rows.map(row => (
          <article key={row.name} className="rounded-2xl border border-border bg-card px-4 py-3.5">
            <div className="flex items-start gap-3">
              <span className="mt-0.5 flex size-9 items-center justify-center rounded-xl border border-border bg-accent text-muted-foreground">
                <Bot size={15} aria-hidden="true" />
              </span>
              <div className="min-w-0 flex-1">
                <h2 className="text-[13.5px] font-semibold text-foreground">{row.name}</h2>
                <p className="mt-0.5 text-[12px] text-muted-foreground">{row.provider} · {row.when}</p>
              </div>
              <span className="rounded-full border border-border px-2 py-0.5 text-[10px] uppercase tracking-[0.06em] text-muted-foreground">{row.status}</span>
            </div>
          </article>
        ))}
      </div>
    </main>
  );
}

function ComposerCanvas({
  surface,
  repo,
  repos,
  host,
  hostMeta,
  HostIcon,
  worktree,
  draft,
  menu,
  activeChatId,
  onDraft,
  onMenu,
  onRepo,
  onBranch,
  onHost,
  onWorktree,
  onCloudShortcut,
}: {
  surface: Surface;
  repo: Repo;
  repos: Repo[];
  host: HostId;
  hostMeta: (typeof HOSTS)[number];
  HostIcon: LucideIcon;
  worktree: boolean;
  draft: string;
  menu: MenuId;
  activeChatId: string | null;
  onDraft: (value: string) => void;
  onMenu: (id: MenuId) => void;
  onRepo: (id: string) => void;
  onBranch: (branch: string) => void;
  onHost: (id: HostId) => void;
  onWorktree: () => void;
  onCloudShortcut: () => void;
}) {
  const chat = repo.chats.find(item => item.id === activeChatId);
  const hero = surface === "new-chat";
  return (
    <div className={cn("flex min-h-0 flex-1 flex-col", hero ? "items-center justify-center px-6 pb-16" : "px-6")}>
      {hero && (
        <h1 className="mb-8 max-w-xl text-center font-display text-[1.85rem] font-medium leading-[1.15] tracking-[-0.03em] text-foreground">
          What should we work on in {repo.name}?
        </h1>
      )}
      {!hero && (
        <div className="min-h-0 flex-1 overflow-y-auto py-8">
          <p className="text-[13px] text-muted-foreground">{chat?.title ?? "Chat"} in {repo.name}</p>
          <p className="mt-4 max-w-xl text-[15px] leading-relaxed text-foreground/90">
            New Chat skipped the project picker. This session opened in the repo you were already in. Switch repo, branch, worktree, or host from the strip under the input.
          </p>
        </div>
      )}

      <div className={cn("w-full", hero ? "max-w-2xl" : "mx-auto max-w-2xl pb-5")}>
        <div className="relative mb-2 flex flex-nowrap items-center justify-center gap-0.5 overflow-x-auto">
          <div className="relative">
            <Chip icon={FolderGit2} label={repo.name} expanded={menu === "repo"} onClick={() => onMenu(menu === "repo" ? null : "repo")} />
            {menu === "repo" && (
              <Menu title="Repository">
                {repos.map(item => (
                  <MenuItem key={item.id} active={item.id === repo.id} onClick={() => onRepo(item.id)}>
                    <FolderGit2 size={13} aria-hidden="true" />
                    {item.name}
                  </MenuItem>
                ))}
              </Menu>
            )}
          </div>
          <div className="relative">
            <Chip icon={GitBranch} label={repo.branch} expanded={menu === "branch"} onClick={() => onMenu(menu === "branch" ? null : "branch")} />
            {menu === "branch" && (
              <Menu title="Branch">
                {repo.branches.map(branch => (
                  <MenuItem key={branch} active={branch === repo.branch} onClick={() => onBranch(branch)}>
                    <GitBranch size={13} aria-hidden="true" />
                    {branch}
                  </MenuItem>
                ))}
              </Menu>
            )}
          </div>
          <Chip icon={GitFork} label={worktree ? "Isolated worktree" : "On branch"} pressed={worktree} onClick={onWorktree} />
          <div className="relative">
            <Chip icon={HostIcon} label={hostMeta.label} expanded={menu === "host"} onClick={() => onMenu(menu === "host" ? null : "host")} />
            {menu === "host" && (
              <Menu title="Agent host">
                {HOSTS.map(item => {
                  const Icon = item.icon;
                  return (
                    <MenuItem key={item.id} active={item.id === host} onClick={() => onHost(item.id)}>
                      <Icon size={13} aria-hidden="true" />
                      <span className="min-w-0 flex-1">
                        <span className="block">{item.label}</span>
                        <span className="block text-[11px] text-muted-foreground">{item.hint}</span>
                      </span>
                    </MenuItem>
                  );
                })}
              </Menu>
            )}
          </div>
        </div>

        <form
          className="relative flex flex-col gap-2 rounded-[1.4rem] border border-input bg-card px-4 py-3 focus-within:border-ring"
          onSubmit={event => event.preventDefault()}
        >
          <textarea
            value={draft}
            rows={hero ? 3 : 2}
            onChange={event => onDraft(event.target.value)}
            placeholder="Plan, build, / for skills, @ for context"
            aria-label="Message"
            className="min-h-12 w-full resize-none bg-transparent text-[15px] leading-relaxed tracking-[-0.006em] text-foreground outline-none placeholder:text-muted-foreground/70"
          />
          <div className="flex items-center gap-1.5">
            <button type="button" aria-label="Add context" className="inline-flex size-7 items-center justify-center rounded-full border border-border text-muted-foreground hover:bg-accent hover:text-foreground">
              <Plus size={14} strokeWidth={1.7} aria-hidden="true" />
            </button>
            <span className="inline-flex h-7 items-center rounded-full px-2.5 text-[12px] text-muted-foreground">High</span>
            <button type="button" aria-label="Voice input" className="ml-auto inline-flex size-8 items-center justify-center rounded-full bg-foreground text-background">
              <Mic size={14} strokeWidth={1.8} aria-hidden="true" />
            </button>
          </div>
        </form>

        {hero && (
          <div className="mt-3 flex flex-wrap items-center justify-center gap-2">
            <button type="button" className="inline-flex h-8 items-center rounded-full border border-border bg-card px-3 text-[12.5px] text-foreground hover:bg-accent">
              Plan new idea
            </button>
            <button type="button" className="inline-flex h-8 items-center rounded-full border border-border bg-card px-3 text-[12.5px] text-foreground hover:bg-accent">
              Multitask
            </button>
            <button type="button" onClick={onCloudShortcut} className="inline-flex h-8 items-center gap-1.5 rounded-full border border-border bg-card px-3 text-[12.5px] text-foreground hover:bg-accent">
              <Cloud size={13} aria-hidden="true" />
              Run in Cloud
            </button>
          </div>
        )}
      </div>
    </div>
  );
}
