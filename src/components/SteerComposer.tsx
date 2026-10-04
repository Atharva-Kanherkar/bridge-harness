import { useState, type ReactNode } from "react";
import { Navigation } from "lucide-react";
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

function SteerBox({ sessionId, steerable, onSteer, label = "Steer this worker…", className = "shrink-0 border-t border-border px-4 py-3 sm:px-6", trailing }: SteerComposerProps) {
  const [draft, setDraft] = useState("");
  const [busy, setBusy] = useState(false);
  const [failure, setFailure] = useState<string>();
  if (!steerable) {
    return <div className={cn(className, "flex items-center gap-2")} role="status">
      <span className="flex-1 text-[11px] text-muted-foreground">This worker has finished. Its typed result is final — ask the orchestrator to delegate a follow-up.</span>
      {trailing}
    </div>;
  }
  const send = () => {
    const sent = draft;
    const text = sent.trim();
    if (!text || busy) return;
    setBusy(true); setFailure(undefined);
    void onSteer(sessionId, text)
      // Typing can go on while the steer is in flight. Only the words that
      // were sent are cleared; anything written since stays.
      .then(() => setDraft(current => current === sent ? "" : current))
      .catch((cause: unknown) => setFailure(cause instanceof Error ? cause.message : String(cause)))
      .finally(() => setBusy(false));
  };
  return <form className={className} onSubmit={requested => { requested.preventDefault(); send(); }}>
    <div className="flex items-end gap-2">
      <textarea
        value={draft}
        onChange={changed => setDraft(changed.target.value)}
        onKeyDown={pressed => {
          // Enter that confirms an IME composition is not a send.
          if (pressed.key === "Enter" && !pressed.shiftKey && !pressed.nativeEvent.isComposing) { pressed.preventDefault(); send(); }
        }}
        rows={1}
        placeholder={label}
        aria-label={label}
        className="min-h-[34px] max-h-32 flex-1 resize-none rounded-xl border border-border bg-card px-3 py-2 text-[12px] text-foreground outline-none placeholder:text-muted-foreground focus-visible:ring-1 focus-visible:ring-ring"
      />
      <Button type="submit" size="sm" disabled={busy || !draft.trim()}><Navigation size={12}/>{busy ? "Sending…" : "Steer"}</Button>
      {trailing}
    </div>
    <p className="mt-1.5 text-[11px] text-muted-foreground">Guidance is folded into the worker&rsquo;s objective and its orchestrator is told. It still reports a typed result.</p>
    {failure && <p role="alert" className="mt-1.5 text-[11px] text-destructive">{failure}</p>}
  </form>;
}
