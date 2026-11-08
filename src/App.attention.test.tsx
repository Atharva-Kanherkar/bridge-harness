// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { bridgeApi } from "./api";
import { pushMockAttention, resetMockAttention } from "./attentionMock";
import type { BridgeState, Session, SessionStatus } from "./types";
import { NEEDS_YOU_HOLD_MS } from "./useAttentionFeed";

const notifyAttention = vi.hoisted(() => vi.fn());
vi.mock("./attention", () => ({ notifyAttention }));

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
    resetMockAttention();
    vi.spyOn(bridgeApi, "state").mockImplementation(async () => stateWith([
      session("a", "working"),
      session("b", "ready"),
      session("w", "working", { parentSessionId: "a", title: "Worker w" }),
    ]));
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
});
