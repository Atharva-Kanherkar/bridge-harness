import { FolderGit2 } from "lucide-react";
import { useEffect, useRef } from "react";
import { cn } from "@/lib/utils";
import { CreateDialogShell } from "./CreateDialogShell";

export type WorkspaceCreateDialogProps = {
  open: boolean;
  title: string;
  busy?: boolean;
  onTitleChange: (value: string) => void;
  onClose: () => void;
  onSubmit: () => void;
};

export function WorkspaceCreateDialog({
  open,
  title,
  busy,
  onTitleChange,
  onClose,
  onSubmit,
}: WorkspaceCreateDialogProps) {
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (!open) return;
    inputRef.current?.focus();
  }, [open]);

  useEffect(() => {
    if (!open) return;
    const handler = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [open, onClose]);

  if (!open) return null;

  const canSubmit = !busy && title.trim().length > 0;

  return (
    <CreateDialogShell
      titleId="workspace-create-title"
      icon={<FolderGit2 className="h-4 w-4 text-muted-foreground" strokeWidth={1.5} aria-hidden="true" />}
      title="New workspace"
      subtitle="Group related chats. Connect a folder later."
      closeLabel="Close"
      closeDisabled={busy}
      onClose={onClose}
      dismissOnScrim
    >
      <label htmlFor="workspace-name" className="mb-2 block text-[11px] font-semibold uppercase tracking-wider text-muted-foreground">
        Workspace name
      </label>
      <input
        ref={inputRef}
        id="workspace-name"
        type="text"
        value={title}
        disabled={busy}
        placeholder="e.g. Payments service"
        onChange={event => onTitleChange(event.target.value)}
        onKeyDown={event => {
          if (event.key === "Enter" && canSubmit) {
            event.preventDefault();
            onSubmit();
          }
        }}
        className={cn(
          "w-full rounded-2xl border border-input bg-card px-4 py-3.5",
          "text-[15px] tracking-[-0.006em] text-foreground placeholder:text-muted-foreground/70",
          "transition-colors",
        )}
      />

      <div className="mt-5 flex flex-wrap gap-2">
        <button
          type="button"
          disabled={!canSubmit}
          onClick={onSubmit}
          className="min-w-40 flex-1 rounded-xl bg-primary px-4 py-2.5 text-sm font-medium text-primary-foreground transition-all hover:bg-primary/90 active:scale-[0.98] disabled:opacity-30"
        >
          {busy ? "Creating…" : "Create workspace"}
        </button>
        <button
          type="button"
          disabled={busy}
          onClick={onClose}
          className="rounded-xl border border-border bg-card px-4 py-2.5 text-sm text-muted-foreground transition-colors hover:bg-accent hover:text-foreground disabled:opacity-30"
        >
          Cancel
        </button>
      </div>
    </CreateDialogShell>
  );
}
