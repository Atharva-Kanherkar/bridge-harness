import { useEffect, useRef, useState } from "react";
import { AlertTriangle, Bot, Camera, ChevronRight, Ghost, Hand, LoaderCircle, Trash2 } from "lucide-react";
import { bridgeApi } from "../api";
import { startSerialPoll } from "../polling";
import type { BrowserCloneSnapshot, BrowserCloneStatus, CloneSignInPath } from "../types";
import { PaneState } from "./ui/pane";
import { Badge } from "./ui/badge";
import { Button } from "./ui/button";

// The dock tenant for a throwaway browser clone: the agent drives a private
// copy of the browser, and this is where the person watches it, takes it over
// for a login or 2FA, or destroys it. It mirrors BrowserSurface (a root that
// fills its host, a poll that runs hot while visible, and a supervision report
// upward) but reads clone state, not a lease on one of your own tabs.

const statusLabel: Record<BrowserCloneStatus, string> = {
  none: "No clone", starting: "Starting", acting: "Acting", waiting_for_you: "Waiting for you", taken_over: "You’re in control", destroyed: "Destroyed",
};
const signInLabel: Record<CloneSignInPath, string> = { import: "Imported sign-in", sign_in_inside: "Signed in inside the clone" };

/** Poll cadences, the same shape as the attached-tab surface: visible polls run
 *  hot because the page changes under the agent while you watch; a hidden pane
 *  that still holds a live clone keeps a slow heartbeat so waiting_for_you can
 *  reach the switcher; hidden with no clone there is nothing to watch. */
export const CLONE_POLL_VISIBLE_MS = 700;
export const CLONE_POLL_HIDDEN_MS = 5000;

export type CloneSupervision = { status: BrowserCloneStatus; attention: boolean };

const isLive = (status: BrowserCloneStatus) => status === "starting" || status === "acting" || status === "waiting_for_you" || status === "taken_over";
const minutesLeft = (expiresAt: string | null) => expiresAt ? Math.max(0, Math.ceil((Date.parse(expiresAt) - Date.now()) / 60_000)) : undefined;

export function CloneSurface({ visible = true, sessionId, onClose, onError, onSupervisionChange }: {
  /** False while another dock pane is showing. The surface stays mounted, so
   *  the clone keeps running, but polling backs off or stops. */
  visible?: boolean;
  /** The session this pane belongs to. Its clone (if any) is what shows here;
   *  without it, only the mock fixture is exercisable (dev and tests). */
  sessionId?: string;
  onClose?: () => void;
  onError: (message: string) => void;
  onSupervisionChange?: (state: CloneSupervision) => void;
}) {
  const [snapshot, setSnapshot] = useState<BrowserCloneSnapshot>();
  const [busy, setBusy] = useState(false);
  const [confirmingDestroy, setConfirmingDestroy] = useState(false);
  const [domainDraft, setDomainDraft] = useState("");
  const latestRequest = useRef(0);

  const refresh = async () => {
    // Only the newest read lands: an action's refresh must not be overwritten
    // by a slower poll that started before it.
    const request = ++latestRequest.current;
    const next = await bridgeApi.browserCloneState(sessionId);
    if (request === latestRequest.current) setSnapshot(next);
  };
  const status = snapshot?.status ?? "none";
  const live = isLive(status);
  useEffect(() => {
    if (!visible && !live) return;
    return startSerialPoll(refresh, visible ? CLONE_POLL_VISIBLE_MS : CLONE_POLL_HIDDEN_MS);
  }, [visible, live]);
  const run = async (task: () => Promise<unknown>) => {
    setBusy(true);
    try { await task(); await refresh(); }
    catch (error) { onError(error instanceof Error ? error.message : String(error)); }
    finally { setBusy(false); setConfirmingDestroy(false); }
  };

  const attention = status === "waiting_for_you" || !!snapshot?.pendingApproval;
  const supervisionRef = useRef<string>();
  useEffect(() => {
    const signature = `${status}:${attention}`;
    if (supervisionRef.current === signature) return;
    supervisionRef.current = signature;
    onSupervisionChange?.({ status, attention });
  }, [status, attention, onSupervisionChange]);

  const remaining = minutesLeft(snapshot?.expiresAt ?? null);
  const subtitle = live
    ? [snapshot?.signInPath && signInLabel[snapshot.signInPath], remaining !== undefined && `expires in ${remaining} min`].filter(Boolean).join(" · ")
    : status === "destroyed" ? "Profile and cookies wiped" : "A throwaway browser for signed-in work";

  return <div className="relative flex h-full min-w-0 flex-col overflow-hidden bg-background">
    <header className="flex min-h-14 shrink-0 items-center gap-2 border-b border-border bg-muted/25 px-3 py-2">
      <Ghost size={15} className="shrink-0 text-muted-foreground" aria-hidden="true" />
      <div className="min-w-0 flex-1">
        <div className="truncate text-[11px] font-medium text-foreground">{snapshot?.domain ?? "Browser clone"}</div>
        <div className="truncate text-[11px] text-muted-foreground">{subtitle}</div>
      </div>
      <Badge variant={status === "waiting_for_you" ? "warning" : status === "taken_over" ? "info" : status === "acting" || status === "starting" ? "success" : "outline"} size="sm">{statusLabel[status]}</Badge>
      {onClose && <Button variant="ghost" size="icon-xs" onClick={onClose} aria-label="Close Clone Surface"><ChevronRight size={14} /></Button>}
    </header>

    {!snapshot ? <div className="grid flex-1 place-items-center text-muted-foreground"><LoaderCircle className="animate-spin" size={18} /></div>
      : status === "none" ? <div className="flex flex-1 flex-col items-center justify-center gap-4 px-6 text-center">
          <Ghost size={22} className="text-muted-foreground" aria-hidden="true" />
          <p className="max-w-sm text-[11px] leading-5 text-muted-foreground">A clone is a throwaway copy of your browser, signed in only to the site you name. Start one to test a signed-in flow; it is destroyed when you are done.</p>
          <form className="flex w-full max-w-sm items-center gap-1.5" onSubmit={(event) => { event.preventDefault(); const domain = domainDraft.trim(); if (domain) void run(() => bridgeApi.requestClone(sessionId ?? "", domain, "chrome", "import").then(() => setDomainDraft(""))); }}>
            <input aria-label="Site to clone" value={domainDraft} onChange={(event) => setDomainDraft(event.target.value)} placeholder="youtube.com" className="h-8 min-w-0 flex-1 rounded-md border border-border bg-background px-2.5 text-[12px] text-foreground placeholder:text-muted-foreground focus:outline-none focus:ring-1 focus:ring-ring" />
            <Button type="submit" size="xs" disabled={busy || !domainDraft.trim()}><Ghost size={12} />Start clone</Button>
          </form>
        </div>
      : status === "destroyed" ? <PaneState icon={Trash2} title="Clone destroyed">Its profile, cookies, and session are gone. Nothing from it stays on disk.</PaneState>
      : <>
        {status === "waiting_for_you" && <div role="status" className="flex items-start gap-2 border-b border-warning/30 bg-warning/10 px-3 py-2 text-[11px] leading-4 text-warning"><Hand size={14} className="mt-0.5 shrink-0" /><span><b className="font-semibold">The clone is waiting for you.</b> {snapshot.waitingReason ?? "Take over to sign in, then hand it back."}</span></div>}
        {status === "taken_over" && <div role="status" className="flex items-start gap-2 border-b border-info/30 bg-info/10 px-3 py-2 text-[11px] leading-4 text-info"><Hand size={14} className="mt-0.5 shrink-0" /><span><b className="font-semibold">You’re in control.</b> The agent is paused until you hand the clone back.</span></div>}
        {snapshot.screenshotRedactedRegions > 0 && <div className="border-b border-border px-3 py-1 text-[11px] text-muted-foreground">{snapshot.screenshotRedactedRegions} sensitive region{snapshot.screenshotRedactedRegions === 1 ? "" : "s"} redacted before persistence</div>}

        <div className="min-h-0 flex-1 overflow-auto">
          <LiveView snapshot={snapshot} />
        </div>

        <div className="flex flex-wrap items-center gap-1.5 border-t border-border p-2">
          {status === "taken_over"
            ? <Button size="xs" disabled={busy} onClick={() => void run(() => bridgeApi.handBackBrowserClone(sessionId))}><Bot size={12} />Hand back</Button>
            : <Button variant={status === "waiting_for_you" ? "default" : "secondary"} size="xs" disabled={busy || status === "starting"} onClick={() => void run(() => bridgeApi.takeoverBrowserClone(sessionId))}><Hand size={12} />Take over</Button>}
          {confirmingDestroy
            ? <div className="ml-auto flex flex-wrap items-center gap-1.5">
              <span className="text-[11px] text-muted-foreground">Wipe this clone’s profile and cookies?</span>
              <Button variant="ghost" size="xs" disabled={busy} onClick={() => setConfirmingDestroy(false)}>Cancel</Button>
              <Button variant="destructive" size="xs" disabled={busy} onClick={() => void run(() => bridgeApi.destroyBrowserClone(sessionId))}><Trash2 size={12} />Destroy clone</Button>
            </div>
            : <Button variant="ghost" size="xs" disabled={busy} onClick={() => setConfirmingDestroy(true)} className="ml-auto text-muted-foreground"><Trash2 size={12} />Destroy</Button>}
        </div>
      </>}

    {snapshot?.pendingApproval && <div className="absolute inset-0 z-20 grid place-items-center overflow-y-auto bg-background/80 p-5 backdrop-blur-sm">
      <div role="region" aria-label="Sensitive clone action" className="u-glass-popover w-full max-w-sm rounded-xl p-5">
        <div className="flex items-center gap-2 text-sm font-semibold text-foreground"><AlertTriangle size={16} className="shrink-0 text-warning" />Sensitive action pending</div>
        <p className="mt-2 text-[13px] leading-relaxed text-muted-foreground">{snapshot.pendingApproval.effect}</p>
        <p className="mt-1 break-all font-mono text-[11px] text-muted-foreground">{snapshot.pendingApproval.domain}</p>
        <div className="mt-4 flex justify-end gap-2">
          <Button variant="ghost" size="sm" disabled={busy} onClick={() => void run(() => bridgeApi.resolveBrowserCloneApproval(snapshot.pendingApproval!.id, false))}>Deny</Button>
          <Button size="sm" disabled={busy} onClick={() => void run(() => bridgeApi.resolveBrowserCloneApproval(snapshot.pendingApproval!.id, true))}>Approve once</Button>
        </div>
      </div>
    </div>}
  </div>;
}

function LiveView({ snapshot }: { snapshot: BrowserCloneSnapshot }) {
  if (!snapshot.screenshot) return <div className="grid min-h-full place-items-center p-6 text-center"><div><Camera size={24} className="mx-auto text-muted-foreground" /><p className="mt-2 max-w-sm text-[11px] leading-5 text-muted-foreground">{snapshot.status === "starting" ? "Starting the clone…" : "Waiting for the first redacted frame…"}</p></div></div>;
  return <div data-clone-viewport><img src={snapshot.screenshot} alt="Redacted live view of the browser clone" className="block h-auto w-full" /></div>;
}
