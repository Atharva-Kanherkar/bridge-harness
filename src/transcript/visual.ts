/**
 * Which transcript rows are calls to Bridge's own `visualize` tool, and the
 * spec each one carried.
 *
 * Keyed on the server and tool the call went through, never on the harness:
 * Claude names it `mcp__bridge__visualize` with the arguments on `input`,
 * Codex sends an `mcpToolCall` item with `server`, `tool` and `arguments`,
 * and OpenCode names it `bridge_visualize` with the arguments under
 * `state.input`. Another server's `visualize` is just a tool call.
 */

import type { ConversationItem } from "./item";

const CLAUDE_NAME = "mcp__bridge__visualize";
const OPENCODE_NAME = "bridge_visualize";

type Data = Record<string, unknown>;

const objectValue = (value: unknown): Data | undefined =>
  typeof value === "object" && value !== null && !Array.isArray(value) ? (value as Data) : undefined;

function parsedArguments(value: unknown): Data | undefined {
  if (typeof value === "string") {
    try {
      return objectValue(JSON.parse(value));
    } catch {
      return undefined;
    }
  }
  return objectValue(value);
}

/** Whether a tool row's data names Bridge's `visualize` tool. */
export function isVisualCallData(data: Data): boolean {
  if (data.name === CLAUDE_NAME) return true;
  if (data.type === "mcpToolCall") return data.server === "bridge" && data.tool === "visualize";
  return data.tool === OPENCODE_NAME;
}

export function isVisualItem(item: ConversationItem): boolean {
  return (item.type === "activity" || item.type === "artifact") && isVisualCallData(item.data);
}

/**
 * The spec a visual call carried, or `undefined` while it is still being
 * written (Claude streams the call's start before its arguments) or when the
 * row is not a visual call at all.
 */
export function visualCallInput(item: ConversationItem): Data | undefined {
  if (!isVisualItem(item)) return undefined;
  const data = item.data;
  const input = parsedArguments(data.input)
    ?? parsedArguments(data.arguments)
    ?? parsedArguments(objectValue(data.state)?.input);
  return input && Object.keys(input).length > 0 ? input : undefined;
}

/** The refusal text the server returned, for a failed call. */
export function visualCallRefusal(item: ConversationItem): string | undefined {
  const data = item.data;
  const state = objectValue(data.state);
  const result = objectValue(data.result);
  const fromContent = (content: unknown) => Array.isArray(content)
    ? content.map(part => objectValue(part)?.text).filter((text): text is string => typeof text === "string").join("\n")
    : typeof content === "string" ? content : undefined;
  const candidates = [
    typeof data.aggregatedOutput === "string" ? data.aggregatedOutput : undefined,
    fromContent(data.content),
    fromContent(result?.content),
    typeof state?.output === "string" ? state.output : undefined,
    typeof state?.error === "string" ? state.error : undefined,
    typeof objectValue(data.error)?.message === "string" ? (objectValue(data.error)!.message as string) : undefined,
    item.text || undefined,
  ];
  return candidates.find(text => text && text.trim().length > 0)?.trim();
}

/** Every refusal the server sends starts with this, on every harness. */
export const REFUSAL_PREFIX = "The visual was not drawn.";

export type VisualCallState = "drawing" | "drawn" | "refused";

/**
 * Where a visual call is. A harness may report a refused call as completed
 * (the MCP result carries `isError`, not the item), so the refusal text the
 * server always sends is the signal of last resort.
 */
export function visualCallState(item: ConversationItem): VisualCallState {
  const data = item.data;
  const state = objectValue(data.state);
  const result = objectValue(data.result);
  const flagged = item.status === "failed"
    || data.is_error === true
    || result?.isError === true
    || state?.status === "error"
    || (objectValue(data.error) !== undefined)
    || (visualCallRefusal(item)?.startsWith(REFUSAL_PREFIX) ?? false);
  if (flagged) return "refused";
  const settled = item.status === "completed" || state?.status === "completed";
  return settled ? "drawn" : "drawing";
}
