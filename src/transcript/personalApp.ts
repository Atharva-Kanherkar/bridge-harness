import type { ConversationItem } from "./item";

const object = (value: unknown): Record<string, unknown> | undefined =>
  value !== null && typeof value === "object" && !Array.isArray(value) ? value as Record<string, unknown> : undefined;

// ACP agents (Cursor) title an MCP call by server and tool, e.g. "bridge: apps_draft".
const ACP_TITLE = /^\s*(?:mcp__)?bridge(?:\s*[:/.]\s*|_{1,2})apps_(draft|open)\s*$/;

export type PersonalAppTool = "draft" | "open";
export type PersonalAppReference =
  | { kind: "draft"; draftId: string; contentHash: string }
  | { kind: "open"; openId: string; contentHash: string };

/** Which personal-app tool this call is, as each agent names it: Claude, Codex, OpenCode, Cursor. */
export function personalAppTool(item: ConversationItem): PersonalAppTool | undefined {
  if (item.type !== "activity" && item.type !== "artifact") return undefined;
  const data = item.data;
  for (const tool of ["draft", "open"] as const) {
    if (data.name === `mcp__bridge__apps_${tool}`
      || (data.type === "mcpToolCall" && data.server === "bridge" && data.tool === `apps_${tool}`)
      || data.tool === `bridge_apps_${tool}`) return tool;
  }
  const acp = typeof item.title === "string" && object(data.update) !== undefined ? ACP_TITLE.exec(item.title) : null;
  return acp ? acp[1] as PersonalAppTool : undefined;
}

export function isPersonalAppItem(item: ConversationItem): boolean {
  return personalAppTool(item) !== undefined;
}

/** Read only the answer's identity. Streamed HTML and input never get rendered. */
export function personalAppReference(item: ConversationItem): PersonalAppReference | undefined {
  const tool = personalAppTool(item);
  if (!tool || item.status === "failed" || item.data.is_error === true) return undefined;
  const result = object(item.data.result);
  if (result?.isError === true) return undefined;
  const texts = (value: unknown) => typeof value === "string" ? [value] : Array.isArray(value) ? value.map(part => object(part)?.text) : [];
  // ACP (Cursor): the start frame stays under `update`, and the completion's
  // fields are merged into the item's root. The answer is a rawOutput or an
  // ACP text content block; tool input (`rawInput`) is never read.
  const acpNested = (value: unknown) => Array.isArray(value) ? value.map(part => object(object(part)?.content)?.text) : [];
  const fromAcp = (frame: Record<string, unknown> | undefined) => frame ? [
    frame.rawOutput, object(frame.rawOutput)?.structuredContent, ...texts(object(frame.rawOutput)?.content), ...acpNested(frame.content),
  ] : [];
  const start = object(item.data.update);
  const candidates = [result?.structuredContent, item.data.structuredContent, ...texts(result?.content),
    item.data.aggregatedOutput, ...texts(item.data.content), object(item.data.state)?.output,
    ...fromAcp(item.data), ...fromAcp(object(start?.update) ?? start)];
  for (const value of candidates) {
    let parsed = value;
    if (typeof value === "string") { try { parsed = JSON.parse(value); } catch { continue; } }
    const answer = object(parsed);
    if (typeof answer?.contentHash !== "string" || !/^sha256:[a-f0-9]{64}$/.test(answer.contentHash)) continue;
    if (tool === "draft" && typeof answer.draftId === "string" && answer.draftId.length <= 100) {
      return { kind: "draft", draftId: answer.draftId, contentHash: answer.contentHash };
    }
    if (tool === "open" && typeof answer.openId === "string" && answer.openId.length <= 100) {
      return { kind: "open", openId: answer.openId, contentHash: answer.contentHash };
    }
  }
  return undefined;
}
