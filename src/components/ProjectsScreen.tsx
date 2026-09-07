import { SCREEN_CONTENT, ScreenHeading } from "./ui/screen";
import { FolderGit2, FolderOpen, GitBranch, Plus, Sparkles } from "lucide-react";
import type { Session, SessionStatus, Workspace } from "../types";
import { cn } from "@/lib/utils";
import { chatName, chatTimestamp } from "./sidebarChats";

// Projects are their own entity, not a second kind of row in the chat rail. This
// screen is where a repo's identity lives: where it is on disk, what branch it is
// on, what is uncommitted, and which agents are working in it.

const CHATS_PER_CARD = 6;

/** The leaf directory identifies a repo; its ancestors rarely do. Shortening in
 * JS rather than clipping with `direction: rtl`, which reorders a leading slash
 * onto the end and renders `/Users/x/harness` as `Users/x/harness/`. */
export function shortPath(path: string): string {
  const parts = path.split("/").filter(Boolean);
  return parts.length > 3 ? `…/${parts.slice(-3).join("/")}` : path;
}

function StatusDot({ status }: { status: SessionStatus }) {
  const color = status === "working" ? "bg-success"
    : status === "waiting" ? "bg-warning"
    : status === "failed" ? "bg-destructive"
    : status === "ready" ? "bg-info"
    : "bg-muted-foreground/25";
  return <span className={cn("h-1.5 w-1.5 shrink-0 rounded-full", color)} />;
}

export type ProjectsScreenProps = {
  workspaces: Workspace[];
  /** Every top-level chat; the screen selects each project's own. */
  chats: Session[];
  activeSessionId?: string;
  busy: boolean;
  onOpenSession: (id: string) => void;
  onNewWorkspace: () => void;
  onNewWorkspaceSession: (workspaceId: string) => void;
  onConnectFolder: (workspaceId: string) => void;
};

export function ProjectsScreen({
  workspaces,
  chats,
  activeSessionId,
  busy,
  onOpenSession,
  onNewWorkspace,
  onNewWorkspaceSession,
  onConnectFolder,
}: ProjectsScreenProps) {
  return (
    <div className="h-full overflow-y-auto">
      <div className={SCREEN_CONTENT}>
        <ScreenHeading title="Projects" description="Your repositories, conversations, and changes." action={
          <button
            type="button"
            onClick={onNewWorkspace}
            className="inline-flex h-8 shrink-0 items-center gap-1.5 rounded-lg bg-primary px-3 text-[13px] font-medium text-primary-foreground transition-opacity hover:opacity-90 active:scale-[0.98]"
          >
            <Plus size={14} strokeWidth={1.9} aria-hidden="true" /> New project
          </button>
        } />

        {!workspaces.length ? (
          <div className="rounded-xl border border-border bg-card px-6 py-10 text-center">
            <FolderGit2 size={20} strokeWidth={1.5} className="mx-auto mb-3 text-muted-foreground" aria-hidden="true" />
            <p className="m-0 text-[13px] font-medium text-foreground">No projects yet</p>
            <p className="mx-auto mt-1 max-w-sm text-[12.5px] leading-relaxed text-muted-foreground">
              A project connects a repository so agents can read it, branch from it, and open pull requests against it.
            </p>
          </div>
        ) : (
          <div className="divide-y divide-border overflow-hidden rounded-xl border border-border bg-card">
            {workspaces.map(workspace => {
              const own = chats
                .filter(chat => chat.workspaceId === workspace.id)
                .sort((a, b) => (chatTimestamp(b) ?? 0) - (chatTimestamp(a) ?? 0));
              const shown = own.slice(0, CHATS_PER_CARD);
              const dirty = workspace.dirtyFiles > 0;
              return (
                <section key={workspace.id} className="px-4 py-4">
                  <div className="flex min-w-0 items-center gap-2">
                    <span className="grid size-9 shrink-0 place-items-center rounded-lg bg-accent text-muted-foreground"><FolderGit2 size={19} strokeWidth={1.6} aria-hidden="true" /></span>
                    <h2 className="m-0 min-w-0 flex-1 truncate text-[15px] font-semibold tracking-[-0.01em] text-foreground">{workspace.title}</h2>
                  <div className="ml-auto flex shrink-0 flex-wrap items-center gap-1.5">
                    <button
                      type="button"
                      disabled={busy}
                      onClick={() => onNewWorkspaceSession(workspace.id)}
                      className="inline-flex h-8 items-center gap-1.5 rounded-lg px-2 text-[12px] font-medium text-muted-foreground transition-colors hover:bg-accent hover:text-foreground disabled:opacity-40"
                    >
                      <Sparkles size={11} strokeWidth={1.75} aria-hidden="true" /> New agent
                    </button>
                    {!workspace.path && (
                      <button
                        type="button"
                        onClick={() => onConnectFolder(workspace.id)}
                        className="inline-flex h-8 items-center gap-1.5 rounded-lg px-2 text-[12px] font-medium text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
                      >
                        <FolderOpen size={11} strokeWidth={1.75} aria-hidden="true" /> Connect folder
                      </button>
                    )}
                  </div>
                  </div>

                  <p
                    className="mt-1 pl-11 truncate text-caption text-muted-foreground"
                    title={workspace.path ?? undefined}
                  >
                    {workspace.path ? shortPath(workspace.path) : "No folder connected"}
                  </p>

                  <p className="mt-1 flex flex-wrap items-center gap-1.5 pl-11 text-caption text-muted-foreground">
                    <GitBranch size={11} strokeWidth={1.6} aria-hidden="true" />
                    {workspace.branch ?? "folder"}
                    <span aria-hidden="true">·</span>
                    <span>{own.length} {own.length === 1 ? "chat" : "chats"}</span>
                    <span aria-hidden="true">·</span>
                    {dirty ? (
                      <>
                        <span className="text-warning">{workspace.dirtyFiles} changed</span>
                        <span className="text-success">+{workspace.additions}</span>
                        <span className="text-destructive">−{workspace.deletions}</span>
                      </>
                    ) : (
                      <span>clean</span>
                    )}
                  </p>

                  <div className="mt-3 min-h-0 pl-10">
                    {shown.map(chat => (
                      <button
                        key={chat.id}
                        type="button"
                        onClick={() => onOpenSession(chat.id)}
                        title={chatName(chat)}
                        className={cn(
                          "flex h-9 w-full items-center gap-2 rounded-md px-1.5 text-left text-[13px] transition-colors",
                          chat.id === activeSessionId
                            ? "bg-accent font-medium text-foreground"
                            : "text-muted-foreground hover:bg-accent hover:text-foreground",
                        )}
                      >
                        <StatusDot status={chat.status} />
                        <span className="min-w-0 flex-1 truncate">{chatName(chat)}</span>
                        <span className="shrink-0 text-caption text-muted-foreground">{chat.status === "working" ? "Working" : chat.status === "waiting" ? "Needs you" : chat.status === "failed" ? "Failed" : ""}</span>
                      </button>
                    ))}
                    {own.length > CHATS_PER_CARD && (
                      <p className="px-1.5 pt-1 text-caption text-muted-foreground">+{own.length - CHATS_PER_CARD} more</p>
                    )}
                    {!own.length && (
                      <p className="px-1.5 text-caption text-muted-foreground">No agents here yet.</p>
                    )}
                  </div>


                </section>
              );
            })}
          </div>
        )}
      </div>
    </div>
  );
}
