import { useEffect, useRef, useState, type KeyboardEvent as ReactKeyboardEvent, type MouseEvent as ReactMouseEvent, type WheelEvent as ReactWheelEvent } from "react";
import { AlertTriangle, ArrowRight, Bot, ChevronRight, Eye, EyeOff, Globe, Hand, LoaderCircle, Lock, MousePointer2, Timer, Trash2 } from "lucide-react";
import { bridgeApi } from "../api";
import { startSerialPoll } from "../polling";
import type { BrowserCloneSnapshot, BrowserCloneStatus, CloneSignInPath } from "../types";
import type { CloneInputEvent } from "../protocol/generated/protocol";
import { CloneConsentCard, SignInToggle } from "./CloneRequestInbox";
import { PaneState } from "./ui/pane";
import { Button } from "./ui/button";
import { cn } from "@/lib/utils";

// The dock tenant for a throwaway browser clone: the agent drives a private
// copy of the browser, and this is where the person watches it, takes it over
// for a login or 2FA, or destroys it. It mirrors BrowserSurface (a root that
// fills its host, a poll that runs hot while visible, and a supervision report
// upward) but reads clone state, not a lease on one of your own tabs.

const statusLabel: Record<BrowserCloneStatus, string> = {
  none: "No clone", requested: "Awaiting your approval", starting: "Starting", acting: "Acting", waiting_for_you: "Waiting for you", taken_over: "You’re in control", destroyed: "Destroyed",
};
const signInLabel: Record<CloneSignInPath, string> = { import: "Signed in as you", sign_in_inside: "Blank profile" };

/** Poll cadences, the same shape as the attached-tab surface: visible polls run
 *  hot because the page changes under the agent while you watch; a hidden pane
 *  that still holds a live clone keeps a slow heartbeat so waiting_for_you can
 *  reach the switcher; hidden with no clone there is nothing to watch. */
export const CLONE_POLL_VISIBLE_MS = 700;
export const CLONE_POLL_HIDDEN_MS = 5000;

export type CloneSupervision = { status: BrowserCloneStatus; attention: boolean };

const isLive = (status: BrowserCloneStatus) => status === "starting" || status === "acting" || status === "waiting_for_you" || status === "taken_over";
const minutesLeft = (expiresAt: string | null) => expiresAt ? Math.max(0, Math.ceil((Date.parse(expiresAt) - Date.now()) / 60_000)) : undefined;
const message = (error: unknown) => error instanceof Error ? error.message : String(error);

function StatusDot({ status }: { status: BrowserCloneStatus }) {
  return <span aria-hidden="true" className={cn("size-1.5 shrink-0 rounded-full",
    status === "acting" || status === "starting" ? "animate-pulse bg-foreground"
      : status === "waiting_for_you" || status === "requested" ? "bg-warning"
      : status === "taken_over" ? "bg-info" : "bg-muted-foreground/40")} />;
}

export function CloneSurface({ visible = true, sessionId, agentLabel = "Claude", onCancelStart, onClose, onError, onSupervisionChange }: {
  /** False while another dock pane is showing. The surface stays mounted, so
   *  the clone keeps running, but polling backs off or stops. */
  visible?: boolean;
  /** The session this pane belongs to. Its clone (if any) is what shows here;
   *  without it, only the mock fixture is exercisable (dev and tests). */
  sessionId?: string;
  /** Names the agent's pointer on the page. */
  agentLabel?: string;
  /** Set when a host shows this surface only while a clone is wanted: the
   *  start form then offers a way back. */
  onCancelStart?: () => void;
  onClose?: () => void;
  onError: (message: string) => void;
  onSupervisionChange?: (state: CloneSupervision) => void;
}) {
  const [snapshot, setSnapshot] = useState<BrowserCloneSnapshot>();
  const [busy, setBusy] = useState(false);
  const [confirmingDestroy, setConfirmingDestroy] = useState(false);
  const [domainDraft, setDomainDraft] = useState("");
  const [startPath, setStartPath] = useState<CloneSignInPath>("import");
  const latestRequest = useRef(0);
  const currentSession = useRef(sessionId);
  currentSession.current = sessionId;

  const refresh = async () => {
    if (currentSession.current !== sessionId) return;
    // Only the newest read lands: an action's refresh must not be overwritten
    // by a slower poll that started before it.
    const request = ++latestRequest.current;
    const next = await bridgeApi.browserCloneState(sessionId);
    if (request === latestRequest.current && currentSession.current === sessionId) setSnapshot(next);
  };
  const status = snapshot?.status ?? "none";
  const live = isLive(status);
  useEffect(() => {
    ++latestRequest.current;
    setSnapshot(undefined);
    setBusy(false);
    setConfirmingDestroy(false);
    setDomainDraft("");
    return () => { ++latestRequest.current; };
  }, [sessionId]);
  useEffect(() => {
    let active = true;
    bridgeApi.readCloneSettings().then(({ settings }) => { if (active) setStartPath(settings.defaultSignInPath); }).catch(() => undefined);
    return () => { active = false; };
  }, []);
  useEffect(() => {
    if (!visible && !live) return;
    return startSerialPoll(refresh, visible ? CLONE_POLL_VISIBLE_MS : CLONE_POLL_HIDDEN_MS);
  }, [visible, live, sessionId]);
  const run = async (task: () => Promise<unknown>) => {
    setBusy(true);
    try { await task(); await refresh(); }
    catch (error) { onError(message(error)); }
    finally { if (currentSession.current === sessionId) { setBusy(false); setConfirmingDestroy(false); } }
  };
  const send = (input: CloneInputEvent) => { void bridgeApi.cloneInput(sessionId ?? "", input).catch(error => onError(message(error))); };

  const attention = status === "requested" || status === "waiting_for_you" || !!snapshot?.pendingApproval;
  const supervisionRef = useRef<string>();
  useEffect(() => {
    const signature = `${status}:${attention}`;
    if (supervisionRef.current === signature) return;
    supervisionRef.current = signature;
    onSupervisionChange?.({ status, attention });
  }, [status, attention, onSupervisionChange]);

  const remaining = minutesLeft(snapshot?.expiresAt ?? null);
  const start = () => {
    const domain = domainDraft.trim();
    if (domain) void run(() => bridgeApi.requestClone(sessionId ?? "", domain, "chrome", startPath).then(() => setDomainDraft("")));
  };

  return <div className="relative flex h-full min-w-0 flex-col overflow-hidden bg-background">
    <header className="flex h-12 shrink-0 items-center gap-2 border-b border-border px-2.5">
      <div className="flex h-8 min-w-0 flex-1 items-center gap-2 rounded-lg border border-border bg-muted/40 px-2.5">
        <StatusDot status={status} />
        {live ? <Lock size={12} className="shrink-0 text-muted-foreground" aria-hidden="true" /> : <Globe size={12} className="shrink-0 text-muted-foreground" aria-hidden="true" />}
        <span className={cn("min-w-0 truncate text-[12px] text-foreground", snapshot?.domain && "font-mono")}>{snapshot?.domain ?? "Browser clone"}</span>
        <span className="ml-auto shrink-0 truncate text-[11px] text-muted-foreground">{statusLabel[status]}</span>
      </div>
      {live && remaining !== undefined && <span title="Time until the clone is wiped" className="flex h-8 shrink-0 items-center gap-1 rounded-lg border border-border px-2 text-[11px] tabular-nums text-muted-foreground"><Timer size={12} aria-hidden="true" />{remaining}m</span>}
      {onClose && <Button variant="ghost" size="icon-xs" onClick={onClose} aria-label="Close Clone Surface"><ChevronRight size={14} /></Button>}
    </header>

    {!snapshot ? <div className="grid flex-1 place-items-center text-muted-foreground"><LoaderCircle className="animate-spin" size={18} /></div>
      : status === "none" ? <div className="flex flex-1 items-center justify-center overflow-y-auto px-6 py-8">
          <div className="w-full max-w-sm">
            <div className="grid size-10 place-items-center rounded-xl border border-border bg-muted/50"><Globe size={18} className="text-foreground" aria-hidden="true" /></div>
            <h2 className="mt-4 font-display text-[15px] font-medium text-foreground">Open a browser for this chat</h2>
            <p className="mt-1.5 text-[12px] leading-5 text-muted-foreground">A throwaway copy of your browser for one site. The agent can see it and use it, and you can step in any time. It is wiped when the work is done.</p>
            <form className="mt-4 flex h-9 items-center gap-2 rounded-lg border border-input bg-background pl-3 pr-1 transition-shadow focus-within:border-ring focus-within:ring-[3px] focus-within:ring-ring/24" onSubmit={event => { event.preventDefault(); start(); }}>
              <Globe size={14} className="shrink-0 text-muted-foreground" aria-hidden="true" />
              <input aria-label="Site to clone" value={domainDraft} onChange={event => setDomainDraft(event.target.value)} placeholder="app.example.com" autoCapitalize="off" autoCorrect="off" spellCheck={false}
                className="h-full min-w-0 flex-1 bg-transparent font-mono text-[12.5px] text-foreground outline-none placeholder:font-sans placeholder:text-muted-foreground" />
              <Button type="submit" size="xs" disabled={busy || !domainDraft.trim()}>Start clone<ArrowRight size={12} /></Button>
            </form>
            {onCancelStart && <Button type="button" variant="ghost" size="xs" className="mt-2 text-muted-foreground" onClick={onCancelStart}>Back to browser</Button>}
            <div className="mt-3 flex items-center justify-between gap-3">
              <span className="text-[11px] text-muted-foreground">Opens</span>
              <SignInToggle value={startPath} onChange={setStartPath} disabled={busy} />
            </div>
          </div>
        </div>
      : status === "requested" ? <div className="flex flex-1 items-center justify-center overflow-y-auto p-5">
          <div className="w-full max-w-sm">
            {snapshot.pendingRequestId && <CloneConsentCard key={snapshot.pendingRequestId} request={{ sessionId: sessionId ?? "", requestId: snapshot.pendingRequestId, domain: snapshot.pendingRequest ?? snapshot.domain ?? "", extensionPath: snapshot.extensionPath, additionalDomains: snapshot.additionalDomains }} onError={onError} onResolved={() => void refresh().catch(error => onError(String(error)))} />}
          </div>
        </div>
      : status === "destroyed" ? <PaneState icon={Trash2} title="Clone destroyed">Its profile, cookies, and session are gone. Nothing from it stays on disk.</PaneState>
      : <>
        {status === "waiting_for_you" && <div role="status" className="flex items-center gap-2.5 border-b border-border bg-muted/40 px-3 py-2 text-[12px] leading-4 text-foreground">
          <Hand size={14} className="shrink-0 text-warning" aria-hidden="true" />
          <span className="min-w-0 flex-1"><b className="font-medium">The clone is waiting for you.</b> <span className="text-muted-foreground">{snapshot.waitingReason ?? "Take over to sign in, then hand it back."}</span></span>
        </div>}
        {status === "taken_over" && <div role="status" className="flex items-center gap-2.5 border-b border-border bg-muted/40 px-3 py-2 text-[12px] leading-4 text-foreground">
          <Hand size={14} className="shrink-0 text-info" aria-hidden="true" />
          <span className="min-w-0 flex-1"><b className="font-medium">You’re in control.</b> <span className="text-muted-foreground">Click the page and type; your keys go to the clone. The agent is paused.</span></span>
        </div>}

        <div className="min-h-0 flex-1 overflow-auto bg-muted/30 p-3">
          <LiveView snapshot={snapshot} takenOver={status === "taken_over"} agentLabel={agentLabel} onInput={send} />
        </div>

        <footer className="flex min-h-11 shrink-0 flex-wrap items-center gap-2 border-t border-border px-2.5 py-2">
          <span className="flex min-w-0 items-center gap-1.5 text-[11px] text-muted-foreground">
            {snapshot.agentVision === false ? <EyeOff size={12} aria-hidden="true" /> : <Eye size={12} aria-hidden="true" />}
            <span className="truncate">{snapshot.agentVision === false ? "Agent reads text only" : "Agent sees the page"}{snapshot.signInPath ? ` · ${signInLabel[snapshot.signInPath]}` : ""}</span>
          </span>
          {confirmingDestroy
            ? <div className="ml-auto flex flex-wrap items-center gap-1.5">
              <span className="text-[11px] text-muted-foreground">Wipe this clone?</span>
              <Button variant="ghost" size="xs" disabled={busy} onClick={() => setConfirmingDestroy(false)}>Cancel</Button>
              <Button variant="destructive" size="xs" disabled={busy} onClick={() => void run(() => bridgeApi.destroyBrowserClone(sessionId))}><Trash2 size={12} />Destroy clone</Button>
            </div>
            : <div className="ml-auto flex items-center gap-1.5">
              <Button variant="ghost" size="xs" disabled={busy} onClick={() => setConfirmingDestroy(true)} className="text-muted-foreground"><Trash2 size={12} />Destroy</Button>
              {status === "taken_over"
                ? <Button size="xs" disabled={busy} onClick={() => void run(() => bridgeApi.handBackBrowserClone(sessionId))}><Bot size={12} />Hand back</Button>
                : <Button variant={status === "waiting_for_you" ? "default" : "outline"} size="xs" disabled={busy || status === "starting"} onClick={() => void run(() => bridgeApi.takeoverBrowserClone(sessionId))}><Hand size={12} />Take over</Button>}
            </div>}
        </footer>
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

/** Keys the clone accepts one at a time; everything printable is typed. */
const FORWARDED_KEYS = new Set(["Enter", "Tab", "Backspace", "Delete", "Escape", "ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight", "Home", "End", "PageUp", "PageDown"]);
/** Typed characters are sent in bursts, so a password arrives as one value. */
const TYPE_FLUSH_MS = 350;
/** How long the agent's cursor stays on the page after its last action. */
const POINTER_VISIBLE_MS = 6000;

function LiveView({ snapshot, takenOver, agentLabel, onInput }: { snapshot: BrowserCloneSnapshot; takenOver: boolean; agentLabel: string; onInput: (input: CloneInputEvent) => void }) {
  const pending = useRef("");
  const timer = useRef<ReturnType<typeof setTimeout>>();
  const flush = () => {
    clearTimeout(timer.current);
    const text = pending.current;
    pending.current = "";
    if (text) onInput({ kind: "type", text });
  };
  useEffect(() => () => clearTimeout(timer.current), []);
  useEffect(() => { if (!takenOver) { clearTimeout(timer.current); pending.current = ""; } }, [takenOver]);

  // While the person holds the clone, a click on the frame is forwarded to the
  // page as a fraction of the viewport, so the scaled image maps onto the site.
  const fraction = (event: { clientX: number; clientY: number; currentTarget: Element }) => {
    const rect = event.currentTarget.getBoundingClientRect();
    if (rect.width === 0 || rect.height === 0) return undefined;
    return { x: (event.clientX - rect.left) / rect.width, y: (event.clientY - rect.top) / rect.height };
  };
  const forwardClick = (event: ReactMouseEvent<HTMLImageElement>) => {
    if (!takenOver) return;
    flush();
    const point = fraction(event);
    if (point) onInput({ kind: "click", ...point });
  };
  const forwardWheel = (event: ReactWheelEvent<HTMLDivElement>) => {
    if (!takenOver) return;
    const point = fraction(event);
    if (point) onInput({ kind: "scroll", ...point, deltaY: event.deltaY });
  };
  const forwardKey = (event: ReactKeyboardEvent<HTMLDivElement>) => {
    if (!takenOver || event.metaKey || event.ctrlKey) return;
    if (FORWARDED_KEYS.has(event.key)) {
      event.preventDefault();
      flush();
      onInput({ kind: "key", key: event.key });
    } else if (event.key.length === 1) {
      event.preventDefault();
      pending.current += event.key;
      clearTimeout(timer.current);
      timer.current = setTimeout(flush, TYPE_FLUSH_MS);
    }
  };

  if (!snapshot.screenshot) return <div className="grid min-h-full place-items-center p-6 text-center"><div><LoaderCircle size={18} className="mx-auto animate-spin text-muted-foreground" /><p className="mt-2 max-w-sm text-[11px] leading-5 text-muted-foreground">{snapshot.status === "starting" ? "Starting the clone…" : "Waiting for the first frame…"}</p></div></div>;
  const pointer = snapshot.agentPointer;
  // The cursor lingers where the agent last acted, then fades, so a quiet page
  // does not look like it is still being driven.
  const pointerShown = !!pointer && !takenOver && Date.now() - pointer.at < POINTER_VISIBLE_MS;
  return <div data-clone-viewport tabIndex={takenOver ? 0 : undefined} aria-label={takenOver ? "Clone page. Click and type to control it" : undefined}
    onKeyDown={forwardKey} onWheel={forwardWheel} onPaste={event => { if (!takenOver) return; event.preventDefault(); pending.current += event.clipboardData.getData("text"); flush(); }}
    className={cn("overflow-hidden rounded-lg border border-border bg-background shadow-sm outline-none transition-shadow",
      takenOver && "ring-2 ring-info/60 focus-visible:ring-info")}>
    <div aria-hidden="true" className="flex h-8 items-center gap-2.5 border-b border-border bg-muted/40 px-3">
      <span className="flex gap-1.5"><i className="size-2 rounded-full bg-muted-foreground/30" /><i className="size-2 rounded-full bg-muted-foreground/30" /><i className="size-2 rounded-full bg-muted-foreground/30" /></span>
      <span className="min-w-0 flex-1 truncate rounded-md bg-background/70 px-2 py-0.5 font-mono text-[11px] text-muted-foreground">{snapshot.domain}</span>
      <span className="shrink-0 text-[11px] text-muted-foreground">throwaway clone</span>
    </div>
    <div className="relative">
      <img src={snapshot.screenshot} alt="Live view of the browser clone" draggable={false} onClick={forwardClick} onMouseDown={event => { if (takenOver) event.currentTarget.parentElement?.parentElement?.focus(); }}
        className={cn("block h-auto w-full select-none", takenOver && "cursor-pointer")} />
      {pointer && <div aria-hidden="true" data-agent-pointer data-shown={pointerShown} style={{ left: `${pointer.x * 100}%`, top: `${pointer.y * 100}%` }}
        className={cn("pointer-events-none absolute z-10 transition-[left,top,opacity] duration-500 ease-out", pointerShown ? "opacity-100" : "opacity-0")}>
        <MousePointer2 size={16} className="fill-foreground text-background drop-shadow-sm" />
        <span className="ml-3.5 -mt-0.5 block w-fit rounded-md bg-foreground px-1.5 py-0.5 text-[11px] font-medium leading-4 text-background shadow-sm">{agentLabel}</span>
      </div>}
    </div>
  </div>;
}
