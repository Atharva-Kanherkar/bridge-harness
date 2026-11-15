import { useEffect, useState } from "react";
import { Clock3, TriangleAlert, X } from "lucide-react";
import { bridgeApi } from "../api";
import type { AdapterDescriptor } from "../types";
import type { LimitRecoveryView } from "../protocol/generated/protocol";
import { limitRecoveryNotice } from "../limitRecovery";

/** How often the banner rereads its row while Bridge is waiting. The
 * countdown is minutes, and the row changes state on its own. */
const POLL_MS = 30_000;

/** The chat's usage-limit recovery: when Bridge will continue it, with
 * Resume now and Cancel, or why it could not. Rereads when the chat's status
 * changes, which is when a limit lands or a continue turn starts. */
export function LimitRecoveryBanner({ sessionId, sessionStatus, adapters, onError }: {
  sessionId: string;
  sessionStatus: string;
  adapters: AdapterDescriptor[];
  onError?: (message: string) => void;
}) {
  const [view, setView] = useState<LimitRecoveryView | null>(null);
  const [dismissed, setDismissed] = useState<string>();
  const [busy, setBusy] = useState(false);
  const [now, setNow] = useState(() => new Date());
  useEffect(() => {
    let active = true;
    const read = () => void bridgeApi.limitRecoveryStatus(sessionId)
      .then(status => { if (active) { setView(status.recovery ?? null); setNow(new Date()); } })
      .catch(() => {});
    read();
    const timer = window.setInterval(read, POLL_MS);
    return () => { active = false; window.clearInterval(timer); };
  }, [sessionId, sessionStatus]);
  const notice = limitRecoveryNotice(view, adapters, now);
  if (!notice || !view || dismissed === `${view.state}:${view.updatedAt}`) return null;
  const act = async (action: "resume" | "cancel") => {
    setBusy(true);
    try {
      const status = action === "resume" ? await bridgeApi.resumeLimitRecovery(sessionId) : await bridgeApi.cancelLimitRecovery(sessionId);
      setView(status.recovery ?? null);
    } catch (cause) { onError?.(cause instanceof Error ? cause.message : String(cause)); }
    finally { setBusy(false); }
  };
  const Icon = notice.tone === "problem" ? TriangleAlert : Clock3;
  return <div className="mx-auto mb-2 flex max-w-conversation justify-center px-4 sm:px-6" data-limit-recovery={view.state}>
    <div className="u-glass-soft inline-flex max-w-full flex-wrap items-center gap-x-3 gap-y-1 rounded-2xl px-3.5 py-1.5 text-xs text-muted-foreground" role="status">
      <Icon size={12} aria-hidden="true" className="shrink-0" />
      <span className="min-w-0">{notice.text}</span>
      {notice.actionable ? <span className="flex items-center gap-2">
        <button type="button" disabled={busy} onClick={() => void act("resume")} className="font-medium text-foreground hover:underline disabled:opacity-45">Resume now</button>
        <button type="button" disabled={busy} onClick={() => void act("cancel")} className="hover:text-foreground hover:underline disabled:opacity-45">Cancel</button>
      </span> : notice.tone === "problem" && <button type="button" aria-label="Dismiss" onClick={() => setDismissed(`${view.state}:${view.updatedAt}`)} className="grid size-5 place-items-center rounded-full hover:bg-accent hover:text-foreground"><X size={11} aria-hidden="true" /></button>}
    </div>
  </div>;
}
