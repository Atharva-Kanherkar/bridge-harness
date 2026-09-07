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
    if (!open || busy) return;
    const handler = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [open, busy, onClose]);

  const canSubmit = !busy && title.trim().length > 0;

  return (
    <CreateDialogShell
      open={open}
      titleId="workspace-create-title"
      icon={<FolderGit2 className="h-4 w-4 text-muted-foreground" strokeWidth={1.5} aria-hidden="true" />}
      title="New workspace"
      subtitle="Group related chats. Connect a folder later."
      closeLabel="Close"
      closeDisabled={busy}
      onClose={onClose}
      dismissOnScrim
    >
      <label htmlFor="workspace-name" className="mb-2 block text-[13px] font-medium text-foreground">
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
          "h-9 w-full rounded-lg border border-input bg-card px-3",
          "text-[13px] text-foreground placeholder:text-muted-foreground",
          "transition-colors",
        )}
      />

      <div className="mt-6 flex flex-wrap justify-end gap-2 border-t border-border pt-4">
        <button
          type="button"
          disabled={busy}
          onClick={onClose}
          className="min-h-8 rounded-lg border border-border bg-card px-4 text-[13px] text-foreground transition-colors hover:bg-accent disabled:opacity-45"
        >
          Cancel
        </button>
        <button
          type="button"
          disabled={!canSubmit}
          onClick={onSubmit}
          className="min-h-8 rounded-lg bg-primary px-4 text-[13px] font-medium text-primary-foreground transition-colors hover:bg-primary/90 disabled:opacity-45"
        >
          {busy ? "Creating…" : "Create workspace"}
        </button>
      </div>
    </CreateDialogShell>
  );
}
