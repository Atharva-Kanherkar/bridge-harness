import { useEffect, useMemo, useState } from "react";
import { AlertTriangle, Camera, ChevronRight, Chrome, ExternalLink, Eye, Hand, LoaderCircle, MousePointer2, Plug, RefreshCw, ScrollText, ShieldAlert, TerminalSquare, Unplug } from "lucide-react";
import { bridgeApi } from "../api";
import { startSerialPoll } from "../polling";
import type { BrowserBridgeSnapshot, BrowserElement } from "../types";
import { Badge } from "./ui/badge";
import { Button } from "./ui/button";

type Pane = "page" | "elements" | "timeline" | "debug" | "metrics";

const statusLabel: Record<string, string> = {
  not_attached: "Not attached", reading: "Reading", acting: "Acting", waiting_for_you: "Waiting for you", paused: "Paused",
};

export function BrowserSurface({ onClose, onError }: { onClose: () => void; onError: (message: string) => void }) {
  const [snapshot, setSnapshot] = useState<BrowserBridgeSnapshot>();
  const [pane, setPane] = useState<Pane>("page");
  const [busy, setBusy] = useState(false);
  const [selectedElement, setSelectedElement] = useState<BrowserElement>();
  const [text, setText] = useState("");
  const [remoteEndpoint, setRemoteEndpoint] = useState("");
  const [remoteTokenEnv, setRemoteTokenEnv] = useState("BRIDGE_REMOTE_BROWSER_TOKEN");
  const [remoteUrl, setRemoteUrl] = useState("https://example.com");

  const refresh = async () => setSnapshot(await bridgeApi.browserBridgeState());
  useEffect(() => startSerialPoll(refresh, 700), []);
  useEffect(() => {
    if (!snapshot?.remoteProvider) return;
    setRemoteEndpoint(snapshot.remoteProvider.endpoint); setRemoteTokenEnv(snapshot.remoteProvider.bearerTokenEnv);
  }, [snapshot?.remoteProvider?.endpoint, snapshot?.remoteProvider?.bearerTokenEnv]);
  const run = async (task: () => Promise<unknown>) => {
    setBusy(true);
    try { await task(); await refresh(); }
    catch (error) { onError(error instanceof Error ? error.message : String(error)); }
    finally { setBusy(false); }
  };
  const action = (kind: string, extras: Record<string, unknown> = {}) => run(async () => {
    if (["click", "click_at", "type", "scroll", "navigate"].includes(kind)) await bridgeApi.takeoverBrowser();
    return bridgeApi.browserAction({ kind, actor: "user", expectedDomain: snapshot?.lease?.domain, ...extras });
  });
  const status = snapshot?.status ?? "not_attached";
  const suspiciousCount = useMemo(() => Math.max(snapshot?.promptInjectionSignals.length ?? 0, snapshot?.elements.filter(element => element.promptInjectionSuspected).length ?? 0), [snapshot?.elements, snapshot?.promptInjectionSignals]);

  return <aside className="u-glass-soft relative flex min-w-[340px] flex-[0_0_48%] flex-col overflow-hidden border-l border-white/[0.07]">
    <header className="flex min-h-12 items-center gap-2 border-b border-white/[0.07] px-3">
      <Chrome size={15} className="text-neutral-300" aria-hidden="true" />
      <div className="min-w-0 flex-1">
        <div className="truncate text-[11px] font-medium text-neutral-200">{snapshot?.tabs.find(tab => tab.attached)?.title ?? "Browser Surface"}</div>
        <div className="truncate text-[9.5px] text-neutral-500">{snapshot?.lease?.domain ?? "Use your existing logged-in tab"}</div>
      </div>
      <Badge variant={status === "waiting_for_you" ? "warning" : status === "paused" ? "secondary" : status === "not_attached" ? "outline" : "success"} size="sm">{statusLabel[status] ?? status}</Badge>
      {snapshot?.lease && <Badge variant="outline" size="sm">{snapshot.lease.permission === "read_only" ? "Read only" : "Interact"}</Badge>}
      <Button variant="ghost" size="icon-xs" onClick={onClose} aria-label="Close Browser Surface"><ChevronRight size={14} /></Button>
    </header>

    {!snapshot ? <div className="grid flex-1 place-items-center text-neutral-500"><LoaderCircle className="animate-spin" size={18} /></div> : !snapshot.nativeHostInstalled ? <SetupCard snapshot={snapshot} busy={busy} onInstall={() => void run(bridgeApi.installBrowserNativeHost)} /> : !snapshot.transportConnected ? <ConnectCard snapshot={snapshot} /> : !snapshot.lease ? <TabPicker snapshot={snapshot} busy={busy} onRefresh={() => void action("list_tabs")} onAttach={tabId => void action("attach", { tabId })} /> : <>
      <div className="flex items-center gap-1 border-b border-white/[0.06] px-2 py-1.5">
        {(["page", "elements", "timeline", "debug", "metrics"] as Pane[]).map(value => <button key={value} type="button" onClick={() => setPane(value)} className={`rounded-md px-2 py-1 text-[9.5px] capitalize ${pane === value ? "bg-white/[0.09] text-neutral-100" : "text-neutral-500 hover:text-neutral-300"}`}>{value}</button>)}
        <div className="ml-auto flex items-center gap-1">
          <Button variant="ghost" size="icon-xs" disabled={busy} onClick={() => void action("snapshot")} aria-label="Refresh semantic page"><RefreshCw size={12} /></Button>
          <Button variant="ghost" size="icon-xs" disabled={busy} onClick={() => void action("screenshot")} aria-label="Capture redacted screenshot"><Camera size={12} /></Button>
        </div>
      </div>

      {snapshot.promptInjectionSuspected && <div className="flex items-start gap-2 border-b border-amber-300/15 bg-amber-300/[0.05] px-3 py-2 text-[10px] leading-4 text-amber-200"><ShieldAlert size={14} className="mt-0.5 shrink-0" /><span>Untrusted page content matches {suspiciousCount} prompt-injection pattern{suspiciousCount === 1 ? "" : "s"}. Agent input is paused; page text is never treated as policy.</span></div>}
      {snapshot.captureError && <div className="flex items-start gap-2 border-b border-sky-300/15 bg-sky-300/[0.05] px-3 py-2 text-[10px] leading-4 text-sky-100"><Camera size={14} className="mt-0.5 shrink-0" /><span><b className="font-semibold">One Chrome click is required for the live mirror.</b> Activate the attached tab, click the Bridge extension icon, then choose <b className="font-semibold">Attach this tab</b>. Chrome only grants tab capture after that click.</span></div>}
      {snapshot.screenshotRedactedRegions > 0 && <div className="border-b border-white/[0.05] px-3 py-1 text-[9px] text-neutral-500">{snapshot.screenshotRedactedRegions} sensitive region{snapshot.screenshotRedactedRegions === 1 ? "" : "s"} redacted before persistence</div>}

      <div className="min-h-0 flex-1 overflow-auto">
        {pane === "page" && <PageMirror snapshot={snapshot} busy={busy} onPoint={(x, y) => void action("click_at", { x, y })} />}
        {pane === "elements" && <ElementsPane snapshot={snapshot} selected={selectedElement} onSelect={setSelectedElement} text={text} onText={setText} onActivate={element => void action("click", { elementId: element.id, sensitiveKind: element.sensitiveKind ?? undefined })} onType={element => void action("type", { elementId: element.id, text })} />}
        {pane === "timeline" && <TimelinePane snapshot={snapshot} />}
        {pane === "debug" && <DebugPane snapshot={snapshot} onEnable={() => void action("debugger")} />}
        {pane === "metrics" && <MetricsPane snapshot={snapshot} endpoint={remoteEndpoint} tokenEnv={remoteTokenEnv} remoteUrl={remoteUrl} onEndpoint={setRemoteEndpoint} onTokenEnv={setRemoteTokenEnv} onRemoteUrl={setRemoteUrl} onConfigure={() => void run(() => bridgeApi.configureRemoteBrowser(remoteEndpoint ? { endpoint: remoteEndpoint, bearerTokenEnv: remoteTokenEnv, enabled: true } : null))} onStart={() => void run(() => bridgeApi.startRemoteBrowser(remoteUrl))} />}
      </div>

      <div className="flex flex-wrap items-center gap-1.5 border-t border-white/[0.07] p-2">
        <Button variant="secondary" size="xs" onClick={() => void run(() => bridgeApi.setBrowserPermission(snapshot.lease?.permission === "interact" ? "read_only" : "interact"))}>{snapshot.lease?.permission === "interact" ? <Eye size={12} /> : <MousePointer2 size={12} />}{snapshot.lease?.permission === "interact" ? "Read only" : "Allow interact"}</Button>
        <Button variant="ghost" size="xs" onClick={() => void run(bridgeApi.takeoverBrowser)}><Hand size={12} />Take over</Button>
        <Button variant="ghost" size="xs" onClick={() => void action("focus")}><ExternalLink size={12} />Open tab</Button>
        <Button variant="ghost" size="xs" onClick={() => void run(bridgeApi.detachBrowser)} className="ml-auto text-neutral-500"><Unplug size={12} />Detach</Button>
      </div>
    </>}

    {snapshot?.pendingApproval && <div className="absolute inset-0 z-20 grid place-items-center bg-black/55 p-5 backdrop-blur-sm">
      <div className="u-glass-popover max-w-sm rounded-2xl p-4">
        <div className="flex items-center gap-2 text-sm font-semibold text-white"><AlertTriangle size={16} className="text-amber-300" />Sensitive action pending</div>
        <p className="mt-2 text-[11px] leading-5 text-neutral-300">{snapshot.pendingApproval.effect}</p>
        <p className="mt-1 font-mono text-[9px] text-neutral-600">{snapshot.pendingApproval.domain}</p>
        <div className="mt-4 flex justify-end gap-2">
          <Button variant="ghost" size="sm" onClick={() => void run(() => bridgeApi.resolveBrowserApproval(snapshot.pendingApproval!.id, false))}>Deny</Button>
          <Button size="sm" onClick={() => void run(() => bridgeApi.resolveBrowserApproval(snapshot.pendingApproval!.id, true))}>Approve once</Button>
        </div>
      </div>
    </div>}
  </aside>;
}

function SetupCard({ snapshot, busy, onInstall }: { snapshot: BrowserBridgeSnapshot; busy: boolean; onInstall: () => void }) {
  return <div className="grid flex-1 place-items-center p-6"><div className="max-w-sm text-center"><Plug size={24} className="mx-auto text-neutral-400" /><h2 className="mt-3 font-display text-base font-semibold text-white">Connect your browser once</h2><p className="mt-2 text-[11px] leading-5 text-neutral-500">Register Bridge’s local native host, then load the Chrome extension. The grant is tab-specific; cookies and profile files never leave Chrome.</p><Button className="mt-4" size="sm" loading={busy} onClick={onInstall}>Register native host</Button><div className="mt-4 rounded-lg bg-black/20 p-2 text-left font-mono text-[9px] leading-4 text-neutral-600"><div>Extension ID: {snapshot.extensionId}</div><div className="break-all">Load unpacked: {snapshot.extensionPath}</div></div></div></div>;
}

function ConnectCard({ snapshot }: { snapshot: BrowserBridgeSnapshot }) {
  return <div className="grid flex-1 place-items-center p-6 text-center"><div><Chrome size={24} className="mx-auto text-neutral-400" /><h2 className="mt-3 font-display text-base font-semibold text-white">Open the Bridge extension</h2><p className="mt-2 max-w-sm text-[11px] leading-5 text-neutral-500">In Chrome, open <span className="font-mono text-neutral-400">chrome://extensions</span>, enable Developer mode, and load the unpacked extension shown below. Bridge reconnects automatically.</p><div className="mt-3 rounded-lg bg-black/20 p-2 font-mono text-[9px] text-neutral-600">{snapshot.extensionPath}</div></div></div>;
}

function TabPicker({ snapshot, busy, onRefresh, onAttach }: { snapshot: BrowserBridgeSnapshot; busy: boolean; onRefresh: () => void; onAttach: (id: number) => void }) {
  return <div className="flex flex-1 flex-col p-3"><div className="flex items-center"><div><h2 className="font-display text-sm font-semibold text-white">Attach one Chrome tab</h2><p className="mt-1 text-[10px] text-neutral-500">Read-only is the default. Domain changes invalidate the lease.</p></div><Button variant="ghost" size="xs" className="ml-auto" loading={busy} onClick={onRefresh}><RefreshCw size={12} />Find tabs</Button></div><div className="mt-3 space-y-1.5">{snapshot.tabs.map(tab => <button type="button" key={tab.id} onClick={() => onAttach(tab.id)} className="u-glass-soft flex w-full items-center gap-2 rounded-xl p-2.5 text-left hover:bg-white/[0.06]"><span className="grid size-7 place-items-center rounded-lg bg-white/[0.05]"><Chrome size={13} /></span><span className="min-w-0 flex-1"><b className="block truncate text-[11px] font-medium text-neutral-200">{tab.title}</b><small className="block truncate text-[9px] text-neutral-600">{tab.domain}</small></span><ChevronRight size={13} className="text-neutral-600" /></button>)}{!snapshot.tabs.length && <div className="rounded-xl border border-dashed border-white/[0.08] p-6 text-center text-[10px] text-neutral-600">Find tabs to show user-approved HTTP(S) pages.</div>}</div></div>;
}

function PageMirror({ snapshot, busy, onPoint }: { snapshot: BrowserBridgeSnapshot; busy: boolean; onPoint: (x: number, y: number) => void }) {
  if (!snapshot.screenshot) return <div className="grid min-h-full place-items-center p-6 text-center"><div><Camera size={24} className="mx-auto text-neutral-600" /><p className="mt-2 max-w-sm text-[10px] leading-5 text-neutral-500">{snapshot.captureError ? "Open the attached Chrome tab, click the Bridge extension icon, and choose Attach this tab to grant the live mirror." : snapshot.captureActive ? "Waiting for the first redacted mirror frame…" : "Attaching the live mirror…"}</p></div></div>;
  return <button type="button" disabled={busy || snapshot.lease?.permission !== "interact"} className="relative block w-full cursor-crosshair disabled:cursor-default" onClick={event => {
    const rect = event.currentTarget.getBoundingClientRect();
    const width = snapshot.viewport?.width ?? rect.width; const height = snapshot.viewport?.height ?? rect.height;
    onPoint((event.clientX - rect.left) / rect.width * width, (event.clientY - rect.top) / rect.height * height);
  }}><img src={snapshot.screenshot} alt="Redacted live capture of the attached Chrome tab" className="block h-auto w-full" /></button>;
}

function ElementsPane({ snapshot, selected, onSelect, text, onText, onActivate, onType }: { snapshot: BrowserBridgeSnapshot; selected?: BrowserElement; onSelect: (value: BrowserElement) => void; text: string; onText: (value: string) => void; onActivate: (value: BrowserElement) => void; onType: (value: BrowserElement) => void }) {
  return <div className="p-2"><div className="mb-2 flex items-center gap-1.5 text-[9px] text-neutral-600"><Eye size={11} />Untrusted page content · {snapshot.elements.length} interactive elements</div><div className="space-y-1">{snapshot.elements.map(element => <button type="button" key={element.id} onClick={() => onSelect(element)} onDoubleClick={() => onActivate(element)} className={`flex w-full items-start gap-2 rounded-lg border px-2 py-1.5 text-left ${selected?.id === element.id ? "border-white/[0.15] bg-white/[0.08]" : "border-transparent hover:bg-white/[0.04]"}`}><span className="mt-0.5 font-mono text-[8px] text-neutral-600">{element.id}</span><span className="min-w-0 flex-1"><b className="block truncate text-[10px] font-medium text-neutral-300">{element.name || `[${element.role}]`}</b><small className="text-[8.5px] text-neutral-600">{element.role}{element.sensitiveKind ? ` · ${element.sensitiveKind} (manual only)` : ""}{element.promptInjectionSuspected ? " · injection warning" : ""}</small></span></button>)}</div>{selected?.role === "textbox" && !selected.sensitiveKind && <div className="sticky bottom-0 mt-2 flex gap-1.5 rounded-xl bg-neutral-950/90 p-2 backdrop-blur"><input value={text} onChange={event => onText(event.target.value)} placeholder="Type into selected field" className="min-w-0 flex-1 rounded-lg border border-white/[0.08] bg-white/[0.04] px-2 text-[10px] outline-none focus:border-white/[0.2]" /><Button size="xs" disabled={snapshot.lease?.permission !== "interact"} onClick={() => onType(selected)}>Type</Button></div>}</div>;
}

function TimelinePane({ snapshot }: { snapshot: BrowserBridgeSnapshot }) { return <div className="divide-y divide-white/[0.04]">{snapshot.audit.map(event => <div key={event.id} className="flex gap-2 px-3 py-2"><ScrollText size={11} className="mt-0.5 shrink-0 text-neutral-600" /><div className="min-w-0"><b className="block text-[9.5px] font-medium text-neutral-300">{event.summary}</b><small className="text-[8px] text-neutral-700">{event.kind} · {new Date(event.createdAt).toLocaleTimeString()}</small></div></div>)}</div>; }
function DebugPane({ snapshot, onEnable }: { snapshot: BrowserBridgeSnapshot; onEnable: () => void }) { return <div className="p-2"><Button variant="secondary" size="xs" onClick={onEnable}><TerminalSquare size={12} />Enable network & console inspection</Button><pre className="mt-2 whitespace-pre-wrap break-all rounded-xl bg-black/20 p-2 font-mono text-[8.5px] leading-4 text-neutral-500">{snapshot.debugEvents.length ? snapshot.debugEvents.map(event => JSON.stringify(event)).join("\n") : "No debugger events captured."}</pre></div>; }
function MetricsPane({ snapshot, endpoint, tokenEnv, remoteUrl, onEndpoint, onTokenEnv, onRemoteUrl, onConfigure, onStart }: { snapshot: BrowserBridgeSnapshot; endpoint: string; tokenEnv: string; remoteUrl: string; onEndpoint: (value: string) => void; onTokenEnv: (value: string) => void; onRemoteUrl: (value: string) => void; onConfigure: () => void; onStart: () => void }) { return <div className="p-2"><div className="grid grid-cols-3 gap-1.5">{[["DOM tokens", snapshot.tokenAccounting.estimatedInputTokens], ["Deltas", snapshot.tokenAccounting.deltaSnapshots], ["Screenshots", snapshot.tokenAccounting.screenshotCount]].map(([label, value]) => <div key={label} className="u-glass-soft rounded-xl p-2"><div className="text-[8px] uppercase tracking-wider text-neutral-600">{label}</div><div className="mt-1 font-mono text-sm text-neutral-300">{value}</div></div>)}</div><div className="mt-2 space-y-1">{snapshot.siteMetrics.map(metric => <div key={metric.domain} className="rounded-lg bg-white/[0.025] p-2 text-[9px] text-neutral-500"><b className="text-neutral-300">{metric.domain}</b><div className="mt-1 font-mono">{metric.successes}/{metric.actions} successful · {metric.actions ? Math.round(metric.totalLatencyMs / metric.actions) : 0}ms avg · {metric.inputTokens} tokens · {metric.interventions} takeovers · {metric.approvals} approvals · {metric.duplicateSideEffects} duplicate effects</div></div>)}</div><div className="mt-3 rounded-xl border border-white/[0.06] p-2"><div className="text-[9px] font-medium text-neutral-300">Optional remote browser</div><p className="mt-1 text-[8.5px] leading-4 text-neutral-600">For geo/proxy, unattended, or high-concurrency tasks. Bridge stores only the HTTPS endpoint and environment-variable name—not the token.</p><div className="mt-2 grid gap-1.5"><input value={endpoint} onChange={event => onEndpoint(event.target.value)} placeholder="https://browser.example.com" className="rounded-lg border border-white/[0.08] bg-white/[0.03] px-2 py-1.5 text-[9px] outline-none" /><input value={tokenEnv} onChange={event => onTokenEnv(event.target.value)} placeholder="BRIDGE_REMOTE_BROWSER_TOKEN" className="rounded-lg border border-white/[0.08] bg-white/[0.03] px-2 py-1.5 font-mono text-[9px] outline-none" /><Button variant="secondary" size="xs" onClick={onConfigure}>{snapshot.remoteProvider ? "Update provider" : "Save provider"}</Button>{snapshot.remoteProvider?.enabled && <div className="mt-1 flex gap-1"><input value={remoteUrl} onChange={event => onRemoteUrl(event.target.value)} className="min-w-0 flex-1 rounded-lg border border-white/[0.08] bg-white/[0.03] px-2 text-[9px] outline-none" /><Button size="xs" onClick={onStart}>Start remote</Button></div>}</div></div></div>; }
