import { useEffect, useRef, useState } from "react";
import { bridgeApi } from "../api";
import { startSerialPoll } from "../polling";
import type { CloneRequest } from "../protocol/generated/protocol";
import type { CloneSettings } from "../types";
import { Button } from "./ui/button";

export function CloneConsentCard({ request, label, onOpen, onResolved, onError }: {
  request: CloneRequest;
  label?: string;
  onOpen?: () => void;
  onResolved?: () => void;
  onError: (message: string) => void;
}) {
  const [settings, setSettings] = useState<CloneSettings>();
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    let active = true;
    bridgeApi.readCloneSettings().then(next => { if (active) setSettings(next.settings); })
      .catch(error => onError(String(error)));
    return () => { active = false; };
  }, [request.requestId]);
  const resolve = async (allow: boolean) => {
    if (!settings) return;
    setBusy(true);
    try {
      await bridgeApi.resolveCloneRequest(request.sessionId, allow, request.requestId, settings);
      onResolved?.();
      if (allow && settings.defaultSignInPath === "sign_in_inside") onOpen?.();
    } catch (error) { onError(error instanceof Error ? error.message : String(error)); }
    finally { setBusy(false); }
  };
  return <section aria-label={`Browser request for ${request.domain}`} className="u-glass-popover rounded-xl border border-border p-4 text-left">
    <p className="text-xs font-semibold text-foreground">{label ?? "The agent wants a browser"}</p>
    <p className="mt-1 break-all font-mono text-xs text-foreground">{request.domain}</p>
    <p className="mt-2 text-xs leading-5 text-muted-foreground">Allow this agent to read and interact with this site in a throwaway browser. It closes when the task finishes.</p>
    {request.extensionPath && <p className="mt-2 break-all text-xs text-muted-foreground">Load extension under test: <span className="font-mono">{request.extensionPath}</span></p>}
    {settings && <div className="mt-3 flex flex-wrap gap-2">
      <select aria-label="Browser request sign-in path" value={settings.defaultSignInPath} disabled={busy} className="min-w-0 rounded-md border border-border bg-background p-2 text-xs" onChange={event => setSettings({ ...settings, defaultSignInPath: event.target.value as CloneSettings["defaultSignInPath"] })}>
        <option value="sign_in_inside">Sign in inside the clone</option>
        <option value="import">Import this site's sign-in</option>
      </select>
      <select aria-label="Browser request lifetime" value={settings.ttlMinutes} disabled={busy} className="rounded-md border border-border bg-background p-2 text-xs" onChange={event => setSettings({ ...settings, ttlMinutes: Number(event.target.value) })}>
        {[...new Set([10, 15, 30, 60, 120, 240, settings.ttlMinutes])].sort((a, b) => a - b).map(minutes => <option key={minutes} value={minutes}>{minutes} min</option>)}
      </select>
    </div>}
    <div className="mt-3 flex justify-end gap-2">
      {onOpen && <Button variant="ghost" size="sm" onClick={onOpen}>Open chat</Button>}
      <Button variant="ghost" size="sm" disabled={busy || !settings} onClick={() => void resolve(false)}>Deny</Button>
      <Button size="sm" disabled={busy || !settings} onClick={() => void resolve(true)}>{busy ? "Starting…" : "Allow"}</Button>
    </div>
  </section>;
}

/** Lives outside the dock: requests must reach unvisited panes and other chats. */
export function CloneRequestInbox({ sessionLabels, onOpen, onError }: {
  sessionLabels: Record<string, string>;
  onOpen: (sessionId: string) => void;
  onError: (message: string) => void;
}) {
  const [requests, setRequests] = useState<CloneRequest[]>([]);
  const active = useRef(true);
  const refresh = async () => {
    const next = await bridgeApi.cloneRequests();
    if (active.current) setRequests(next);
  };
  useEffect(() => {
    active.current = true;
    const stop = startSerialPoll(refresh, 1000);
    return () => { active.current = false; stop(); };
  }, []);
  if (!requests.length) return null;
  return <aside aria-label="Browser approvals" aria-live="polite" className="fixed right-4 top-16 z-50 flex max-h-[75dvh] w-96 max-w-[calc(100vw-2rem)] flex-col gap-3 overflow-y-auto">
    {requests.map(request => <CloneConsentCard key={request.requestId} request={request} label={`${sessionLabels[request.sessionId] ?? "Chat"} · browser request`} onOpen={() => onOpen(request.sessionId)} onResolved={() => void refresh().catch(error => onError(String(error)))} onError={onError} />)}
  </aside>;
}
