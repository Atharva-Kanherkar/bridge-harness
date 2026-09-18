// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ATTENTION_TOAST_TTL_MS, AttentionToasts, type AttentionToast } from "./AttentionToasts";

let root: Root | undefined;
let host: HTMLDivElement | undefined;

const needsYou: AttentionToast = {
  key: "s1:needs-you:waiting",
  sessionId: "s1",
  copy: {
    headline: "Bridge needs you",
    detail: "Auth tokens · Claude is waiting for your input",
    tone: "needs-you",
  },
};

const completed: AttentionToast = {
  key: "s2:turn-completed:ready",
  sessionId: "s2",
  copy: {
    headline: "Turn completed",
    detail: "Landing page · Codex finished its turn",
    tone: "completed",
  },
};

async function mount(ui: React.ReactElement) {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
  await act(async () => { root?.render(ui); });
}

afterEach(async () => {
  await act(async () => { root?.unmount(); });
  host?.remove();
  root = undefined;
  host = undefined;
  vi.restoreAllMocks();
  vi.useRealTimers();
});

describe("AttentionToasts", () => {
  it("renders copy, opens on click, and dismisses on the X", async () => {
    const onOpen = vi.fn();
    const onDismiss = vi.fn();
    await mount(<AttentionToasts toasts={[needsYou, completed]} onOpen={onOpen} onDismiss={onDismiss} />);
    expect(host!.textContent).toContain("Bridge needs you");
    expect(host!.textContent).toContain("Auth tokens · Claude is waiting for your input");
    expect(host!.textContent).toContain("Turn completed");

    (host!.querySelector('button[aria-label*="open the chat"]') as HTMLButtonElement).click();
    expect(onOpen).toHaveBeenCalledWith(needsYou);
    (host!.querySelector('button[aria-label="Dismiss notification"]') as HTMLButtonElement).click();
    expect(onDismiss).toHaveBeenCalledWith(needsYou.key);
  });

  it("auto-dismisses a card after its TTL", async () => {
    vi.useFakeTimers();
    const onDismiss = vi.fn();
    await mount(<AttentionToasts toasts={[needsYou]} onOpen={() => undefined} onDismiss={onDismiss} />);
    await act(async () => { vi.advanceTimersByTime(ATTENTION_TOAST_TTL_MS + 1); });
    expect(onDismiss).toHaveBeenCalledWith(needsYou.key);
  });

  it("keeps counting toward the original TTL when the parent re-renders with a new onDismiss identity", async () => {
    // App re-renders on unrelated state (e.g. its git-stat poll) and always
    // passes a fresh onDismiss closure. That must not restart the timer.
    vi.useFakeTimers();
    const firstDismiss = vi.fn();
    const secondDismiss = vi.fn();
    await mount(<AttentionToasts toasts={[needsYou]} onOpen={() => undefined} onDismiss={firstDismiss} />);
    await act(async () => { vi.advanceTimersByTime(ATTENTION_TOAST_TTL_MS - 1); });
    expect(firstDismiss).not.toHaveBeenCalled();

    await act(async () => {
      root?.render(<AttentionToasts toasts={[needsYou]} onOpen={() => undefined} onDismiss={secondDismiss} />);
    });
    await act(async () => { vi.advanceTimersByTime(2); });
    expect(secondDismiss).toHaveBeenCalledWith(needsYou.key);
  });

  it("restarts the TTL when the same key fires again with a fresh occurrence", async () => {
    vi.useFakeTimers();
    const onDismiss = vi.fn();
    const first: AttentionToast = { ...needsYou, firedAt: 1 };
    await mount(<AttentionToasts toasts={[first]} onOpen={() => undefined} onDismiss={onDismiss} />);
    await act(async () => { vi.advanceTimersByTime(ATTENTION_TOAST_TTL_MS - 1); });
    expect(onDismiss).not.toHaveBeenCalled();

    const second: AttentionToast = { ...needsYou, firedAt: 2 };
    await act(async () => {
      root?.render(<AttentionToasts toasts={[second]} onOpen={() => undefined} onDismiss={onDismiss} />);
    });
    await act(async () => { vi.advanceTimersByTime(ATTENTION_TOAST_TTL_MS - 1); });
    expect(onDismiss).not.toHaveBeenCalled();
    await act(async () => { vi.advanceTimersByTime(2); });
    expect(onDismiss).toHaveBeenCalledWith(needsYou.key);
  });

  it("renders nothing when empty", async () => {
    await mount(<AttentionToasts toasts={[]} onOpen={() => undefined} onDismiss={() => undefined} />);
    expect(host!.textContent).toBe("");
  });
});
