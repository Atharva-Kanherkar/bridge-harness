import { FolderOpen, MessageSquareText } from "lucide-react";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogPanel, DialogTitle } from "@/components/ui/dialog";

export function NewProjectDialog({ open, busy, canStartChat, onClose, onStartChat, onChooseFolder }: {
  open: boolean;
  busy: boolean;
  canStartChat: boolean;
  onClose: () => void;
  onStartChat: () => void;
  onChooseFolder: () => void;
}) {
  return (
    <Dialog open={open} onOpenChange={next => { if (!next && !busy) onClose(); }}>
      <DialogContent showCloseButton={!busy}>
        <DialogHeader>
          <DialogTitle>New project</DialogTitle>
          <DialogDescription>Start with a conversation, or connect a folder from this Mac.</DialogDescription>
        </DialogHeader>
        <DialogPanel className="grid gap-2 sm:grid-cols-2">
          <button
            type="button"
            disabled={busy || !canStartChat}
            onClick={onStartChat}
            className="u-glass-soft group flex min-h-28 flex-col items-start rounded-xl border border-border p-4 text-left outline-none transition-colors hover:border-ring/50 hover:bg-accent focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-1 focus-visible:ring-offset-background disabled:cursor-not-allowed disabled:opacity-45"
          >
            <MessageSquareText size={18} strokeWidth={1.7} className="mb-4 text-muted-foreground transition-colors group-hover:text-foreground" aria-hidden="true" />
            <span className="text-[13px] font-semibold text-foreground">Start a chat</span>
            <span className="mt-1 text-[11.5px] leading-relaxed text-muted-foreground">
              {canStartChat ? "Begin without choosing a repository yet." : "Install or sign in to a model adapter first."}
            </span>
          </button>
          <button
            type="button"
            disabled={busy}
            onClick={onChooseFolder}
            className="u-glass-soft group flex min-h-28 flex-col items-start rounded-xl border border-border p-4 text-left outline-none transition-colors hover:border-ring/50 hover:bg-accent focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-1 focus-visible:ring-offset-background disabled:cursor-not-allowed disabled:opacity-45"
          >
            <FolderOpen size={18} strokeWidth={1.7} className="mb-4 text-muted-foreground transition-colors group-hover:text-foreground" aria-hidden="true" />
            <span className="text-[13px] font-semibold text-foreground">Choose a folder</span>
            <span className="mt-1 text-[11.5px] leading-relaxed text-muted-foreground">Open the standard folder picker and connect an existing project.</span>
          </button>
        </DialogPanel>
      </DialogContent>
    </Dialog>
  );
}
