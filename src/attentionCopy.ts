import { chatName } from "./components/sidebarChats";
import type { AttentionItem, Session } from "./types";
import { harnessLabel } from "./utils";

export type AttentionCopy = {
  headline: string;
  detail: string;
  tone: "needs-you" | "completed" | "failed";
};

const TONES: Record<AttentionItem["kind"], AttentionCopy["tone"]> = {
  needsYou: "needs-you",
  finished: "completed",
  failed: "failed",
  escalation: "needs-you",
  // An informational post reads like a finished turn; see below.
  posted: "needs-you",
  // Never shown: the feed drops kinds it does not know.
  other: "completed",
};

/**
 * Shared copy for the in-app glass toast and the OS banner, so a human who saw
 * the macOS notification recognizes the toast when they return to Bridge.
 *
 * Core writes the same sentences into the row. When the chat is known here its
 * live name wins: a title job may have renamed it since the row was written.
 */
export function attentionCopy(item: Pick<AttentionItem, "kind" | "title" | "body"> & Partial<Pick<AttentionItem, "severity">>, session?: Session): AttentionCopy {
  const tone = item.kind === "posted" && item.severity !== "actionable" ? "completed" : TONES[item.kind];
  const who = session ? `${chatName(session)} · ${harnessLabel(session.harness)}` : undefined;
  // A post is the agent's own words; the chat it came from is the detail
  // when it gave none.
  if (item.kind === "posted") return { headline: item.title, detail: item.body ?? (who ? `${who} asks for you` : ""), tone };
  // An escalation's body carries how much is waiting, which only core knows.
  if (!who || item.kind === "escalation") return { headline: item.title, detail: item.body ?? "", tone };
  if (item.kind === "needsYou") return { headline: "Bridge needs you", detail: `${who} is waiting for your input`, tone };
  if (item.kind === "failed") return { headline: "Turn failed", detail: `${who} ended this turn with an error`, tone };
  return { headline: "Turn completed", detail: `${who} finished its turn`, tone };
}

export function attentionToastKey(item: Pick<AttentionItem, "sequence">): string {
  return `attention:${item.sequence}`;
}
