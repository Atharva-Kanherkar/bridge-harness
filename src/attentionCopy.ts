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
export function attentionCopy(item: Pick<AttentionItem, "kind" | "title" | "body">, session?: Session): AttentionCopy {
  const tone = TONES[item.kind];
  if (!session) return { headline: item.title, detail: item.body ?? "", tone };
  const who = `${chatName(session)} · ${harnessLabel(session.harness)}`;
  if (item.kind === "needsYou") return { headline: "Bridge needs you", detail: `${who} is waiting for your input`, tone };
  if (item.kind === "failed") return { headline: "Turn failed", detail: `${who} ended this turn with an error`, tone };
  return { headline: "Turn completed", detail: `${who} finished its turn`, tone };
}

export function attentionToastKey(item: Pick<AttentionItem, "sequence">): string {
  return `attention:${item.sequence}`;
}
