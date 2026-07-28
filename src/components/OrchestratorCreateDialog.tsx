import { GitBranch, GitFork, X } from "lucide-react";
import { useEffect, useRef } from "react";

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
    <div
      className="fixed inset-0 z-50 flex items-start justify-center bg-black/45 p-4 pt-[10vh] backdrop-blur-md"
      role="dialog"
      aria-modal="true"
      aria-labelledby="orchestrator-create-title"
    >
      <div className="u-glass-popover animate-page-enter w-full max-w-md overflow-hidden rounded-3xl">
        <div className="relative border-b border-white/[0.06] px-5 py-4">
          <div className="absolute inset-0 bg-gradient-to-r from-white/[0.03] via-transparent to-white/[0.015]" />
          <div className="relative flex items-center justify-between gap-3">
            <div className="flex min-w-0 items-center gap-3">
              <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-[12px] border border-white/[0.08] bg-white/[0.05]">
                <GitFork className="h-4 w-4 text-neutral-300" strokeWidth={1.5} aria-hidden="true" />
              </span>
              <div className="min-w-0">
                <h2 id="orchestrator-create-title" className="font-display text-base font-semibold text-white">Create an isolated worktree?</h2>
                <p className="mt-0.5 truncate text-[13px] text-neutral-500">New orchestrator in {workspaceTitle}</p>
              </div>
            </div>
            <button type="button" disabled={busy} onClick={onClose} className="shrink-0 rounded-xl p-2 text-neutral-500 transition-colors hover:bg-white/[0.08] hover:text-neutral-200 disabled:opacity-40" aria-label="Continue without a worktree">
              <X className="h-4 w-4" strokeWidth={2} aria-hidden="true" />
            </button>
          </div>
        </div>

        <div className="p-5">
          <p className="text-[13px] leading-5 text-neutral-400">
            Bridge can create a new Git branch and worktree automatically. This orchestrator gets its own files and cannot overwrite another chat’s uncommitted work.
          </p>
          {!canCreateWorktree && <p role="status" className="mt-3 rounded-xl border border-amber-300/10 bg-amber-300/[0.04] px-3 py-2.5 text-[11.5px] leading-4 text-amber-100/65">Connect a Git repository to this workspace to enable isolated worktrees.</p>}

          <div className="mt-5 flex flex-col gap-2">
            <button ref={primaryRef} type="button" disabled={busy || !canCreateWorktree} onClick={onCreateWorktree} className="flex items-center justify-center gap-2 rounded-xl bg-white px-4 py-2.5 text-sm font-medium text-neutral-900 transition-all hover:brightness-105 active:scale-[0.98] disabled:opacity-30">
              <GitFork className="h-4 w-4" aria-hidden="true" />
              {busy ? "Creating…" : "Create worktree"}
            </button>
            <button ref={sharedRef} type="button" disabled={busy} onClick={onUseCurrentFolder} className="flex items-center justify-center gap-2 rounded-xl border border-white/[0.08] bg-white/[0.03] px-4 py-2.5 text-sm text-neutral-400 transition-colors hover:bg-white/[0.06] hover:text-neutral-200 disabled:opacity-30">
              <GitBranch className="h-4 w-4" aria-hidden="true" />
              Use current folder
            </button>
          </div>
          <p className="mt-3 text-center text-[10.5px] leading-4 text-neutral-600">The worktree starts from the repository’s current HEAD. Uncommitted changes are not copied.</p>
        </div>
      </div>
    </div>
  );
}
