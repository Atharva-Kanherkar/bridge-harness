import { useContext, useEffect, useRef, useState } from "react";
import { AppWindow, Pencil, ShieldCheck } from "lucide-react";
import type { CallToolResult } from "@modelcontextprotocol/client";
import { bridgeApi } from "../api";
import type { PersonalAppOpen } from "../protocol/generated/protocol";
import { McpAppFrame, useNearViewport } from "../mcp-apps/McpAppFrame";
import { ConnectorApprovalSheet } from "./ConnectorApprovalSheet";
import { useDarkTheme } from "./Markdown";
import { connectionLabel, connectionNote, LIVE_READS_UNAVAILABLE } from "./personalAppShared";
import { VisualActionsContext } from "./VisualCall";
import { Button } from "./ui/button";

/** A write waiting in the sheet, and the view's call that waits on it. */
type Pending = {
  token: string;
  tool: string;
  server: string;
  harness: string;
  argumentsJson: string;
  arguments: Record<string, unknown>;
  resolve: (result: CallToolResult) => void;
  reject: (error: Error) => void;
};

const asToolResult = (value: unknown): CallToolResult => {
  if (!value || typeof value !== "object" || !("content" in value) || !Array.isArray((value as { content: unknown }).content)) {
    throw new Error("Invalid MCP result");
  }
  return value as CallToolResult;
};

/**
 * A locked personal app, opened in a chat or (with no `sessionId`) by the
 * person in its own pane. Reads answer at once; a write returns the exact
 * canonical JSON it would send, shown in the sheet, and runs only when the
 * person sends it. One write waits at a time.
 */
export function PersonalAppOpenCard({ sessionId, openId, contentHash, readOnly }: { sessionId?: string; openId: string; contentHash: string; readOnly: boolean }) {
  const [app, setApp] = useState<PersonalAppOpen>();
  const [error, setError] = useState("");
  const [pending, setPending] = useState<Pending>();
  const [busy, setBusy] = useState(false);
  const pendingRef = useRef<Pending>();
  const actions = useContext(VisualActionsContext);
  const dark = useDarkTheme();
  const [ref, near] = useNearViewport<HTMLDivElement>();
  const [height, setHeight] = useState(320);
  const [isolated, setIsolated] = useState<boolean>();
  useEffect(() => {
    let current = true;
    void bridgeApi.isolateAppPreviews().then(value => value === true, () => false).then(value => { if (current) setIsolated(value); });
    return () => { current = false; };
  }, []);
  useEffect(() => {
    let current = true;
    void bridgeApi.getPersonalAppOpen({ sessionId, openId, contentHash }).then(value => { if (current) setApp(value); }, reason => { if (current) setError(String(reason)); });
    return () => { current = false; };
  }, [sessionId, openId, contentHash]);
  // A card that goes away settles the view's waiting call instead of leaving it hanging.
  useEffect(() => () => { pendingRef.current?.reject(new Error("The app was closed before the action was sent")); }, []);

  const settle = (next: Pending | undefined) => { pendingRef.current = next; setPending(next); };

  if (!app) return <div ref={ref} role="status" className="rounded-2xl border border-dashed border-border px-4 py-3 text-xs text-muted-foreground">{error || "Loading app…"}</div>;

  const callTool = async (tool: string, args: Record<string, unknown>): Promise<CallToolResult> => {
    if (readOnly) throw new Error("Live tools are unavailable in history");
    if (app.reads.includes(tool) && isolated !== true) throw new Error(LIVE_READS_UNAVAILABLE);
    if (app.writes.includes(tool) && pendingRef.current) throw new Error("Another action is waiting for approval");
    const answer = await bridgeApi.callPersonalAppTool({ sessionId, openId, contentHash, tool, arguments: args });
    if (answer.kind === "result") return asToolResult(answer.result);
    return new Promise<CallToolResult>((resolve, reject) => settle({ ...answer, arguments: args, resolve, reject }));
  };

  const approve = async () => {
    const waiting = pendingRef.current;
    if (!waiting || busy) return;
    setBusy(true);
    try {
      const answer = await bridgeApi.callPersonalAppTool({ sessionId, openId, contentHash, tool: waiting.tool, arguments: waiting.arguments, approvalToken: waiting.token });
      if (answer.kind !== "result") throw new Error("The action needs approving again");
      waiting.resolve(asToolResult(answer.result));
    } catch (reason) {
      waiting.reject(reason instanceof Error ? reason : new Error(String(reason)));
    } finally {
      setBusy(false);
      settle(undefined);
    }
  };
  const deny = () => {
    pendingRef.current?.reject(new Error("The person declined this action"));
    settle(undefined);
  };

  return <div ref={ref} data-personal-app-state="open" className="min-w-0 overflow-hidden rounded-2xl border border-border bg-card text-foreground">
    <div className="flex items-center gap-2.5 border-b border-border px-4 py-3">
      <AppWindow size={16} className="shrink-0 text-muted-foreground" aria-hidden="true" />
      <div className="min-w-0 flex-1"><div className="truncate text-sm font-medium">{app.name}</div><div className="truncate text-xs text-muted-foreground">{app.server} · via {connectionLabel(app.harness)}</div></div>
      <span className="rounded-full border border-border px-2 py-0.5 text-xs text-muted-foreground">v{app.version}</span>
    </div>
    <div className="space-y-2 border-b border-border px-4 py-3 text-xs text-muted-foreground">
      <p>{app.intent}</p>
      <p className="flex items-center gap-1.5 text-foreground"><ShieldCheck size={12} aria-hidden="true" />Writes ask first. You see the exact call before anything is sent.</p>
      {connectionNote(app.harness) && <p>{connectionNote(app.harness)}</p>}
      {readOnly && <p>History. Live tools are disabled.</p>}
      {!readOnly && isolated === false && <p>{LIVE_READS_UNAVAILABLE}</p>}
      <details><summary className="cursor-pointer">Access · {app.reads.length} {app.reads.length === 1 ? "read" : "reads"} · {app.writes.length} {app.writes.length === 1 ? "write" : "writes"}</summary>
        <div className="mt-2 space-y-1 break-words"><p>Reads: {app.reads.join(", ") || "None"}</p><p>Writes: {app.writes.join(", ") || "None"}</p><p className="break-all font-mono text-[10px]">{app.contentHash}</p></div>
      </details>
    </div>
    {near && isolated !== undefined ? <McpAppFrame title={app.name} input={(app.prefill ?? {}) as Record<string, unknown>} theme={dark ? "dark" : "light"}
      personalView={{ html: app.viewHtml, resourceUri: `ui://bridge/personal/${app.appId}@${app.contentHash}` }}
      onToolCall={callTool} initialHeight={height} onHeight={setHeight} /> : <div aria-hidden="true" style={{ height }} />}
    {pending && <ConnectorApprovalSheet inFlow busy={busy} onApprove={() => void approve()} onDeny={deny}
      effect={`${pending.tool} on ${pending.server} via ${connectionLabel(pending.harness)}. Nothing is sent until you send it.`}
      detail={pending.argumentsJson} />}
    {sessionId && <div className="flex items-center gap-2 border-t border-border px-4 py-3">
      <Button size="sm" variant="ghost" disabled={readOnly || !actions?.ask} onClick={() => actions?.ask?.(`Revise the personal app “${app.name}” (app_id: ${app.appId}), starting from version ${app.version}. Keep its current server and tool access unless I ask to change them. `)}><Pencil aria-hidden="true" />Revise</Button>
    </div>}
  </div>;
}
