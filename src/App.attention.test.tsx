// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { bridgeApi } from "./api";
import { mockUnreadSummary, pushMockAttention, resetMockAttention } from "./attentionMock";
import { DEFAULT_ATTENTION_SETTINGS, writeAttentionSettings, type AttentionSettings } from "./attentionSettings";
import type { BridgeState, Session, SessionStatus } from "./types";
import { NEEDS_YOU_HOLD_MS } from "./useAttentionFeed";

const notifyAttention = vi.hoisted(() => vi.fn());
vi.mock("./attention", () => ({ notifyAttention }));
const playAttentionSound = vi.hoisted(() => vi.fn());
vi.mock("./attentionSound", () => ({ playAttentionSound }));

function session(id: string, status: SessionStatus, overrides: Partial<Session> = {}): Session {
  return {
    continuationFidelity: "native",
    harness: "claude",
    id,
    kind: "chat",
    label: `Chat ${id}`,
    metricSource: "provider",
    restorationMode: "fresh",
    status,
    title: `Title ${id}`,
    ...overrides,
  };
}

function stateWith(sessions: Session[]): BridgeState {
  return { projects: [], workspaces: [], sessions, events: [] };
}

const wait = (ms: number) => act(async () => { await new Promise(resolve => setTimeout(resolve, ms)); });
const toastRegion = (container: HTMLElement) => container.querySelector('[aria-label="Attention notifications"]');

describe("App attention wiring", () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    const store = new Map<string, string>();
    Object.defineProperty(globalThis, "localStorage", {
      configurable: true,
      value: {
        getItem: (key: string) => store.get(key) ?? null,
        setItem: (key: string, value: string) => { store.set(key, String(value)); },
        removeItem: (key: string) => { store.delete(key); },
        clear: () => store.clear(),
        key: () => null,
        length: 0,
      },
    });
    notifyAttention.mockReset();
    playAttentionSound.mockReset();
    resetMockAttention();
    vi.spyOn(bridgeApi, "state").mockImplementation(async () => stateWith([
      session("a", "working"),
      session("b", "ready"),
      session("w", "working", { parentSessionId: "a", title: "Worker w" }),
    ]));
    // The mock summary's session tree is the one this test's state serves.
    const parents: Record<string, string | null> = { a: null, b: null, w: "a" };
    vi.spyOn(bridgeApi, "unreadSummary").mockImplementation(async () => mockUnreadSummary(id => parents[id]));
    container = document.createElement("div");
    document.body.append(container);
    root = createRoot(container);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
    vi.restoreAllMocks();
  });

  async function mountApp() {
    const { App } = await import("./App");
    await act(async () => { root.render(<App />); });
    await wait(40);
  }

  it("shows nothing for rows written before launch", async () => {
    pushMockAttention({ kind: "finished", sessionId: "b", title: "Turn completed", body: "Title b · Claude finished its turn" });
    await mountApp();
    await wait(40);
    expect(notifyAttention).not.toHaveBeenCalled();
    expect(toastRegion(container)).toBeFalsy();
  });

  it("raises a worker's approval request once, with its approval id", async () => {
    await mountApp();
    await act(async () => { pushMockAttention({ kind: "needsYou", sessionId: "w", title: "Bridge needs you", approvalEventId: 7 }); });
    await wait(40);
    expect(notifyAttention, "held for a second first").not.toHaveBeenCalled();
    await wait(NEEDS_YOU_HOLD_MS + 100);
    expect(notifyAttention).toHaveBeenCalledTimes(1);
    expect(notifyAttention).toHaveBeenCalledWith(
      "Bridge needs you",
      "Worker w · Claude is waiting for your input",
      { sessionId: "w", approvalEventId: 7 },
    );
    expect(toastRegion(container)?.textContent).toContain("Worker w · Claude is waiting for your input");
  });

  it("replaces a chat's toast with its newer row", async () => {
    await mountApp();
    await act(async () => { pushMockAttention({ kind: "failed", sessionId: "b", title: "Turn failed" }); });
    await wait(40);
    await act(async () => { pushMockAttention({ kind: "finished", sessionId: "b", title: "Turn completed" }); });
    await wait(40);
    const text = toastRegion(container)?.textContent ?? "";
    expect(text).toContain("Turn completed");
    expect(text).not.toContain("Turn failed");
    expect(notifyAttention).toHaveBeenCalledTimes(2);
  });

  it("opens the chat, dismisses the card and marks it seen when the toast is clicked", async () => {
    const seen = vi.spyOn(bridgeApi, "markAttentionSeen");
    await mountApp();
    await act(async () => { pushMockAttention({ kind: "finished", sessionId: "b", title: "Turn completed" }); });
    await wait(40);

    const toastButton = container.querySelector('[aria-label="Attention notifications"] button[aria-label*="open the chat"]') as HTMLButtonElement;
    expect(toastButton, "toast body button for the finished chat").toBeTruthy();
    await act(async () => { toastButton.click(); });
    await wait(20);

    expect(container.querySelector('[aria-label="Attention notifications"] button[aria-label*="open the chat"]')).toBeFalsy();
    expect(container.querySelector('button[title^="Title b"]')?.getAttribute("aria-current")).toBe("page");
    expect(seen).toHaveBeenCalledWith("b");
  });

  it("offers Deliver now on an escalation and delivers that chat's input", async () => {
    const deliver = vi.spyOn(bridgeApi, "deliverQueuedInput").mockResolvedValue({ disposition: "delivered" });
    await mountApp();
    await act(async () => {
      pushMockAttention({ kind: "escalation", sessionId: "a", title: "A result is waiting", body: "Title a · Claude stopped before it read 1 update" });
    });
    await wait(40);
    expect(toastRegion(container)?.textContent).toContain("A result is waiting");
    expect(notifyAttention).toHaveBeenCalledWith("A result is waiting", "Title a · Claude stopped before it read 1 update", { sessionId: "a", approvalEventId: null });

    const button = [...container.querySelectorAll('[aria-label="Attention notifications"] button')].find(item => item.textContent === "Deliver now") as HTMLButtonElement;
    expect(button, "the remedy is on the card").toBeTruthy();
    await act(async () => { button.click(); });
    await wait(20);
    expect(deliver).toHaveBeenCalledWith("a");
    expect(toastRegion(container)).toBeFalsy();
  });

  it("marks the chat seen when its toast reopens the retained selection", async () => {
    const seen = vi.spyOn(bridgeApi, "markAttentionSeen");
    await mountApp();
    await act(async () => { (container.querySelector('button[title^="Title b"]') as HTMLButtonElement).click(); });
    await wait(20);
    // Away to Memory; the chat stays selected underneath.
    const memory = [...container.querySelectorAll("button")].find(button => button.textContent?.trim().startsWith("Memory")) as HTMLButtonElement;
    expect(memory, "the Memory entry in the sidebar").toBeTruthy();
    await act(async () => { memory.click(); });
    await wait(20);
    seen.mockClear();

    await act(async () => { pushMockAttention({ kind: "finished", sessionId: "b", title: "Turn completed" }); });
    await wait(40);
    const toastButton = container.querySelector('[aria-label="Attention notifications"] button[aria-label*="open the chat"]') as HTMLButtonElement;
    await act(async () => { toastButton.click(); });
    await wait(20);
    expect(seen).toHaveBeenCalledWith("b");
  });

  it("passes no approval id for a question", async () => {
    await mountApp();
    // Core gives a question no approval id: it is answered through
    // resolve_question, so the Linux banner must offer Open only.
    await act(async () => { pushMockAttention({ kind: "needsYou", sessionId: "w", title: "Bridge needs you", approvalEventId: null }); });
    await wait(NEEDS_YOU_HOLD_MS + 100);
    expect(notifyAttention).toHaveBeenCalledWith("Bridge needs you", "Worker w · Claude is waiting for your input", { sessionId: "w", approvalEventId: null });
  });

  it("keeps Deliver now when delivery fails", async () => {
    vi.spyOn(bridgeApi, "deliverQueuedInput").mockRejectedValue(new Error("Bridge could not hand the waiting input to the agent; it is still queued. Try again."));
    await mountApp();
    await act(async () => {
      pushMockAttention({ kind: "escalation", sessionId: "a", title: "A result is waiting", body: "Title a · Claude stopped before it read 1 update" });
    });
    await wait(40);
    const button = [...container.querySelectorAll('[aria-label="Attention notifications"] button')].find(item => item.textContent === "Deliver now") as HTMLButtonElement;
    await act(async () => { button.click(); });
    await wait(20);
    expect(toastRegion(container)?.textContent, "the remedy is still there to retry").toContain("Deliver now");
    expect(container.textContent).toContain("it is still queued");
  });

  describe("unread", () => {
    const row = (container: HTMLElement, id: string) => container.querySelector(`button[title^="Title ${id}"]`) as HTMLButtonElement;
    const open = async (container: HTMLElement, id: string) => {
      await act(async () => { row(container, id).click(); });
      await wait(20);
    };

    it("marks a chat that finishes while another is open, badges it, and clears both when opened", async () => {
      const badge = vi.spyOn(bridgeApi, "setUnreadBadge");
      await mountApp();
      await open(container, "a");
      await act(async () => { pushMockAttention({ kind: "finished", sessionId: "b", title: "Turn completed" }); });
      await wait(40);
      expect(row(container, "b").textContent).toContain("Unread: ");
      expect(row(container, "a").textContent).not.toContain("Unread");
      expect(badge).toHaveBeenLastCalledWith(1);

      await open(container, "b");
      await wait(20);
      expect(row(container, "b").textContent).not.toContain("Unread");
      expect(badge).toHaveBeenLastCalledWith(0);
    });

    it("sends the badge event again when what needs you changes at the same total", async () => {
      // The native menu bar wakes on this event and words its row from
      // needsYou, so a finished chat that starts waiting must not go unheard.
      const badge = vi.spyOn(bridgeApi, "setUnreadBadge");
      await mountApp();
      await open(container, "a");
      await act(async () => { pushMockAttention({ kind: "finished", sessionId: "b", title: "Turn completed" }); });
      await wait(40);
      const calls = badge.mock.calls.length;
      expect(badge).toHaveBeenLastCalledWith(1);
      await act(async () => { pushMockAttention({ kind: "escalation", sessionId: "b", title: "A result is waiting" }); });
      await wait(40);
      expect(badge.mock.calls.length).toBeGreaterThan(calls);
      expect(badge).toHaveBeenLastCalledWith(1);
    });

    it("clears the dot when another client marks the chat seen", async () => {
      await mountApp();
      await act(async () => { pushMockAttention({ kind: "failed", sessionId: "b", title: "Turn failed" }); });
      await wait(40);
      expect(row(container, "b").textContent).toContain("Unread: ");
      // What the web client's open does: a server-side stamp and a hint.
      await act(async () => { await bridgeApi.markAttentionSeen("b"); });
      await wait(20);
      expect(row(container, "b").textContent).not.toContain("Unread");
    });

    it("marks an orchestrator needs you while its worker waits", async () => {
      await mountApp();
      await open(container, "b");
      await act(async () => { pushMockAttention({ kind: "needsYou", sessionId: "w", title: "Bridge needs you", approvalEventId: 3 }); });
      await wait(40);
      const text = row(container, "a").textContent ?? "";
      expect(text).toContain("Unread: ");
      expect(text).toContain("needs you");
      expect(text).not.toContain("working");
    });

    it("marks a row for the chat on screen seen at once in a focused window", async () => {
      vi.spyOn(document, "hasFocus").mockReturnValue(true);
      await mountApp();
      await open(container, "b");
      const seen = vi.spyOn(bridgeApi, "markAttentionSeen");
      await act(async () => { pushMockAttention({ kind: "finished", sessionId: "b", title: "Turn completed" }); });
      await wait(40);
      expect(seen).toHaveBeenCalledWith("b");
      expect(row(container, "b").textContent).not.toContain("Unread");
    });

    it("leaves the chat on screen unread while the window is in the background", async () => {
      vi.spyOn(document, "hasFocus").mockReturnValue(false);
      const badge = vi.spyOn(bridgeApi, "setUnreadBadge");
      await mountApp();
      await open(container, "b");
      await act(async () => { window.dispatchEvent(new Event("blur")); });
      await act(async () => { pushMockAttention({ kind: "finished", sessionId: "b", title: "Turn completed" }); });
      await wait(40);
      expect(badge).toHaveBeenLastCalledWith(1);
      // Coming back to Bridge is seeing it.
      vi.spyOn(document, "hasFocus").mockReturnValue(true);
      await act(async () => { window.dispatchEvent(new Event("focus")); });
      await wait(40);
      expect(badge).toHaveBeenLastCalledWith(0);
    });
  });

  describe("policy", () => {
    const openChat = async (id: string) => {
      await act(async () => { (container.querySelector(`button[title^="Title ${id}"]`) as HTMLButtonElement).click(); });
      await wait(20);
    };

    it("says nothing about the chat on screen in a focused window", async () => {
      vi.spyOn(document, "hasFocus").mockReturnValue(true);
      await mountApp();
      await openChat("b");
      await act(async () => { pushMockAttention({ kind: "finished", sessionId: "b", title: "Turn completed" }); });
      await wait(40);
      expect(toastRegion(container)).toBeFalsy();
      expect(notifyAttention).not.toHaveBeenCalled();
    });

    it("keeps eight toasts queued and reveals the next on dismiss", async () => {
      await mountApp();
      for (let index = 1; index <= 9; index += 1) {
        await act(async () => { pushMockAttention({ kind: "failed", sessionId: `s${index}`, title: "Turn failed", body: `Chat s${index} · Codex ended this turn with an error` }); });
        await wait(10);
      }
      const shown = () => [...container.querySelectorAll('[aria-label="Attention notifications"] button[aria-label*="open the chat"]')]
        .map(button => /Chat (s\d)/.exec(button.textContent ?? "")?.[1]);
      expect(shown()).toEqual(["s7", "s8", "s9"]);
      const dismissNewest = async () => {
        const buttons = container.querySelectorAll('[aria-label="Attention notifications"] button[aria-label="Dismiss notification"]');
        await act(async () => { (buttons[buttons.length - 1] as HTMLButtonElement).click(); });
      };
      await dismissNewest();
      expect(shown(), "the next queued toast appears").toEqual(["s6", "s7", "s8"]);
      for (let count = 0; count < 5; count += 1) await dismissNewest();
      expect(shown(), "the ninth row evicted the oldest").toEqual(["s2", "s3"]);
    });

    it("toasts another chat in a focused window, without an OS banner", async () => {
      vi.spyOn(document, "hasFocus").mockReturnValue(true);
      await mountApp();
      await openChat("a");
      await act(async () => { pushMockAttention({ kind: "failed", sessionId: "b", title: "Turn failed" }); });
      await wait(40);
      expect(toastRegion(container)?.textContent).toContain("Turn failed");
      expect(notifyAttention).not.toHaveBeenCalled();
    });
  });

  describe("settings", () => {
    const set = (change: (next: AttentionSettings) => void) => {
      const next = structuredClone(DEFAULT_ATTENTION_SETTINGS);
      change(next);
      writeAttentionSettings(next);
    };
    const hhmm = (date: Date) => `${String(date.getHours()).padStart(2, "0")}:${String(date.getMinutes()).padStart(2, "0")}`;

    it("shows nothing for a kind whose toast and banner are off", async () => {
      set(next => { next.kinds.finished = { toast: false, banner: false, sound: false }; });
      await mountApp();
      await act(async () => { pushMockAttention({ kind: "finished", sessionId: "b", title: "Turn completed" }); });
      await wait(40);
      expect(toastRegion(container)).toBeFalsy();
      expect(notifyAttention).not.toHaveBeenCalled();
    });

    it("plays one sound for a request that is still open after the hold", async () => {
      await mountApp();
      await act(async () => { pushMockAttention({ kind: "needsYou", sessionId: "w", title: "Bridge needs you" }); });
      await wait(NEEDS_YOU_HOLD_MS + 100);
      expect(playAttentionSound).toHaveBeenCalledTimes(1);
      expect(playAttentionSound).toHaveBeenCalledWith("needsYou");
      await act(async () => { pushMockAttention({ kind: "finished", sessionId: "b", title: "Turn completed" }); });
      await wait(40);
      expect(playAttentionSound, "finished turns are silent by default").toHaveBeenCalledTimes(1);
    });

    it("keeps the toast but posts no banner in quiet hours", async () => {
      const now = new Date();
      set(next => { next.quietHours = { enabled: true, start: hhmm(new Date(now.getTime() - 3_600_000)), end: hhmm(new Date(now.getTime() + 3_600_000)) }; });
      await mountApp();
      await act(async () => { pushMockAttention({ kind: "failed", sessionId: "b", title: "Turn failed" }); });
      await wait(40);
      expect(toastRegion(container)?.textContent).toContain("Turn failed");
      expect(notifyAttention).not.toHaveBeenCalled();
    });
  });
});

