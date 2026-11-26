// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { MotionGlobalConfig } from "framer-motion";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { AgentConversation } from "./AgentConversation";
import { asWireKind } from "../transcript/wire";
import { formatReplyPrompt } from "../replyTo";
import type { AgentEvent, Session, SessionEntry } from "../types";

const session: Session = { id: "s", workspaceId: "w", harness: "codex", label: "Chat", status: "idle", startedAt: "now", endedAt: null, contextPercent: null, usagePercent: null, metricSource: "reported", model: "gpt-5.6-luna", restorationMode: "fresh", continuationFidelity: "native", kind: "chat" };
const event = (id: number, kind: string, overrides: Partial<AgentEvent> = {}): AgentEvent => ({ id, sessionId: "s", sequence: id, protocolVersion: 1, kind: asWireKind(kind), itemId: null, role: null, status: null, title: null, text: null, data: {}, providerMeta: {}, createdAt: "now", ...overrides });
const entry = (id: string, parent: string | null, sequence: number, kind: string, text: string): SessionEntry => ({
  id, sessionId: "s", parentEntryId: parent, sequence, semanticSchemaVersion: 2,
  kind, payload: { text, itemId: `item-${id}` },
  providerEventId: null, contextVisibility: "eligible", tokenEstimate: null, createdAt: "now",
});

let host: HTMLDivElement;
let root: Root;

beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  MotionGlobalConfig.skipAnimations = true;
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
});

afterEach(() => {
  act(() => root.unmount());
  host.remove();
  MotionGlobalConfig.skipAnimations = false;
});

const replyButtons = () => [...host.querySelectorAll<HTMLButtonElement>('button[aria-label="Reply"]')];

describe("Reply on an agent message", () => {
  it("shows on a settled assistant message and hands that message to onReply", async () => {
    const onReply = vi.fn();
    const answer = event(2, "message.completed", { itemId: "a", role: "assistant", status: "completed", text: "Option A, then option B." });
    const ask = event(1, "message.completed", { itemId: "u", role: "user", status: "completed", text: "Which option?" });
    await act(async () => root.render(<AgentConversation session={session} events={[ask, answer]} onResolve={() => undefined} onReply={onReply} />));
    // One Reply, on the agent's message; the user's own bubble has none.
    expect(replyButtons()).toHaveLength(1);
    expect(replyButtons()[0].closest("[data-reply-actions]")?.getAttribute("data-reply-actions")).toBe("latest");
    const labels = [...host.querySelectorAll("[data-reply-actions='latest'] button")].map(button => button.getAttribute("aria-label"));
    expect(labels.slice(0, 2)).toEqual(["Copy", "Reply"]);
    await act(async () => replyButtons()[0].click());
    expect(onReply).toHaveBeenCalledTimes(1);
    expect(onReply.mock.calls[0][0]).toMatchObject({ role: "assistant", text: "Option A, then option B." });
  });

  it("is absent on a streaming message, a read-only transcript and a preview", async () => {
    const onReply = vi.fn();
    const streaming = event(1, "message.delta", { itemId: "a", role: "assistant", status: "streaming", text: "Half an ans" });
    await act(async () => root.render(<AgentConversation session={{ ...session, status: "working" }} events={[streaming]} onResolve={() => undefined} onReply={onReply} />));
    expect(replyButtons()).toHaveLength(0);

    const settled = event(1, "message.completed", { itemId: "a", role: "assistant", status: "completed", text: "A whole answer." });
    await act(async () => root.render(<AgentConversation session={session} events={[settled]} onResolve={() => undefined} onReply={onReply} readOnly />));
    expect(host.textContent).toContain("A whole answer.");
    expect(replyButtons()).toHaveLength(0);

    await act(async () => root.render(<AgentConversation session={session} events={[settled]} onResolve={() => undefined} onReply={onReply} preview />));
    expect(host.textContent).toContain("A whole answer.");
    expect(replyButtons()).toHaveLength(0);
  });

  it("draws a sent reply as a card over the user's words, and the card jumps back to the original", async () => {
    const onRevealEntry = vi.fn();
    const prompt = formatReplyPrompt({ entryId: "e-a", excerpt: "## Plan\n1. **Ship** the store" }, "Go with step one.");
    const forestEntries = [
      entry("e-q", null, 1, "user.message", "Make a plan."),
      entry("e-a", "e-q", 2, "assistant.message", "## Plan\n1. **Ship** the store"),
      entry("e-r", "e-a", 3, "user.message", prompt),
    ];
    await act(async () => root.render(<AgentConversation session={session} forestEntries={forestEntries} activeLeafId="e-r" events={[]} onResolve={() => undefined} onRevealEntry={onRevealEntry} />));
    const card = host.querySelector<HTMLButtonElement>("button[data-reply-card]")!;
    expect(card).toBeTruthy();
    expect(card.textContent).toContain("Replying to");
    expect(card.textContent).toContain("Plan Ship the store");
    expect(card.getAttribute("aria-label")).toBe("Go to the response this replies to: Plan Ship the store");
    // The bubble shows the user's words and none of the raw prompt.
    const bubble = card.parentElement!;
    expect(bubble.textContent).toContain("Go with step one.");
    expect(bubble.textContent).not.toContain("bridge-reply");
    expect(bubble.textContent).not.toContain("> ");
    await act(async () => card.click());
    expect(onRevealEntry).toHaveBeenCalledWith("e-a");
  });

  it("renders an ordinary message, and one quoting by hand, as the text it is", async () => {
    const plain = event(1, "message.completed", { itemId: "u", role: "user", status: "completed", text: "> a quote I typed\n\nmy words" });
    await act(async () => root.render(<AgentConversation session={session} events={[plain]} onResolve={() => undefined} onRevealEntry={() => undefined} />));
    expect(host.querySelector("[data-reply-card]")).toBeNull();
    expect(host.textContent).toContain("> a quote I typed");
  });

  it("copies a sent reply as its quote and words, without the marker line", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText } });
    const prompt = formatReplyPrompt({ entryId: "e-a", excerpt: "First line" }, "Thanks.");
    const forestEntries = [entry("e-r", null, 1, "user.message", prompt)];
    await act(async () => root.render(<AgentConversation session={session} forestEntries={forestEntries} activeLeafId="e-r" events={[]} onResolve={() => undefined} />));
    const copy = host.querySelector<HTMLButtonElement>('button[aria-label="Copy"]')!;
    await act(async () => copy.click());
    expect(writeText).toHaveBeenCalledWith("> First line\n\nThanks.");
  });
});
