import { FolderGit2, X } from "lucide-react";
import { useEffect, useRef } from "react";
import { cn } from "@/lib/utils";

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
    <div
      className="fixed inset-0 z-50 flex items-start justify-center bg-black/45 p-4 pt-[10vh] backdrop-blur-md"
      onClick={event => {
        if (event.target === event.currentTarget) onClose();
      }}
      onKeyDown={event => {
        if (event.key === "Escape") onClose();
      }}
      role="dialog"
      aria-modal="true"
      aria-labelledby="workspace-create-title"
    >
      <div className="u-glass-popover animate-page-enter w-full max-w-md overflow-hidden rounded-3xl">
        <div className="relative border-b border-white/[0.06] px-5 py-4">
          <div className="absolute inset-0 bg-gradient-to-r from-white/[0.03] via-transparent to-white/[0.015]" />
          <div className="relative flex items-center justify-between gap-3">
            <div className="flex min-w-0 items-center gap-3">
              <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-[12px] border border-white/[0.08] bg-white/[0.05] shadow-[inset_0_1px_0_rgba(255,255,255,0.06)]">
                <FolderGit2 className="h-4 w-4 text-neutral-300" strokeWidth={1.5} aria-hidden="true" />
              </span>
              <div className="min-w-0">
                <h2 id="workspace-create-title" className="font-display text-base font-semibold text-white">
                  New workspace
                </h2>
                <p className="mt-0.5 text-[13px] text-neutral-500">
                  Group related chats. Connect a folder later.
                </p>
              </div>
            </div>
            <button
              type="button"
              onClick={onClose}
              disabled={busy}
              className="shrink-0 rounded-xl p-2 text-neutral-500 transition-colors hover:bg-white/[0.08] hover:text-neutral-200 disabled:opacity-40"
              aria-label="Close"
            >
              <X className="h-4 w-4" strokeWidth={2} aria-hidden="true" />
            </button>
          </div>
        </div>

        <div className="p-5">
          <label htmlFor="workspace-name" className="mb-2 block text-[11px] font-semibold uppercase tracking-wider text-neutral-600">
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
              "w-full rounded-2xl border border-white/[0.08] bg-white/[0.035] px-4 py-3.5",
              "text-[15px] tracking-[-0.006em] text-neutral-100 placeholder:text-neutral-600",
              "shadow-[inset_0_1px_0_rgba(255,255,255,0.04)]",
              "transition-all focus:border-white/[0.18] focus:bg-white/[0.055] focus:outline-none",
            )}
          />

          <div className="mt-5 flex gap-2">
            <button
              type="button"
              disabled={!canSubmit}
              onClick={onSubmit}
              className="flex-1 rounded-xl bg-white px-4 py-2.5 text-sm font-medium text-neutral-900 shadow-[inset_0_1px_0_rgba(255,255,255,0.5),0_10px_24px_-12px_rgba(0,0,0,0.6)] transition-all hover:brightness-105 active:scale-[0.98] disabled:opacity-30"
            >
              {busy ? "Creating…" : "Create workspace"}
            </button>
            <button
              type="button"
              disabled={busy}
              onClick={onClose}
              className="rounded-xl border border-white/[0.08] bg-white/[0.03] px-4 py-2.5 text-sm text-neutral-400 transition-colors hover:bg-white/[0.06] hover:text-neutral-200 disabled:opacity-30"
            >
              Cancel
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
