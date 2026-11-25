import { FolderOpen, MessageSquareText } from "lucide-react";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogPanel, DialogTitle } from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";

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
        <DialogPanel className="grid gap-3">
          <Button variant="outline" size="xs"
           
            disabled={busy || !canStartChat}
            onClick={onStartChat}
           
          >
            <MessageSquareText size={18} strokeWidth={1.7} className="mb-2 text-muted-foreground transition-colors group-hover:text-foreground" aria-hidden="true" />
            <span className="text-ui font-semibold text-foreground">Start a chat</span>
            <span className="mt-1 text-ui leading-relaxed text-muted-foreground">
              {canStartChat ? "Begin without choosing a repository yet." : "Install or sign in to a model adapter first."}
            </span>
          </Button>
          <Button variant="outline" size="xs"
           
            disabled={busy}
            onClick={onChooseFolder}
           
          >
            <FolderOpen size={18} strokeWidth={1.7} className="mb-2 text-muted-foreground transition-colors group-hover:text-foreground" aria-hidden="true" />
            <span className="text-ui font-semibold text-foreground">Choose a folder</span>
            <span className="mt-1 text-ui leading-relaxed text-muted-foreground">Open the standard folder picker and connect an existing project.</span>
          </Button>
        </DialogPanel>
      </DialogContent>
    </Dialog>
  );
}
