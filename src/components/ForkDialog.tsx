import { GitFork, GitBranch, ShieldAlert } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { CreateDialogShell } from "./CreateDialogShell";

export type ForkWorktreeChoice = "shared" | "new";

export type ForkDialogProps = {
  open: boolean;
  /** The session being forked (used for the subtitle). */
  sessionLabel: string;
  sessionId: string;
  entryId: string;
  busy?: boolean;
  error?: string | null;
  onFork: (title: string | null, worktree: ForkWorktreeChoice) => void;
  onClose: () => void;
};

/**
 * The Fork dialog: pick a title and a worktree policy, then create an
 * independent session whose forest begins with the parent's history up to
 * the fork point. The parent is never modified — that is the one-line
 * reassurance the dialog opens with.
 */
export function ForkDialog({
  open,
  sessionLabel,
  sessionId,
  entryId,
  busy,
  error,
  onFork,
  onClose,
}: ForkDialogProps) {
  const [title, setTitle] = useState("");
  const [worktree, setWorktree] = useState<ForkWorktreeChoice>("shared");
  const submitRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (!open) return;
    setTitle("");
    setWorktree("shared");
    submitRef.current?.focus();
  }, [open]);

  useEffect(() => {
    if (!open || busy) return;
    const handleKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", handleKey);
    return () => window.removeEventListener("keydown", handleKey);
  }, [busy, onClose, open]);

  const submit = () => {
    if (busy) return;
    onFork(title.trim() || null, worktree);
  };

  return (
    <CreateDialogShell
      open={open}
      titleId="fork-session-title"
      icon={<GitFork className="h-4 w-4 text-muted-foreground" strokeWidth={1.5} aria-hidden="true" />}
      title="Fork here"
      subtitle={`New branch of ${sessionLabel} from this message`}
      subtitleClassName="truncate"
      closeLabel="Cancel fork"
      closeDisabled={busy}
      onClose={onClose}
    >
      <p className="text-[13px] leading-5 text-muted-foreground">
        The parent chat stays exactly as it is. The fork starts with this
        conversation's history up to this message and continues on its own.
      </p>

      <label className="mt-4 block">
        <span className="text-[12px] font-medium text-foreground">Fork title</span>
        <input
          value={title}
          onChange={event => setTitle(event.target.value)}
          placeholder={`Fork of ${sessionLabel}`}
          aria-label="Fork title"
          className="mt-1.5 w-full rounded-lg border border-border bg-background px-3 py-2 text-[13px] text-foreground outline-none transition-colors placeholder:text-muted-foreground/60 focus:border-input-ring focus:ring-1 focus:ring-input-ring"
        />
      </label>

      <fieldset className="mt-4">
        <legend className="text-[12px] font-medium text-foreground">Where the fork works</legend>
        <div className="mt-1.5 grid grid-cols-2 gap-2">
          <label className="flex cursor-pointer items-center gap-2 rounded-lg border border-border bg-card px-3 py-2.5 text-[13px] text-foreground transition-colors has-[[data-state=checked]]:border-foreground/60 has-[[data-state=checked]]:bg-accent">
            <input type="radio" name="fork-worktree" data-state={worktree === "shared" ? "checked" : "unchecked"} checked={worktree === "shared"} onChange={() => setWorktree("shared")} className="sr-only" />
            <GitBranch className="h-4 w-4 shrink-0 text-muted-foreground" aria-hidden="true" />
            <span className="min-w-0">
              Share the workspace
              <span className="block text-[11px] leading-4 text-muted-foreground">same files as the parent</span>
            </span>
          </label>
          <label className="flex cursor-pointer items-center gap-2 rounded-lg border border-border bg-card px-3 py-2.5 text-[13px] text-foreground transition-colors has-[[data-state=checked]]:border-foreground/60 has-[[data-state=checked]]:bg-accent">
            <input type="radio" name="fork-worktree" data-state={worktree === "new" ? "checked" : "unchecked"} checked={worktree === "new"} onChange={() => setWorktree("new")} className="sr-only" />
            <GitFork className="h-4 w-4 shrink-0 text-muted-foreground" aria-hidden="true" />
            <span className="min-w-0">
              New worktree and branch
              <span className="block text-[11px] leading-4 text-muted-foreground">isolated copy of the repo</span>
            </span>
          </label>
        </div>
        {worktree === "shared" ? (
          <p className="mt-2 flex items-start gap-1.5 text-[11px] leading-4 text-muted-foreground">
            <ShieldAlert className="mt-0.5 h-3.5 w-3.5 shrink-0" aria-hidden="true" />
            Two sessions writing the same files — a real hazard when both are live.
          </p>
        ) : (
          <p className="mt-2 text-[11px] leading-4 text-muted-foreground">
            A Git worktree on a new <span className="font-mono">bridge/fork/</span> branch. The parent repo is untouched.
          </p>
        )}
      </fieldset>

      {error && <p role="alert" className="mt-3 rounded-xl border border-destructive/30 bg-destructive/5 px-3 py-2.5 text-[12px] leading-4 text-destructive">{error}</p>}

      <div className="mt-5 flex flex-wrap justify-end gap-2 border-t border-border pt-4">
        <button type="button" disabled={busy} onClick={onClose} className="mr-auto min-h-8 rounded-lg px-3 text-[13px] text-muted-foreground hover:bg-accent hover:text-foreground disabled:opacity-30">Cancel</button>
        <button ref={submitRef} type="button" disabled={busy} onClick={submit} className="flex items-center justify-center gap-2 min-h-8 rounded-lg bg-primary px-3 text-[13px] font-medium text-primary-foreground transition-colors hover:bg-primary/90 disabled:opacity-30">
          <GitFork className="h-4 w-4" aria-hidden="true" />
          {busy ? "Forking…" : "Create fork"}
        </button>
      </div>
    </CreateDialogShell>
  );
}