import { useContext, useEffect, useState } from "react";
import { AppWindow, Check, LockKeyhole, Pencil } from "lucide-react";
import type { CallToolResult } from "@modelcontextprotocol/client";
import { bridgeApi } from "../api";
import type { ConversationItem } from "../conversation";
import type { LockPersonalAppResult, PersonalAppDraft } from "../protocol/generated/protocol";
import { personalAppReference } from "../transcript/personalApp";
import { McpAppFrame, useNearViewport } from "../mcp-apps/McpAppFrame";
import { useDarkTheme } from "./Markdown";
import { VisualActionsContext } from "./VisualCall";
import { Button } from "./ui/button";

/** The agent whose MCP connection the app uses, as the person knows it. */
const CONNECTION_LABELS: Record<string, string> = { claude: "Claude Code", codex: "Codex", opencode: "OpenCode", cursor: "Cursor" };
const connectionLabel = (harness: string) => CONNECTION_LABELS[harness] ?? harness;

const LIVE_READS_UNAVAILABLE = "Live reads run only in the Bridge Mac app, where a preview cannot reach the network. Writes still show what would be sent.";

function DraftCard({ sessionId, draftId, contentHash, readOnly }: { sessionId: string; draftId: string; contentHash: string; readOnly: boolean }) {
  const [draft, setDraft] = useState<PersonalAppDraft>();
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [wider, setWider] = useState<Extract<LockPersonalAppResult, { kind: "widens" }>>();
  const actions = useContext(VisualActionsContext);
  const dark = useDarkTheme();
  const [ref, near] = useNearViewport<HTMLDivElement>();
  const [height, setHeight] = useState(320);
  // Settled before the frame mounts: WebRTC must already be off when the view's
  // document is created, or it keeps the constructor it started with.
  const [isolated, setIsolated] = useState<boolean>();
  useEffect(() => {
    let current = true;
    void bridgeApi.isolateAppPreviews().then(value => value === true, () => false).then(value => { if (current) setIsolated(value); });
    return () => { current = false; };
  }, []);
  useEffect(() => {
    let current = true;
    void bridgeApi.getPersonalAppDraft({ sessionId, draftId, contentHash }).then(value => { if (current) setDraft(value); }, reason => { if (current) setError(String(reason)); });
    return () => { current = false; };
  }, [sessionId, draftId, contentHash]);

  const lock = async (allowWider: boolean) => {
    if (readOnly || busy || !draft || draft.locked || (allowWider && !wider)) return;
    setBusy(true); setError("");
    try {
      const result = await bridgeApi.lockPersonalApp({ draftId, contentHash, allowWider });
      if (result.kind === "widens") setWider(result);
      else { setDraft({ ...draft, locked: true }); setWider(undefined); }
    } catch (reason) { setError(String(reason)); }
    finally { setBusy(false); }
  };
  if (!draft) return <div ref={ref} role="status" className="rounded-2xl border border-dashed border-border px-4 py-3 text-xs text-muted-foreground">{error || "Loading app preview…"}</div>;
  const callTool = async (tool: string, args: Record<string, unknown>): Promise<CallToolResult> => {
    if (readOnly) throw new Error("Live tools are unavailable in history");
    // A write is a dry-run stub that echoes the view's own arguments. A read
    // returns real data, so it needs a view that cannot reach the network.
    if (!draft.writes.includes(tool) && isolated !== true) throw new Error(LIVE_READS_UNAVAILABLE);
    const result = await bridgeApi.previewPersonalAppTool({ sessionId, draftId, contentHash, tool, arguments: args });
    if (!result || typeof result !== "object" || !("content" in result) || !Array.isArray(result.content)) throw new Error("Invalid MCP preview result");
    return result as CallToolResult;
  };
  return <div ref={ref} data-personal-app-state={draft.locked ? "locked" : "preview"} className="min-w-0 overflow-hidden rounded-2xl border border-border bg-card text-foreground">
    <div className="flex items-center gap-2.5 border-b border-border px-4 py-3">
      <AppWindow size={16} className="shrink-0 text-muted-foreground" aria-hidden="true" />
      <div className="min-w-0 flex-1"><div className="truncate text-sm font-medium">{draft.name}</div><div className="truncate text-xs text-muted-foreground">{draft.server} · via {connectionLabel(draft.harness)}</div></div>
      <span className="rounded-full border border-border px-2 py-0.5 text-xs text-muted-foreground">{draft.locked ? "Locked" : "Preview"}</span>
    </div>
    <div className="space-y-2 border-b border-border px-4 py-3 text-xs text-muted-foreground">
      <p>{draft.intent}</p>
      <p className="text-foreground">Preview only. Write actions show what would be sent.</p>
      {readOnly && <p>History preview. Live tools and changes are disabled.</p>}
      {!readOnly && isolated === false && <p>{LIVE_READS_UNAVAILABLE}</p>}
      <details><summary className="cursor-pointer">Access · {draft.reads.length} {draft.reads.length === 1 ? "read" : "reads"} · {draft.writes.length} {draft.writes.length === 1 ? "write" : "writes"}</summary>
        <div className="mt-2 space-y-1 break-words"><p>Reads: {draft.reads.join(", ") || "None"}</p><p>Writes: {draft.writes.join(", ") || "None"}</p><p className="break-all font-mono text-[10px]">{contentHash}</p></div>
      </details>
    </div>
    {near && isolated !== undefined ? <McpAppFrame title={`${draft.name} preview`} input={(draft.sampleArgs ?? {}) as Record<string, unknown>} theme={dark ? "dark" : "light"}
      personalView={{ html: draft.viewHtml, resourceUri: `ui://bridge/personal/${draft.appId}@${contentHash}` }}
      onToolCall={callTool} initialHeight={height} onHeight={setHeight} /> : <div aria-hidden="true" style={{ height }} />}
    <div className="space-y-3 border-t border-border px-4 py-3">
      {error && <p role="alert" className="text-xs text-destructive">{error}</p>}
      {wider && <div role="group" aria-label="Confirm wider access" className="space-y-2 rounded-xl border border-border bg-muted/40 p-3 text-xs">
        <p className="font-medium">This version adds access</p>
        {wider.targetChanged && <p>Target changes to {draft.server} via {connectionLabel(draft.harness)}.</p>}
        {wider.addedReads.length > 0 && <p className="break-words">New reads: {wider.addedReads.join(", ")}</p>}
        {wider.addedWrites.length > 0 && <p className="break-words">New writes: {wider.addedWrites.join(", ")}</p>}
        <div className="flex gap-2"><Button size="xs" disabled={busy || readOnly} onClick={() => void lock(true)}>Allow access and lock</Button><Button size="xs" variant="ghost" disabled={busy} onClick={() => setWider(undefined)}>Cancel</Button></div>
      </div>}
      <div className="flex items-center gap-2">
        <Button size="sm" disabled={readOnly || busy || draft.locked || Boolean(wider)} onClick={() => void lock(false)}>
          {draft.locked ? <Check aria-hidden="true" /> : <LockKeyhole aria-hidden="true" />}{draft.locked ? "Version locked" : busy ? "Locking…" : "Lock version"}
        </Button>
        <Button size="sm" variant="ghost" disabled={readOnly || busy || !actions?.ask} onClick={() => actions?.ask?.(`Revise the personal app “${draft.name}” (app_id: ${draft.appId}), starting from the preview ${contentHash}. Keep its current server and tool access unless I ask to change them. `)}><Pencil aria-hidden="true" />Revise</Button>
      </div>
    </div>
  </div>;
}

export function PersonalAppDraftCall({ item, sessionId, readOnly = false }: { item: ConversationItem; sessionId?: string; readOnly?: boolean }) {
  const reference = personalAppReference(item);
  if (!sessionId) return <p className="py-2 text-xs text-muted-foreground">Open the original chat to view this app preview.</p>;
  if (!reference) return <div className="px-1.5 py-2 text-xs text-muted-foreground">{item.status === "inProgress" ? "Drafting an app…" : "App not drafted. The reason went back to the chat."}</div>;
  return <DraftCard key={`${sessionId}:${reference.draftId}:${reference.contentHash}`} sessionId={sessionId} {...reference} readOnly={readOnly} />;
}
