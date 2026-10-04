import type { AgentEvent } from "./types";
import { authModeFromText } from "./errors";
import { readWireKind } from "./transcript/wire";
import type { UsageProvider } from "./usage";
import { attachmentUris, type ConversationItem } from "./conversation";
import type { ComposerAttachment } from "./pasteAttachments";

/** Only a failed provider operation can start recovery; a generic 403 could
 * be a policy refusal and must never open an unrelated login flow. */
export function needsProviderSignIn(harness: string, message: string): UsageProvider | null {
  if (!["codex", "claude", "cursor", "opencode"].includes(harness)) return null;
  // A rejected API key is not an expired session. Opening the subscription
  // sign-in for it sends the user through a flow that cannot fix their key —
  // and, for someone who just replaced a subscription with a key on purpose,
  // undoes the thing they meant to do.
  if (authModeFromText(message) === "api-key") return null;
  const expired = /(?:token|credentials?|session).{0,20}expired|expired (?:token|credentials?)|not (?:logged|signed) in|authentication (?:failed|required)|please (?:log|sign) ?in|re-?authenticate|invalid (?:access|refresh) token|\b401\s+(?:unauthorized|authentication)|refresh token.{0,80}(?:already used|revoked)/i;
  return expired.test(message) ? harness as UsageProvider : null;
}

/** Live provider errors only. Transcript text mentioning login is not a request. */
export function providerSignInForEvent(harness: string, event: Pick<AgentEvent, "kind" | "text">): UsageProvider | null {
  return readWireKind(event.kind) === "error" ? needsProviderSignIn(harness, event.text ?? "") : null;
}

export type SignInRetry = { sessionId: string; payload?: { text: string; attachments: ComposerAttachment[] } };

/** The prompt a sign-in retry resends: the failed send's exact payload when
 * there is one, otherwise the transcript's last user turn with its images.
 * Only offered in the chat that hit the wall, so it never lands elsewhere. */
export function signInRetryPayload(retry: SignInRetry | null, sessionId: string | undefined, items: ConversationItem[]): SignInRetry["payload"] {
  if (!retry || retry.sessionId !== sessionId) return undefined;
  const payload = retry.payload ?? lastUserPrompt(items);
  return payload && (payload.text || payload.attachments.length) ? payload : undefined;
}

function lastUserPrompt(items: ConversationItem[]): SignInRetry["payload"] {
  const last = items.filter(item => item.type === "message" && item.role === "user").at(-1);
  if (!last) return undefined;
  const attachments = attachmentUris(last.data).map(dataUri => ({
    id: crypto.randomUUID(),
    mediaType: dataUri.slice("data:".length, dataUri.indexOf(";")),
    dataUri,
  }));
  return { text: last.text.trim(), attachments };
}
