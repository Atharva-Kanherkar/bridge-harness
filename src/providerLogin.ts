import type { AgentEvent } from "./types";
import { readWireKind } from "./transcript/wire";
import type { UsageProvider } from "./usage";

/** Only a failed provider operation can start recovery; a generic 403 could
 * be a policy refusal and must never open an unrelated login flow. */
export function needsProviderSignIn(harness: string, message: string): UsageProvider | null {
  if (!["codex", "claude", "cursor", "opencode"].includes(harness)) return null;
  const expired = /(?:token|credentials?|session).{0,20}expired|expired (?:token|credentials?)|not (?:logged|signed) in|authentication (?:failed|required)|please (?:log|sign) ?in|re-?authenticate|invalid (?:access|refresh) token|\b401\s+(?:unauthorized|authentication)|refresh token.{0,80}(?:already used|revoked)/i;
  return expired.test(message) ? harness as UsageProvider : null;
}

/** Live provider errors only. Transcript text mentioning login is not a request. */
export function providerSignInForEvent(harness: string, event: Pick<AgentEvent, "kind" | "text">): UsageProvider | null {
  return readWireKind(event.kind) === "error" ? needsProviderSignIn(harness, event.text ?? "") : null;
}
