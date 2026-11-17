import type { ConversationItem } from "./item";

const object = (value: unknown): Record<string, unknown> | undefined =>
  value !== null && typeof value === "object" && !Array.isArray(value) ? value as Record<string, unknown> : undefined;

// ACP agents (Cursor) title an MCP call by server and tool, e.g. "bridge: apps_draft".
const ACP_TITLE = /^\s*(?:mcp__)?bridge(?:\s*[:/.]\s*|_{1,2})apps_draft\s*$/;

/** The draft tool's call, as each agent names it: Claude, Codex, OpenCode, Cursor. */
export function isPersonalAppItem(item: ConversationItem): boolean {
  if (item.type !== "activity" && item.type !== "artifact") return false;
  const data = item.data;
  return data.name === "mcp__bridge__apps_draft"
    || (data.type === "mcpToolCall" && data.server === "bridge" && data.tool === "apps_draft")
    || data.tool === "bridge_apps_draft"
    || (typeof item.title === "string" && ACP_TITLE.test(item.title) && object(data.update) !== undefined);
}

/** Read only the answer's identity. Streamed HTML and input never get rendered. */
export function personalAppReference(item: ConversationItem): { draftId: string; contentHash: string } | undefined {
  if (!isPersonalAppItem(item) || item.status === "failed" || item.data.is_error === true) return undefined;
  const result = object(item.data.result);
  if (result?.isError === true) return undefined;
  const texts = (value: unknown) => typeof value === "string" ? [value] : Array.isArray(value) ? value.map(part => object(part)?.text) : [];
  // ACP keeps the session/update frame: the answer is its rawOutput or text content.
  const acp = object(item.data.update);
  const acpUpdate = object(acp?.update) ?? acp;
  const acpTexts = Array.isArray(acpUpdate?.content) ? acpUpdate.content.map(part => object(object(part)?.content)?.text) : [];
  const candidates = [result?.structuredContent, item.data.structuredContent, ...texts(result?.content),
    item.data.aggregatedOutput, ...texts(item.data.content), object(item.data.state)?.output,
    acpUpdate?.rawOutput, object(acpUpdate?.rawOutput)?.structuredContent, ...texts(object(acpUpdate?.rawOutput)?.content), ...acpTexts];
  for (const value of candidates) {
    let parsed = value;
    if (typeof value === "string") { try { parsed = JSON.parse(value); } catch { continue; } }
    const answer = object(parsed);
    if (typeof answer?.draftId === "string" && answer.draftId.length <= 100
      && typeof answer.contentHash === "string" && /^sha256:[a-f0-9]{64}$/.test(answer.contentHash)) {
      return { draftId: answer.draftId, contentHash: answer.contentHash };
    }
  }
  return undefined;
}
