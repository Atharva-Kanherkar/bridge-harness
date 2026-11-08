/**
 * Which transcript rows are calls to Bridge's own `outgoing_draft` tool, and
 * the draft each one made.
 *
 * Keyed on the server and tool the call went through, never on the harness:
 * Claude names it `mcp__bridge__outgoing_draft` and returns the result as a
 * JSON string in `aggregatedOutput`, Codex sends an `mcpToolCall` item with
 * `server`, `tool` and a `result`, and OpenCode names it
 * `bridge_outgoing_draft` with the output under `state.output`. Another
 * server's `outgoing_draft` is just a tool call.
 */

import type { ConversationItem } from "./item";

const CLAUDE_NAME = "mcp__bridge__outgoing_draft";
const OPENCODE_NAME = "bridge_outgoing_draft";

type Data = Record<string, unknown>;

const objectValue = (value: unknown): Data | undefined =>
  typeof value === "object" && value !== null && !Array.isArray(value) ? (value as Data) : undefined;

/** Whether a tool row's data names Bridge's `outgoing_draft` tool. */
export function isOutgoingCallData(data: Data): boolean {
  if (data.name === CLAUDE_NAME) return true;
  if (data.type === "mcpToolCall") return data.server === "bridge" && data.tool === "outgoing_draft";
  return data.tool === OPENCODE_NAME;
}

export function isOutgoingItem(item: ConversationItem): boolean {
  return (item.type === "activity" || item.type === "artifact") && isOutgoingCallData(item.data);
}

function texts(content: unknown): string[] {
  if (typeof content === "string") return [content];
  if (!Array.isArray(content)) return [];
  return content.map(part => objectValue(part)?.text).filter((text): text is string => typeof text === "string");
}

/** Every place a harness may have put the tool's answer, most structured first. */
function candidates(item: ConversationItem): unknown[] {
  const data = item.data;
  const result = objectValue(data.result);
  const state = objectValue(data.state);
  return [
    result?.structuredContent,
    data.structuredContent,
    ...texts(result?.content),
    typeof data.aggregatedOutput === "string" ? data.aggregatedOutput : undefined,
    ...texts(data.content),
    typeof state?.output === "string" ? state.output : undefined,
    item.text || undefined,
  ];
}

function parsed(value: unknown): Data | undefined {
  if (typeof value !== "string") return objectValue(value);
  try {
    return objectValue(JSON.parse(value));
  } catch {
    return undefined;
  }
}

/** The id of the draft a call made, once the call has answered. */
export function outgoingDraftId(item: ConversationItem): string | undefined {
  if (!isOutgoingItem(item)) return undefined;
  for (const candidate of candidates(item)) {
    const answer = parsed(candidate);
    if (typeof answer?.id === "string" && typeof answer.state === "string") return answer.id;
  }
  return undefined;
}

/** The refusal the core sent back, for a call that drafted nothing. */
export function outgoingRefusal(item: ConversationItem): string | undefined {
  const data = item.data;
  const state = objectValue(data.state);
  const found = [
    ...candidates(item).filter((candidate): candidate is string => typeof candidate === "string"),
    typeof state?.error === "string" ? state.error : undefined,
    typeof objectValue(data.error)?.message === "string" ? (objectValue(data.error)!.message as string) : undefined,
  ].find(text => text && text.trim().length > 0);
  return found?.trim();
}

export type OutgoingCallState = "drafting" | "drafted" | "refused";

/**
 * Where a draft call is. A refused call answers with plain text rather than
 * a draft, and some harnesses report it as completed, so "completed without
 * a draft id" counts as refused.
 */
export function outgoingCallState(item: ConversationItem): OutgoingCallState {
  if (outgoingDraftId(item)) return "drafted";
  const data = item.data;
  const state = objectValue(data.state);
  const failed = item.status === "failed"
    || data.is_error === true
    || objectValue(data.result)?.isError === true
    || state?.status === "error"
    || objectValue(data.error) !== undefined;
  const settled = item.status === "completed" || state?.status === "completed";
  return failed || settled ? "refused" : "drafting";
}
