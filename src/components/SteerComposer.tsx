import { useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { LoaderCircle, Navigation } from "lucide-react";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";

/// Guidance into a running worker, from the surface where you can see it going
/// wrong.
///
/// Labelled as steering, not chatting: the worker still answers to the objective
/// its orchestrator gave it, and what you type amends that objective rather than
/// starting a conversation. Hidden once the worker has reported, because at that
/// point the result is final and offering the box would be a lie.
///
/// One box per worker: the same slot can show a different worker next, and a
/// draft, a steer still in flight, or a refusal must not carry over to it.
export function SteerComposer(props: SteerComposerProps) {
  return <SteerBox key={props.sessionId} {...props}/>;
}

interface SteerComposerProps {
  sessionId: string;
  steerable: boolean;
  /** Live but not yet taking steers (starting, resuming). Without it a
   *  worker that cannot be steered reads as finished. */
  starting?: boolean;
  onSteer: (sessionId: string, text: string) => Promise<void>;
  label?: string;
  /** Container chrome. The overlay wants a full-width divider; the docked
   *  composer in a session pane is already inside its own gutter. */
  className?: string;
  /** Rendered beside the Steer button (and beside the finished-worker notice) —
   *  a worker view has no chat ComposerPill of its own, so anything that needs
   *  to live "next to send" for every session, usage health included, has to
   *  be threaded in here too. */
  trailing?: ReactNode;
}

function SteerBox({ sessionId, steerable, starting = false, onSteer, label = "Steer this worker…", className = "shrink-0 border-t border-border px-4 py-3 sm:px-6", trailing }: SteerComposerProps) {
  const [draft, setDraft] = useState("");
  const [busy, setBusy] = useState(false);
  const [failure, setFailure] = useState<string>();
  const field = useRef<HTMLTextAreaElement>(null);
  // Grow with the draft up to a few lines, then scroll. A fixed one-row box
  // shorter than its own line height scrolled from the first keystroke, and
  // WebKit drew both scrollbars over the placeholder.
  useLayoutEffect(() => {
    const box = field.current;
    if (!box) return;
    box.style.height = "0px";
    const height = Math.min(box.scrollHeight, MAX_FIELD_HEIGHT);
    box.style.height = `${height}px`;
    box.style.overflowY = box.scrollHeight > MAX_FIELD_HEIGHT ? "auto" : "hidden";
  }, [draft, steerable]);
  if (!steerable && starting) {
    return <div className={cn(className, "flex items-center gap-2")} role="status">
      <LoaderCircle size={12} className="shrink-0 animate-spin text-muted-foreground" aria-hidden="true" />
      <span className="flex-1 text-caption text-muted-foreground">Starting up. You can steer this worker once it is running.</span>
      {trailing}
    </div>;
  }
  if (!steerable) {
    return <div className={cn(className, "flex items-center gap-2")} role="status">
      <span className="flex-1 text-caption text-muted-foreground">This worker has finished. Its typed result is final, so ask the orchestrator for a follow-up.</span>
      {trailing}
    </div>;
  }
  const send = () => {
    const sent = draft;
    const text = sent.trim();
    if (!text || busy) return;
    setDraft(""); setBusy(true); setFailure(undefined);
    void onSteer(sessionId, text)
      .catch((cause: unknown) => {
        setDraft(current => current ? `${sent}\n${current}` : sent);
        setFailure(cause instanceof Error ? cause.message : String(cause));
      })
      .finally(() => setBusy(false));
  };
  return <form className={className} onSubmit={requested => { requested.preventDefault(); send(); }}>
    <div className="flex items-end gap-2">
      <div className="flex min-w-0 flex-1 items-end gap-1.5 rounded-xl border border-border bg-card py-1 pl-3 pr-1 transition-colors focus-within:border-ring">
        <textarea
          ref={field}
          value={draft}
          onChange={changed => setDraft(changed.target.value)}
          onKeyDown={pressed => {
            // Enter that confirms an IME composition is not a send.
            if (pressed.key === "Enter" && !pressed.shiftKey && !pressed.nativeEvent.isComposing) { pressed.preventDefault(); send(); }
          }}
          rows={1}
          placeholder={label}
          aria-label={label}
          className="my-1 min-w-0 flex-1 resize-none overflow-hidden bg-transparent text-meta leading-5 text-foreground outline-none placeholder:text-muted-foreground"
        />
        {/* Quiet until there is something to send: a disabled primary button
            is a bright block in the dark theme. */}
        <Button type="submit" size="xs" variant={draft.trim() || busy ? "default" : "ghost"} disabled={busy || !draft.trim()} className="shrink-0">
          {busy ? <LoaderCircle size={12} className="animate-spin" aria-hidden="true"/> : <Navigation size={12} aria-hidden="true"/>}
          {busy ? "Sending…" : "Steer"}
        </Button>
      </div>
      {trailing}
    </div>
    <p role={busy ? "status" : undefined} className="mt-1.5 px-1 text-caption text-faint">{busy ? "Sending guidance…" : "Amends its task, mid-turn where supported, otherwise next turn."}</p>
    {failure && <p role="alert" className="mt-1 px-1 text-caption text-destructive">{failure}</p>}
  </form>;
}

/** About five lines of `text-meta` before the box scrolls. */
const MAX_FIELD_HEIGHT = 112;
