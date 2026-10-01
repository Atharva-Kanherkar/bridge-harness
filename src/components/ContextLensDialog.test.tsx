// @vitest-environment jsdom
// The Context lens modal: one tab per live window, an honest tab for a window
// that cannot report, the chat's replaced windows, a detail view with the
// pressure hero and composition, and polling that gives up quietly.
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { bridgeApi } from "../api";
import type { ContextWindowsResult } from "../protocol/generated/protocol";
import { ContextLensDialog } from "./ContextLensDialog";

// jsdom has no Web Animations API; Base UI's dialog asks for running
// animations when it closes.
if (typeof Element !== "undefined" && !Element.prototype.getAnimations) {
  Element.prototype.getAnimations = () => [];
}

let container: HTMLDivElement;
let root: Root;

const result: ContextWindowsResult = {
  sessionId: "chat",
  windows: [
    {
      sessionId: "chat", label: "Chat", kind: "direct", role: "chat", harness: "claude", model: "claude-opus-5-5", status: "ready", depth: 0, unavailableReason: null,
      current: {
        usedTokens: 164_000, windowTokens: 200_000, percent: 82, state: "measured", source: "claude.context_usage", observedAt: "2026-10-01T10:00:00Z", turnId: "t",
        autoCompactTokens: 186_000, compactionOwner: "harness",
        segments: [
          { name: "Free space", tokens: 36_000, kind: "free" },
          { name: "Messages", tokens: 70_000, kind: "used" },
          { name: "Tool results", tokens: 50_000, kind: "used" },
        ],
        consumers: [{ label: "MCP · railway", tokens: 9_000, detail: "47 tools · every turn" }],
        forecast: { growthPerTurn: 8_000, turnsRemaining: 3, samples: 6 },
      },
    },
    {
      sessionId: "w1", label: "Verification · strong", kind: "worker", role: "worker", harness: "codex", model: "gpt-5.6", status: "working", depth: 1, unavailableReason: null,
      current: { usedTokens: 12_000, windowTokens: 400_000, percent: 3, state: "reported", source: "codex.token_usage", observedAt: "now", turnId: null, autoCompactTokens: null, compactionOwner: "harness", segments: [], consumers: [], forecast: null },
    },
    { sessionId: "w2", label: "Docs", kind: "worker", role: "worker", harness: "cursor", model: null, status: "working", depth: 1, current: null, unavailableReason: "Cursor has not reported its context window in this chat." },
  ],
  earlier: [{ harness: "claude", model: "claude-sonnet-5-5", usedTokens: 52_000, windowTokens: 200_000, percent: 26, state: "measured", observedAt: "then" }],
  bridge: { stableTokens: 3_200, variableTokens: 1_800, method: "chars/4" },
};

beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.restoreAllMocks();
});

const dialog = () => document.body.querySelector<HTMLElement>('[role="dialog"][aria-label="Context lens"]');
const tabs = () => [...document.body.querySelectorAll<HTMLButtonElement>('[role="tab"]')];

async function mount(props: Partial<Parameters<typeof ContextLensDialog>[0]> = {}) {
  await act(async () => {
    root.render(<ContextLensDialog open sessionId="chat" onClose={() => {}} {...props} />);
  });
  await act(async () => { await Promise.resolve(); });
}

describe("ContextLensDialog", () => {
  it("opens as a modal with one tab per window, and an honest tab for one that cannot report", async () => {
    vi.spyOn(bridgeApi, "contextWindows").mockResolvedValue(result);
    await mount();
    expect(dialog()).not.toBeNull();
    expect(container.querySelector('[role="dialog"]')).toBeNull(); // portalled, not inline
    const text = dialog()!.textContent!;
    expect(text).toContain("Context lens");
    expect(text).toContain("2 live · 3 windows");
    const labels = tabs().map(tab => tab.textContent);
    expect(labels).toHaveLength(3);
    expect(labels[0]).toContain("Chat");
    expect(labels[0]).toContain("82%");
    expect(labels[1]).toContain("Verification · strong");
    expect(labels[1]).toContain("3%");
    expect(labels[2]).toContain("Docs");
    expect(labels[2]).not.toContain("0%");
    expect(labels[2]).toContain("–");
    expect(tabs()[0].getAttribute("aria-selected")).toBe("true");
  });

  it("shows the chat's window first: pressure hero, composition, consumers, forecast, Bridge's share and earlier windows", async () => {
    vi.spyOn(bridgeApi, "contextWindows").mockResolvedValue(result);
    await mount();
    const pressure = dialog()!.querySelector<HTMLElement>('[aria-label="Context pressure"]')!;
    expect(pressure.textContent).toContain("High pressure");
    expect(pressure.className).toContain("border-warning/40");
    expect(pressure.textContent).toContain("164k");
    expect(pressure.textContent).toContain("compacts ~186k");
    const bar = pressure.querySelector('[role="img"]')!;
    expect(bar.getAttribute("aria-label")).toBe("82% of the window is occupied");
    expect(bar.querySelector(".bg-ctx-1")).not.toBeNull();
    expect(bar.querySelector(".bg-ctx-2")).not.toBeNull();
    expect(bar.querySelector(".ctx-hatch-neutral")).not.toBeNull();
    const text = dialog()!.textContent!;
    expect(text).toContain("Messages");
    expect(text).toContain("Not attributed");
    expect(text).toContain("44k"); // 164k − 70k − 50k
    expect(text).toContain("MCP · railway");
    expect(text).toContain("47 tools · every turn");
    expect(text).toContain("About 3 turns until it compacts");
    expect(text).toContain("What Bridge adds");
    expect(text).toContain("Earlier in this chat");
    expect(text).toContain("52k / 200k · 26%");
    expect(text).toContain("compacts this window itself");
  });

  it("switches windows from the tabs and says when a harness reports fullness but not contents", async () => {
    vi.spyOn(bridgeApi, "contextWindows").mockResolvedValue(result);
    await mount();
    await act(async () => tabs()[1].click());
    expect(tabs()[1].getAttribute("aria-selected")).toBe("true");
    const text = dialog()!.textContent!;
    expect(text).toContain("reports how full the window is but not what is in it");
    expect(text).not.toContain("What Bridge adds");
    expect(text).not.toContain("Earlier in this chat");
    await act(async () => tabs()[2].click());
    const pressure = dialog()!.querySelector<HTMLElement>('[aria-label="Context pressure"]')!;
    expect(pressure.textContent).toContain("No reading yet");
    expect(pressure.textContent).toContain("Cursor has not reported its context window in this chat.");
    expect(pressure.textContent).not.toContain("0%");
  });

  it("opens on the requested window", async () => {
    vi.spyOn(bridgeApi, "contextWindows").mockResolvedValue(result);
    await mount({ focusSessionId: "w1" });
    expect(tabs()[1].getAttribute("aria-selected")).toBe("true");
  });

  it("hides the tab strip when the chat is the only window", async () => {
    vi.spyOn(bridgeApi, "contextWindows").mockResolvedValue({ ...result, windows: [result.windows[0]] });
    await mount();
    expect(tabs()).toHaveLength(0);
    expect(dialog()!.querySelector('[aria-label="Context pressure"]')).not.toBeNull();
  });

  it("stops polling and says so when the daemon lacks the method", async () => {
    const fetch = vi.spyOn(bridgeApi, "contextWindows").mockRejectedValue(new Error("method_not_found: sessions/get_context_windows"));
    await mount();
    expect(fetch).toHaveBeenCalledTimes(1);
    expect(dialog()!.querySelector('[role="alert"]')!.textContent).toContain("Context is unavailable");
  });

  it("does not fetch while closed", async () => {
    const fetch = vi.spyOn(bridgeApi, "contextWindows").mockResolvedValue(result);
    await mount({ open: false });
    expect(fetch).not.toHaveBeenCalled();
    expect(dialog()).toBeNull();
  });

  it("closes through the dialog", async () => {
    vi.spyOn(bridgeApi, "contextWindows").mockResolvedValue(result);
    const onClose = vi.fn();
    await mount({ onClose });
    await act(async () => dialog()!.querySelector<HTMLButtonElement>('button[aria-label="Close"]')!.click());
    expect(onClose).toHaveBeenCalledTimes(1);
  });
});
