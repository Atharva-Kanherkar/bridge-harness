import { SCREEN_CONTENT, ScreenHeading } from "./ui/screen";
import { FolderGit2, FolderOpen, GitBranch, Plus, Sparkles } from "lucide-react";
import type { Session, SessionStatus, Workspace } from "../types";
import { cn } from "@/lib/utils";
import { chatName, chatTimestamp } from "./sidebarChats";
import { Tip } from "@/components/ui/tooltip";
import { Button } from "@/components/ui/button";
import { RowButton } from "@/components/ui/row";

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
    <div className="@container/projects h-full overflow-y-auto">
      <div className={SCREEN_CONTENT}>
        <ScreenHeading title="Projects" action={
          <Button variant="default" size="sm" className="shrink-0"
            onClick={onNewWorkspace}
          >
            <Plus size={14} strokeWidth={1.9} aria-hidden="true" /> New project
          </Button>
        } />

        {!workspaces.length ? (
          <div className="rounded-xl border border-border bg-card px-6 py-10 text-center">
            <FolderGit2 size={20} strokeWidth={1.5} className="mx-auto mb-3 text-muted-foreground" aria-hidden="true" />
            <p className="m-0 text-ui font-medium text-foreground">No projects yet</p>
            <p className="mx-auto mt-1 max-w-sm text-meta leading-relaxed text-muted-foreground">
              A project connects a repository so agents can read it, branch from it, and open pull requests against it.
            </p>
          </div>
        ) : (
          <div className="grid grid-cols-1 gap-4 @min-[640px]/projects:grid-cols-2 @min-[960px]/projects:grid-cols-3">
            {workspaces.map(workspace => {
              const own = chats
                .filter(chat => chat.workspaceId === workspace.id)
                .sort((a, b) => (chatTimestamp(b) ?? 0) - (chatTimestamp(a) ?? 0));
              const shown = own.slice(0, CHATS_PER_CARD);
              const dirty = workspace.dirtyFiles > 0;
              return (
                <section key={workspace.id} className="flex min-w-0 flex-col rounded-xl border border-border bg-card p-4">
                  <div className="flex min-w-0 items-center gap-2">
                    <span className="grid size-9 shrink-0 place-items-center rounded-lg bg-accent text-muted-foreground"><FolderGit2 size={19} strokeWidth={1.6} aria-hidden="true" /></span>
                    <Tip label={workspace.title}><h2 className="m-0 min-w-0 flex-1 truncate text-message font-semibold tracking-snug text-foreground">{workspace.title}</h2></Tip>
                  </div>

                  <Tip label={workspace.path ?? undefined}><p
                    className="mt-3 truncate text-caption text-muted-foreground"
                  >
                    {workspace.path ? shortPath(workspace.path) : "No folder connected"}
                  </p></Tip>

                  <p className="mt-1 flex min-w-0 items-center gap-1.5 text-caption text-muted-foreground">
                    <GitBranch size={11} strokeWidth={1.6} className="shrink-0" aria-hidden="true" />
                    <Tip label={workspace.branch ?? undefined}><span className="truncate">{workspace.branch ?? "folder"}</span></Tip>
                  </p>
                  <p className="mt-2 flex flex-wrap items-center gap-1.5 text-caption text-muted-foreground">
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

                  <div className="mb-4 mt-3 min-h-0 flex-1 border-t border-border pt-2">
                    {shown.map(chat => (
                      <Tip key={chat.id} label={chatName(chat)}><RowButton
                        tone="muted"
                        active={chat.id === activeSessionId}
                        onClick={() => onOpenSession(chat.id)}
                        className="h-9"
                      >
                        <StatusDot status={chat.status} />
                        <span className="min-w-0 flex-1 truncate">{chatName(chat)}</span>
                        <span className="shrink-0 text-caption text-muted-foreground">{chat.status === "working" ? "Working" : chat.status === "waiting" ? "Needs you" : chat.status === "failed" ? "Failed" : ""}</span>
                      </RowButton></Tip>
                    ))}
                    {own.length > CHATS_PER_CARD && (
                      <p className="px-1.5 pt-1 text-caption text-muted-foreground">+{own.length - CHATS_PER_CARD} more</p>
                    )}
                    {!own.length && (
                      <p className="px-1.5 text-caption text-muted-foreground">No agents here yet.</p>
                    )}
                  </div>
                  <div className="mt-auto flex flex-wrap items-center gap-1.5 border-t border-border pt-3">
                    <Button variant="quiet" size="sm"
                      disabled={busy}
                      onClick={() => onNewWorkspaceSession(workspace.id)}
                    >
                      <Sparkles size={13} strokeWidth={1.75} aria-hidden="true" /> New agent
                    </Button>
                    {!workspace.path && (
                      <Button variant="quiet" size="sm"
                        onClick={() => onConnectFolder(workspace.id)}
                      >
                        <FolderOpen size={13} strokeWidth={1.75} aria-hidden="true" /> Connect folder
                      </Button>
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
