import { useEffect, useRef, useState, type ReactNode } from "react";
import { Globe, Package, Waypoints } from "lucide-react";
import { bridgeApi } from "../api";
import { startSerialPoll } from "../polling";
import type { CloneRequest } from "../protocol/generated/protocol";
import type { CloneSettings, CloneSignInPath } from "../types";
import { Select, Switch } from "./settings/kit";
import { Button } from "./ui/button";

const LIFETIMES = [10, 15, 30, 60, 120, 240];
const lifetimeLabel = (minutes: number) => minutes < 60 ? `${minutes} min` : minutes === 60 ? "1 hour" : `${minutes / 60} hours`;

/** Two-way choice between an imported sign-in and a blank browser. */
export function SignInToggle({ value, onChange, disabled }: { value: CloneSignInPath; onChange: (next: CloneSignInPath) => void; disabled?: boolean }) {
  return <div className="u-segmented w-fit" role="radiogroup" aria-label="Sign-in">
    {([["import", "Signed in as you"], ["sign_in_inside", "Blank"]] as const).map(([path, label]) =>
      <button key={path} type="button" role="radio" aria-checked={value === path} data-active={value === path} disabled={disabled} onClick={() => onChange(path)} className="u-segmented-item">{label}</button>)}
  </div>;
}

function OptionRow({ label, hint, children }: { label: string; hint?: string; children: ReactNode }) {
  return <div className="flex items-center justify-between gap-3 px-3 py-2.5">
    <div className="min-w-0">
      <div className="text-[12px] text-foreground">{label}</div>
      {hint && <div className="mt-0.5 text-[11px] leading-4 text-muted-foreground">{hint}</div>}
    </div>
    {children}
  </div>;
}

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
    bridgeApi.readCloneSettings().then(next => { if (active) setSettings({ ...next.settings, agentVision: next.settings.agentVision ?? true }); })
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
  const blank = settings?.defaultSignInPath === "sign_in_inside";
  return <section aria-label={`Browser request for ${request.domain}`} className="u-glass-popover w-full rounded-xl border border-border p-4 text-left">
    <div className="flex items-center gap-3">
      <div className="grid size-9 shrink-0 place-items-center rounded-lg border border-border bg-muted/60"><Globe size={16} className="text-foreground" aria-hidden="true" /></div>
      <div className="min-w-0">
        <p className="truncate text-[11px] text-muted-foreground">{label ?? "The agent wants a browser"}</p>
        <p className="truncate font-mono text-[13px] font-medium text-foreground">{request.domain}</p>
      </div>
    </div>
    <p className="mt-3 text-[12px] leading-5 text-muted-foreground">
      The agent opens this site in a private Chrome{blank ? "" : ", signed in as you"}. It can see the page, click, and type. The browser is wiped when the turn ends.
    </p>
    {!!request.additionalDomains?.length && <p className="mt-2 flex items-start gap-1.5 text-[11px] leading-4 text-muted-foreground"><Waypoints size={12} className="mt-0.5 shrink-0" aria-hidden="true" /><span className="min-w-0 break-all">Also connects to <span className="font-mono text-foreground">{request.additionalDomains.join(", ")}</span></span></p>}
    {request.extensionPath && <p className="mt-2 flex items-start gap-1.5 text-[11px] leading-4 text-muted-foreground"><Package size={12} className="mt-0.5 shrink-0" aria-hidden="true" /><span className="min-w-0 break-all">Loads extension <span className="font-mono text-foreground">{request.extensionPath}</span></span></p>}
    {settings && <div className="mt-3 divide-y divide-border rounded-lg border border-border bg-background/60">
      <OptionRow label="Sign-in"><SignInToggle value={settings.defaultSignInPath} disabled={busy} onChange={path => setSettings({ ...settings, defaultSignInPath: path })} /></OptionRow>
      <OptionRow label="Agent sees screenshots" hint={settings.agentVision ? undefined : "It reads page text only"}>
        <Switch label="Agent sees screenshots" checked={settings.agentVision ?? true} disabled={busy} onChange={agentVision => setSettings({ ...settings, agentVision })} />
      </OptionRow>
      <OptionRow label="Closes after">
        <Select label="Browser request lifetime" width="w-28" value={String(settings.ttlMinutes)} disabled={busy}
          options={[...new Set([...LIFETIMES, settings.ttlMinutes])].sort((a, b) => a - b).map(minutes => ({ value: String(minutes), label: lifetimeLabel(minutes) }))}
          onChange={value => setSettings({ ...settings, ttlMinutes: Number(value) })} />
      </OptionRow>
    </div>}
    <div className="mt-4 flex items-center gap-2">
      {onOpen && <Button variant="ghost" size="sm" onClick={onOpen} className="text-muted-foreground">Open chat</Button>}
      <div className="ml-auto flex gap-2">
        <Button variant="outline" size="sm" disabled={busy || !settings} onClick={() => void resolve(false)}>Deny</Button>
        <Button size="sm" disabled={busy || !settings} onClick={() => void resolve(true)}>{busy ? "Starting…" : "Allow"}</Button>
      </div>
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
