import type { ConversationItem } from "./conversation";

/** The error row a failed turn ended on, and the user message it answers. */
export interface FailedTurn {
  errorKey: string;
  prompt: ConversationItem;
}

/**
 * The turn the chat can retry: the newest error, when nothing has answered or
 * followed it, and the user message that turn was answering.
 *
 * Tool rows, thinking and partial assistant text between the prompt and the
 * error are part of the failed turn. An assistant reply or a new user message
 * after the error means the chat has moved on, so there is nothing to retry.
 * A degraded history entry is a gap in storage, not a failed turn.
 */
export function failedTurn(items: readonly ConversationItem[]): FailedTurn | null {
  let errorKey: string | undefined;
  for (let index = items.length - 1; index >= 0; index -= 1) {
    const item = items[index];
    if (errorKey === undefined) {
      if (item.type === "error" && item.status !== "degraded") { errorKey = item.key; continue; }
      if (item.type === "message") return null;
      continue;
    }
    if (item.type === "message" && item.role === "user") return { errorKey, prompt: item };
  }
  return null;
}
