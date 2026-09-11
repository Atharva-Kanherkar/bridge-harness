import { ChartNoAxesColumn, ChevronDown, ChevronRight, FolderGit2, LayoutGrid, Pin, Search, Settings2, SquarePen, Store, TerminalSquare } from "lucide-react";
import { directChats, harnessDot, repos, type SidebarSession, type Tone } from "../../content/appScenes";

const dot: Record<Tone, string> = {
  success: "bg-success",
  warning: "bg-warning",
  info: "bg-info",
  destructive: "bg-destructive",
  faint: "bg-faint",
};

function NavRow({ icon: Icon, label, active }: { icon: typeof Store; label: string; active?: boolean }) {
  return (
    <span
      className={`flex h-8 w-full items-center gap-2.5 rounded-lg px-2 text-[13px] tracking-[-0.008em] ${
        active ? "bg-accent font-medium text-foreground" : "text-foreground/85"
      }`}
    >
      <Icon size={15} className={`shrink-0 ${active ? "text-foreground" : "text-muted-foreground"}`} aria-hidden="true" />
      {label}
    </span>
  );
}

function Row({ session }: { session: SidebarSession }) {
  return (
    <span className={`relative flex items-center rounded-[7px] ${session.selected ? "bg-selection text-selection-foreground" : ""}`}>
      <span className={`flex h-11 min-w-0 flex-1 items-center gap-2 rounded-[7px] pr-2 text-left ${session.worker ? "pl-9" : "pl-7"}`}>
        <span className="flex min-w-0 flex-1 flex-col justify-center gap-0.5">
          <span className="truncate text-[13px] font-medium leading-4 tracking-[-0.008em] text-foreground">{session.title}</span>
          <span className="flex min-w-0 items-center gap-1.5 truncate text-[11px] leading-[0.875rem] tracking-[-0.004em] text-muted-foreground">
            <span className={`h-1.5 w-1.5 shrink-0 rounded-full ${dot[session.tone]}`} aria-hidden="true" />
            <span className="truncate">{session.status}</span>
            <span aria-hidden="true">·</span>
            <span className="shrink-0 tabular-nums text-faint">{session.time}</span>
          </span>
        </span>
        <span className={`size-2 shrink-0 rounded-full ${harnessDot[session.harness]}`} aria-hidden="true" />
      </span>
    </span>
  );
}

/** The real rail: 248px, the same nav order, and the repository tree filled the way a
 * working week actually looks — several checkouts, workers under their orchestrators,
 * four harnesses at once. */
export default function Sidebar({ activeNav }: { activeNav: "chats" | "mission" | "usage" }) {
  return (
    <aside className="flex min-h-0 flex-col border-r border-sidebar-border bg-sidebar max-md:hidden">
      <div className="flex h-11 shrink-0 items-center gap-1.5 pl-3.5 pr-1.5">
        <span className="size-3 rounded-full bg-[#ff5f57]" aria-hidden="true" />
        <span className="size-3 rounded-full bg-[#febc2e]" aria-hidden="true" />
        <span className="size-3 rounded-full bg-[#28c840]" aria-hidden="true" />
      </div>

      <div className="flex min-h-0 flex-1 flex-col overflow-hidden px-3 pb-3 pt-1">
        <div className="mb-3 flex shrink-0 items-center gap-1.5">
          <span className="inline-flex h-8 min-w-0 flex-1 items-center gap-2 rounded-[7px] border border-border-card bg-card px-2.5 text-[13px] font-medium text-foreground">
            <SquarePen size={14} className="shrink-0" aria-hidden="true" />
            <span className="min-w-0 flex-1 truncate">New Chat</span>
            <span className="text-[11px] font-normal text-muted-foreground">⌘N</span>
          </span>
          <span className="inline-flex size-8 shrink-0 items-center justify-center rounded-[7px] border border-border text-muted-foreground">
            <Search size={14} aria-hidden="true" />
          </span>
        </div>

        <nav className="mb-4 shrink-0 space-y-0.5">
          <NavRow icon={Store} label="Marketplace" />
          <NavRow icon={FolderGit2} label="Projects" />
          <NavRow icon={TerminalSquare} label="Agent Fleet" />
          <NavRow icon={LayoutGrid} label="Mission Control" active={activeNav === "mission"} />
          <NavRow icon={Pin} label="Memory" />
          <NavRow icon={ChartNoAxesColumn} label="Usage" active={activeNav === "usage"} />
        </nav>

        <div className="min-h-0 flex-1 overflow-hidden">
          <div className="flex h-7 items-center gap-1.5 px-2">
            <span className="text-[12px] font-medium tracking-[-0.004em] text-muted-foreground">Repositories</span>
          </div>
          {repos.map(repo => (
            <div key={repo.name} className="mb-1.5 flex flex-col gap-0.5">
              <div className="flex h-7 w-full items-center gap-1.5 rounded-md pl-2 pr-1 text-[13px] tracking-[-0.008em]">
                <span className="shrink-0 text-faint" aria-hidden="true">
                  {repo.open ? <ChevronDown size={12} /> : <ChevronRight size={12} />}
                </span>
                <span className="min-w-0 truncate text-[12px] text-muted-foreground">{repo.name}</span>
                <span className="ml-auto shrink-0 text-[11px] tabular-nums text-muted-foreground/60">{repo.sessions.length}</span>
              </div>
              {repo.open && repo.sessions.map(session => <Row key={session.title} session={session} />)}
            </div>
          ))}

          <div className="mt-3 flex h-7 items-center gap-1.5 px-2">
            <span className="text-[12px] font-medium tracking-[-0.004em] text-muted-foreground">Chats</span>
          </div>
          {directChats.map(chat => <Row key={chat.title} session={chat} />)}
        </div>

        <div className="mt-1 flex shrink-0 items-center gap-0.5 border-t border-sidebar-border pt-1.5">
          <span className="flex h-8 min-w-0 flex-1 items-center gap-2 rounded-lg px-2 text-[12px] text-muted-foreground">
            <Settings2 size={14} aria-hidden="true" />
            Settings
          </span>
        </div>
      </div>
    </aside>
  );
}
