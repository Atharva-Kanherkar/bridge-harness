import { chatName } from "./components/sidebarChats";
import type { AttentionEvent } from "./attentionEvents";
import { harnessLabel } from "./utils";

export type AttentionCopy = {
  headline: string;
  detail: string;
  tone: "needs-you" | "completed" | "failed";
};

/**
 * Shared copy for the in-app glass toast and the OS banner. Keeps both surfaces
 * saying the same thing so a human who saw the macOS notification recognizes
 * the toast when they return to Bridge.
 */
export function attentionCopy(event: AttentionEvent): AttentionCopy {
  const name = chatName(event.session);
  const harness = harnessLabel(event.session.harness);
  if (event.kind === "needs-you") {
    return {
      headline: "Bridge needs you",
      detail: `${name} · ${harness} is waiting for your input`,
      tone: "needs-you",
    };
  }
  if (event.session.status === "failed") {
    return {
      headline: "Turn failed",
      detail: `${name} · ${harness} ended this turn with an error`,
      tone: "failed",
    };
  }
  return {
    headline: "Turn completed",
    detail: `${name} · ${harness} finished its turn`,
    tone: "completed",
  };
}

export function attentionToastKey(event: AttentionEvent): string {
  return `${event.session.id}:${event.kind}:${event.session.status}`;
}
