import { Dialog, DialogPopup } from "@/components/ui/dialog";
import { ExternalLink, PanelRight } from "lucide-react";

// Where a GitHub link should open. Bridge can render a pull request, an issue
// or the repository beside the conversation, and the browser can render all of
// them — so when both are possible the reader picks, rather than the app
// deciding for them. This only ever appears for a link the pane could actually
// show; everything else has one destination and goes straight there.

export type GithubLinkDestinationDialogProps = {
  /** What the link points at, e.g. "Pull request #341 · Changes". */
  subject: string;
  /** The repository it belongs to, as `owner/name`. */
  repository: string;
  /** The full URL, shown so the choice is made against the real target. */
  url: string;
  onOpenInline: () => void;
  onOpenInBrowser: () => void;
  onCancel: () => void;
};

export function GithubLinkDestinationDialog({
  subject,
  repository,
  url,
  onOpenInline,
  onOpenInBrowser,
  onCancel,
}: GithubLinkDestinationDialogProps) {
  return <Dialog open onOpenChange={next => { if (!next) onCancel(); }}>
    <DialogPopup showCloseButton={false} aria-label="Open GitHub link" className="max-w-md p-5">
      <p className="text-xs text-muted-foreground">Open this in</p>
      <p className="mt-1 text-[15px] font-medium leading-snug text-foreground">{subject}</p>
      <p className="mt-0.5 font-mono text-[11px] text-muted-foreground">{repository}</p>
      {/* The URL is the thing being acted on, so it is shown rather than
        * summarised — a link written by an agent is worth reading before it
        * is followed. */}
      <p className="mt-3 break-all font-mono text-[11px] leading-relaxed text-muted-foreground">{url}</p>

      <div className="mt-5 flex flex-col gap-2 sm:flex-row sm:justify-end">
        <button
          type="button"
          onClick={onOpenInBrowser}
          className="inline-flex min-h-9 items-center justify-center gap-2 rounded-lg border border-border px-3 text-[13px] font-medium text-foreground transition-colors hover:bg-accent"
        >
          <ExternalLink size={13} aria-hidden="true" />
          Open in browser
        </button>
        <button
          type="button"
          autoFocus
          onClick={onOpenInline}
          className="inline-flex min-h-9 items-center justify-center gap-2 rounded-lg bg-primary px-3 text-[13px] font-medium text-primary-foreground transition-colors hover:bg-primary/90"
        >
          <PanelRight size={13} aria-hidden="true" />
          Open in Bridge
        </button>
      </div>
    </DialogPopup>
  </Dialog>;
}
