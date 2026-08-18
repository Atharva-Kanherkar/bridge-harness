import { GitBranch, GitFork } from "lucide-react";
import { useEffect, useRef } from "react";
import { CreateDialogShell } from "./CreateDialogShell";

export type OrchestratorCreateDialogProps = {
  open: boolean;
  workspaceTitle: string;
  canCreateWorktree: boolean;
  busy?: boolean;
  onCreateWorktree: () => void;
  onUseCurrentFolder: () => void;
  onClose: () => void;
};

export function OrchestratorCreateDialog({
  open,
  workspaceTitle,
  canCreateWorktree,
  busy,
  onCreateWorktree,
  onUseCurrentFolder,
  onClose,
}: OrchestratorCreateDialogProps) {
  const primaryRef = useRef<HTMLButtonElement>(null);
  const sharedRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (!open) return;
    (canCreateWorktree ? primaryRef : sharedRef).current?.focus();
  }, [canCreateWorktree, open]);

  useEffect(() => {
    if (!open || busy) return;
    const handleKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", handleKey);
    return () => window.removeEventListener("keydown", handleKey);
  }, [busy, onClose, open]);

  if (!open) return null;

  return (
    <CreateDialogShell
      titleId="orchestrator-create-title"
      icon={<GitFork className="h-4 w-4 text-muted-foreground" strokeWidth={1.5} aria-hidden="true" />}
      title="Create an isolated worktree?"
      subtitle={`New orchestrator in ${workspaceTitle}`}
      subtitleClassName="truncate"
      closeLabel="Continue without a worktree"
      closeDisabled={busy}
      onClose={onClose}
    >
      <p className="text-[13px] leading-5 text-muted-foreground">
        Bridge can create a new Git branch and worktree automatically. This orchestrator gets its own files and cannot overwrite another chat’s uncommitted work.
      </p>
      {!canCreateWorktree && <p role="status" className="mt-3 rounded-xl border border-warning/30 bg-warning/10 px-3 py-2.5 text-[11.5px] leading-4 text-warning">Connect a Git repository to this workspace to enable isolated worktrees.</p>}

      <div className="mt-5 flex flex-col gap-2">
        <button ref={primaryRef} type="button" disabled={busy || !canCreateWorktree} onClick={onCreateWorktree} className="flex items-center justify-center gap-2 rounded-xl bg-primary px-4 py-2.5 text-sm font-medium text-primary-foreground transition-all hover:bg-primary/90 active:scale-[0.98] disabled:opacity-30">
          <GitFork className="h-4 w-4" aria-hidden="true" />
          {busy ? "Creating…" : "Create worktree"}
        </button>
        <button ref={sharedRef} type="button" disabled={busy} onClick={onUseCurrentFolder} className="flex items-center justify-center gap-2 rounded-xl border border-border bg-card px-4 py-2.5 text-sm text-muted-foreground transition-colors hover:bg-accent hover:text-foreground disabled:opacity-30">
          <GitBranch className="h-4 w-4" aria-hidden="true" />
          Use current folder
        </button>
      </div>
      <p className="mt-3 text-center text-[10.5px] leading-4 text-muted-foreground/70">The worktree starts from the repository’s current HEAD. Uncommitted changes are not copied.</p>
    </CreateDialogShell>
  );
}
