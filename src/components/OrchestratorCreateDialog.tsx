import { GitBranch, GitFork } from "lucide-react";
import { useEffect, useRef } from "react";
import { CreateDialogShell } from "./CreateDialogShell";
import { Button } from "@/components/ui/button";

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

  return (
    <CreateDialogShell
      open={open}
      titleId="orchestrator-create-title"
      icon={<GitFork className="h-4 w-4 text-muted-foreground" strokeWidth={1.5} aria-hidden="true" />}
      title="Create an isolated worktree?"
      subtitle={`New orchestrator in ${workspaceTitle}`}
      subtitleClassName="truncate"
      closeLabel="Cancel new orchestrator"
      closeDisabled={busy}
      onClose={onClose}
    >
      <p className="text-ui leading-5 text-muted-foreground">
        Bridge can create a new Git branch and worktree automatically. This orchestrator gets its own files and cannot overwrite another chat’s uncommitted work.
      </p>
      {!canCreateWorktree && <p role="status" className="mt-3 rounded-xl border border-warning/30 bg-warning/10 px-3 py-2.5 text-ui leading-4 text-warning">Connect a Git repository to this workspace to enable isolated worktrees.</p>}

      <div className="mt-5 flex flex-wrap justify-end gap-2 border-t border-border pt-4">
        <Button variant="quiet" size="sm" className="mr-auto" disabled={busy} onClick={onClose}>Cancel</Button>
        <Button variant="outline" size="sm" ref={sharedRef} disabled={busy} onClick={onUseCurrentFolder}>
          <GitBranch className="h-4 w-4" aria-hidden="true" />
          Use current folder
        </Button>
        <Button variant="default" size="sm" ref={primaryRef} disabled={busy || !canCreateWorktree} loading={busy} onClick={onCreateWorktree}>
          <GitFork className="h-4 w-4" aria-hidden="true" />
          Create worktree
        </Button>
      </div>
      <p className="mt-3 text-meta leading-4 text-muted-foreground">The worktree starts from the repository’s current HEAD. Uncommitted changes are not copied.</p>
    </CreateDialogShell>
  );
}
