import type { ConversationItem } from "./item";

const object = (value: unknown): Record<string, unknown> | undefined =>
  value !== null && typeof value === "object" && !Array.isArray(value) ? value as Record<string, unknown> : undefined;

export function isPersonalAppItem(item: ConversationItem): boolean {
  if (item.type !== "activity" && item.type !== "artifact") return false;
  const data = item.data;
  return data.name === "mcp__bridge__apps_draft"
    || (data.type === "mcpToolCall" && data.server === "bridge" && data.tool === "apps_draft")
    || data.tool === "bridge_apps_draft";
}

/** Read only the answer's identity. Streamed HTML and input never get rendered. */
export function personalAppReference(item: ConversationItem): { draftId: string; contentHash: string } | undefined {
  if (!isPersonalAppItem(item) || item.status === "failed" || item.data.is_error === true) return undefined;
  const result = object(item.data.result);
  if (result?.isError === true) return undefined;
  const texts = (value: unknown) => typeof value === "string" ? [value] : Array.isArray(value) ? value.map(part => object(part)?.text) : [];
  const candidates = [result?.structuredContent, item.data.structuredContent, ...texts(result?.content),
    item.data.aggregatedOutput, ...texts(item.data.content), object(item.data.state)?.output];
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
