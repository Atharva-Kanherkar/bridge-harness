import { useEffect, useMemo, useState } from "react";
import { Check, FolderGit2, GitFork, MessagesSquare, MessageSquarePlus } from "lucide-react";
import type { Workspace } from "../types";
import { cn } from "@/lib/utils";
import { CreateDialogShell } from "./CreateDialogShell";

// Every chat starts with the same two questions: does it belong to a project, and
// if so does it get its own worktree. Asking once, here, is what lets the rail
// keep plain chats and project work apart.

export type NewChatChoice = { workspaceId: string | null; worktree: boolean };

export type NewChatDialogProps = {
  open: boolean;
  workspaces: Workspace[];
  /** Preselects a project, e.g. when started from that project's card. */
  initialWorkspaceId?: string | null;
  busy?: boolean;
  onClose: () => void;
  onStart: (choice: NewChatChoice) => void;
};

/** A worktree needs a repository behind the workspace, which is what `projectId`
 * records. Mirrors the check the orchestrator dialog already makes. */
function canWorktree(workspace: Workspace | undefined): boolean {
  return !!workspace?.projectId;
}

export function NewChatDialog({
  open,
  workspaces,
  initialWorkspaceId = null,
  busy,
  onClose,
  onStart,
}: NewChatDialogProps) {
  const [workspaceId, setWorkspaceId] = useState<string | null>(initialWorkspaceId);
  const [worktree, setWorktree] = useState(false);

  // Each visit starts from the caller's intent, not from the last visit's answer.
  useEffect(() => {
    if (!open) return;
    setWorkspaceId(initialWorkspaceId);
    setWorktree(false);
  }, [open, initialWorkspaceId]);

  useEffect(() => {
    if (!open || busy) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [busy, onClose, open]);

  const selected = useMemo(() => workspaces.find(item => item.id === workspaceId), [workspaces, workspaceId]);
  const worktreeAvailable = canWorktree(selected);

  if (!open) return null;

  return (
    <CreateDialogShell
      titleId="new-chat-title"
      icon={<MessageSquarePlus className="h-4 w-4 text-muted-foreground" strokeWidth={1.5} aria-hidden="true" />}
      title="Start a chat"
      subtitle="Where should it run?"
      closeLabel="Cancel"
      closeDisabled={busy}
      onClose={onClose}
    >
      <div role="radiogroup" aria-label="Project" className="flex max-h-64 flex-col gap-1 overflow-y-auto">
        <button
          type="button"
          role="radio"
          aria-checked={workspaceId === null}
          onClick={() => setWorkspaceId(null)}
          className={cn(
            "flex items-center gap-2.5 rounded-lg border px-3 py-2 text-left transition-colors",
            workspaceId === null ? "border-border bg-accent" : "border-transparent hover:bg-accent",
          )}
        >
          <MessagesSquare size={15} strokeWidth={1.6} className="shrink-0 text-muted-foreground" aria-hidden="true" />
          <span className="min-w-0 flex-1">
            <span className="block text-[13px] font-medium text-foreground">No project</span>
            <span className="block text-[11px] text-muted-foreground">A plain chat, no repository attached.</span>
          </span>
          {workspaceId === null && <Check size={14} strokeWidth={2.2} className="shrink-0 text-foreground" aria-hidden="true" />}
        </button>

        {workspaces.map(workspace => {
          const active = workspace.id === workspaceId;
          return (
            <button
              key={workspace.id}
              type="button"
              role="radio"
              aria-checked={active}
              onClick={() => setWorkspaceId(workspace.id)}
              className={cn(
                "flex items-center gap-2.5 rounded-lg border px-3 py-2 text-left transition-colors",
                active ? "border-border bg-accent" : "border-transparent hover:bg-accent",
              )}
            >
              <FolderGit2 size={15} strokeWidth={1.6} className="shrink-0 text-muted-foreground" aria-hidden="true" />
              <span className="min-w-0 flex-1">
                <span className="block truncate text-[13px] font-medium text-foreground">{workspace.title}</span>
                <span className="block truncate font-mono text-[10px] text-muted-foreground">
                  {workspace.branch ?? "folder"}
                  {workspace.dirtyFiles ? ` · ${workspace.dirtyFiles} changed` : " · clean"}
                </span>
              </span>
              {active && <Check size={14} strokeWidth={2.2} className="shrink-0 text-foreground" aria-hidden="true" />}
            </button>
          );
        })}
      </div>

      {workspaceId !== null && (
        <label
          className={cn(
            "mt-3 flex items-start gap-2.5 rounded-lg border border-border px-3 py-2.5",
            worktreeAvailable ? "cursor-pointer hover:bg-accent" : "opacity-60",
          )}
        >
          <input
            type="checkbox"
            checked={worktree && worktreeAvailable}
            disabled={!worktreeAvailable || busy}
            onChange={event => setWorktree(event.target.checked)}
            className="mt-0.5 size-3.5 shrink-0 accent-primary"
          />
          <span className="min-w-0 flex-1">
            <span className="flex items-center gap-1.5 text-[13px] font-medium text-foreground">
              <GitFork size={13} strokeWidth={1.6} aria-hidden="true" /> Isolated worktree
            </span>
            <span className="mt-0.5 block text-[11px] leading-4 text-muted-foreground">
              {worktreeAvailable
                ? "Its own branch and folder, so it cannot overwrite another chat’s uncommitted work. Starts from the repository’s current HEAD."
                : "Connect a Git repository to this project to enable isolated worktrees."}
            </span>
          </span>
        </label>
      )}

      <button
        type="button"
        disabled={busy}
        onClick={() => onStart({ workspaceId, worktree: worktree && worktreeAvailable })}
        className="mt-4 flex w-full items-center justify-center gap-2 rounded-xl bg-primary px-4 py-2.5 text-sm font-medium text-primary-foreground transition-all hover:bg-primary/90 active:scale-[0.98] disabled:opacity-30"
      >
        {busy ? "Starting…" : selected ? `Start in ${selected.title}` : "Start chat"}
      </button>
    </CreateDialogShell>
  );
}
