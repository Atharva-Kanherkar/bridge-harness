import { Button } from "./ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogPanel, DialogTitle } from "@/components/ui/dialog";
import { Skeleton, SkeletonRegion } from "@/components/ui/skeleton";

export type CodexUpdatePhase = "installing" | "refreshing" | null;

export function CodexUpdateDialog({ open, phase, onOpenChange, onConfirm }: {
  open: boolean;
  phase: CodexUpdatePhase;
  onOpenChange: (open: boolean) => void;
  onConfirm: () => void;
}) {
  return <Dialog open={open} onOpenChange={onOpenChange}>
    <DialogContent showCloseButton={false}>
      <DialogHeader>
        <DialogTitle>{phase ? "Updating Codex CLI" : "Update Codex CLI?"}</DialogTitle>
        <DialogDescription>{phase
          ? "You can close this window. Bridge will notify you when the update finishes."
          : "Bridge will run the official Codex installer without terminal prompts:"}</DialogDescription>
      </DialogHeader>
      <DialogPanel>
      {phase ? <SkeletonRegion label={phase === "installing" ? "Installing Codex" : "Verifying Codex"}><Skeleton className="h-1 w-full rounded-full" /></SkeletonRegion> : <code className="block break-all rounded-lg bg-muted p-3 font-mono text-xs text-foreground">curl -fsSL https://chatgpt.com/codex/install.sh | CODEX_NON_INTERACTIVE=1 sh</code>}
      </DialogPanel>
      <DialogFooter variant="bare">
        <Button variant="ghost" onClick={() => onOpenChange(false)}>{phase ? "Close" : "Ignore"}</Button>
        {!phase && <Button onClick={onConfirm}>Yes</Button>}
      </DialogFooter>
    </DialogContent>
  </Dialog>;
}
