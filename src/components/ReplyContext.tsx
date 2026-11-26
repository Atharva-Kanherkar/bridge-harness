import { Reply, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { RowButton } from "@/components/ui/row";
import { replyPreview, type ReplyTarget } from "@/replyTo";

/**
 * The reply the composer answers, attached to the top of its frame like the
 * work shelf: "Replying to" and how the message starts, which jumps back to it,
 * and ✕ to let it go. Modeled on T3 Code's reply banner.
 */
export function ComposerReplyBanner({ target, onCancel, onReveal }: {
  target: ReplyTarget;
  onCancel?: () => void;
  /** Scroll to the message being answered; absent when it has no entry to find. */
  onReveal?: () => void;
}) {
  const preview = replyPreview(target.excerpt);
  return <div data-composer-reply className="flex h-9 shrink-0 items-center gap-2 border-b border-border pl-3 pr-1.5 text-meta text-muted-foreground">
    <Reply size={13} className="shrink-0" aria-hidden="true" />
    <span className="shrink-0 font-medium text-foreground">Replying to</span>
    {onReveal
      ? <RowButton size="bare" tone="plain" shape="rounded" className="min-w-0 flex-1" onClick={onReveal} aria-label={`Go to the response this replies to: ${preview}`}>
        <span className="min-w-0 truncate text-meta">{preview}</span>
      </RowButton>
      : <span className="min-w-0 flex-1 truncate">{preview}</span>}
    {onCancel && <Button variant="quiet" size="icon-2xs" aria-label="Cancel reply" tooltip="Cancel reply" onClick={onCancel}><X aria-hidden="true" /></Button>}
  </div>;
}

/**
 * The response a sent message replies to, at the top of its bubble: how it
 * starts, two lines at most. Clicking it scrolls to the original and
 * highlights it. Modeled on T3 Code's `ResponseReplyCard`.
 */
export function ReplyQuoteCard({ target, onReveal }: { target: ReplyTarget; onReveal?: (entryId: string) => void }) {
  const preview = replyPreview(target.excerpt);
  const body = <span className="block min-w-0 whitespace-normal rounded-lg bg-foreground/5 px-2.5 py-1.5 text-left text-meta text-muted-foreground transition-colors group-hover/reply:bg-foreground/10">
    <span className="mb-0.5 flex items-center gap-1 text-caption font-medium text-foreground/80">
      <Reply size={12} className="shrink-0" aria-hidden="true" />
      Replying to
    </span>
    <span className="line-clamp-2 break-words">{preview}</span>
  </span>;
  const entryId = target.entryId;
  if (!entryId || !onReveal) return <div data-reply-card className="mb-2">{body}</div>;
  return <RowButton
    data-reply-card
    size="bare"
    tone="plain"
    shape="card"
    className="group/reply mb-2"
    aria-label={`Go to the response this replies to: ${preview}`}
    onClick={() => onReveal(entryId)}
  >{body}</RowButton>;
}
