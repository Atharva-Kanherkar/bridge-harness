import { LoaderCircle } from "lucide-react";
import { Button } from "./ui/button";
import { Dialog, DialogContent, DialogDescription, DialogTitle } from "./ui/dialog";

export type CodexUpdatePhase = "installing" | "refreshing" | null;

export function CodexUpdateDialog({ open, phase, onOpenChange, onConfirm }: {
  open: boolean;
  phase: CodexUpdatePhase;
  onOpenChange: (open: boolean) => void;
  onConfirm: () => void;
}) {
  return <Dialog open={open} onOpenChange={onOpenChange}>
    <DialogContent showCloseButton={false} className="gap-4 p-6">
      <DialogTitle>{phase ? "Updating Codex CLI" : "Update Codex CLI?"}</DialogTitle>
      <DialogDescription>{phase
        ? "You can close this window. Bridge will notify you when the update finishes."
        : "Bridge will run the official Codex installer without terminal prompts:"}</DialogDescription>
      {phase ? <p role="status" className="flex items-center gap-2 text-sm text-foreground">
        <LoaderCircle size={16} className="shrink-0 animate-spin" aria-hidden="true" />
        {phase === "installing" ? "Downloading and installing Codex. This can take up to 4 minutes." : "Installation finished. Checking the Codex runtime…"}
      </p> : <code className="block break-all rounded-lg bg-muted p-3 font-mono text-xs text-foreground">curl -fsSL https://chatgpt.com/codex/install.sh | CODEX_NON_INTERACTIVE=1 sh</code>}
      <div className="flex justify-end gap-2">
        <Button variant="ghost" onClick={() => onOpenChange(false)}>{phase ? "Close" : "Ignore"}</Button>
        {!phase && <Button onClick={onConfirm}>Yes</Button>}
      </div>
    </DialogContent>
  </Dialog>;
}
