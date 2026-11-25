import { GitFork, GitBranch, ShieldAlert } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { CreateDialogShell } from "./CreateDialogShell";
import { Input } from "@/components/ui/input";
import { RowButton } from "@/components/ui/row";
import { Button } from "@/components/ui/button";

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
      <p className="text-ui leading-5 text-muted-foreground">
        The parent chat stays exactly as it is. The fork starts with this
        conversation's history up to this message and continues on its own.
      </p>

      <label className="mt-4 block">
        <span className="text-meta font-medium text-foreground">Fork title</span>
        <Input
          className="mt-1.5"
          value={title}
          onChange={event => setTitle(event.target.value)}
          placeholder={`Fork of ${sessionLabel}`}
          aria-label="Fork title"
        />
      </label>

      <fieldset className="mt-4">
        <legend className="text-meta font-medium text-foreground">Where the fork works</legend>
        <div role="radiogroup" aria-label="Where the fork works" className="mt-1.5 grid grid-cols-2 gap-2">
          <RowButton size="card" tone="choice" shape="card" role="radio" aria-checked={worktree === "shared"} data-worktree="shared" onClick={() => setWorktree("shared")}>
            <GitBranch className="h-4 w-4 shrink-0 text-muted-foreground" aria-hidden="true" />
            <span className="min-w-0">
              Share the workspace
              <span className="block text-caption leading-4 text-muted-foreground">same files as the parent</span>
            </span>
          </RowButton>
          <RowButton size="card" tone="choice" shape="card" role="radio" aria-checked={worktree === "new"} data-worktree="new" onClick={() => setWorktree("new")}>
            <GitFork className="h-4 w-4 shrink-0 text-muted-foreground" aria-hidden="true" />
            <span className="min-w-0">
              New worktree and branch
              <span className="block text-caption leading-4 text-muted-foreground">isolated copy of the repo</span>
            </span>
          </RowButton>
        </div>
        {worktree === "shared" ? (
          <p className="mt-2 flex items-start gap-1.5 text-caption leading-4 text-muted-foreground">
            <ShieldAlert className="mt-0.5 h-3.5 w-3.5 shrink-0" aria-hidden="true" />
            Two sessions writing the same files — a real hazard when both are live.
          </p>
        ) : (
          <p className="mt-2 text-caption leading-4 text-muted-foreground">
            A Git worktree on a new <span className="font-mono">bridge/fork/</span> branch. The parent repo is untouched.
          </p>
        )}
      </fieldset>

      {error && <p role="alert" className="mt-3 rounded-xl border border-destructive/30 bg-destructive/5 px-3 py-2.5 text-meta leading-4 text-destructive">{error}</p>}

      <div className="mt-5 flex flex-wrap justify-end gap-2 border-t border-border pt-4">
        <Button variant="quiet" size="sm" className="mr-auto" disabled={busy} onClick={onClose}>Cancel</Button>
        <Button variant="default" size="sm" ref={submitRef} disabled={busy} onClick={submit}>
          <GitFork className="h-4 w-4" aria-hidden="true" />
          {busy ? "Forking…" : "Create fork"}
        </Button>
      </div>
    </CreateDialogShell>
  );
}