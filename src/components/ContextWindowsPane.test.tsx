// @vitest-environment jsdom
// The Context pane: one card per live window, an honest card for a window
// that cannot report, the chat's replaced windows, a detail view with the
// pressure card and composition, and polling that gives up quietly.
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { bridgeApi } from "../api";
import type { ContextWindowsResult } from "../protocol/generated/protocol";
import { ContextWindowsPane } from "./ContextWindowsPane";

let container: HTMLDivElement;
let root: Root;

const result: ContextWindowsResult = {
  sessionId: "chat",
  windows: [
    {
      sessionId: "chat", label: "Chat", kind: "direct", role: "chat", harness: "claude", model: "claude-opus-5-5", status: "ready", depth: 0, unavailableReason: null,
      current: {
        usedTokens: 164_000, windowTokens: 200_000, percent: 82, state: "measured", source: "claude.context_usage", observedAt: "now", turnId: "t",
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

async function mount(props: Partial<Parameters<typeof ContextWindowsPane>[0]> = {}) {
  await act(async () => {
    root.render(<ContextWindowsPane sessionId="chat" visible {...props} />);
  });
  await act(async () => { await Promise.resolve(); });
}

describe("ContextWindowsPane", () => {
  it("lists every live window, with an honest card for one that cannot report", async () => {
    vi.spyOn(bridgeApi, "contextWindows").mockResolvedValue(result);
    await mount();
    expect(container.textContent).toContain("Context windows");
    expect(container.textContent).toContain("Chat · claude-opus-5-5");
    expect(container.textContent).toContain("82%");
    expect(container.textContent).toContain("164k / 200k");
    expect(container.textContent).toContain("3%");
    expect(container.textContent).toContain("Cursor has not reported its context window in this chat.");
    expect(container.textContent).toContain("Unavailable");
    // The cursor card never claims a zero.
    const cursorCard = [...container.querySelectorAll("div.border-dashed")].find(card => card.textContent?.includes("Docs"))!;
    expect(cursorCard.textContent).not.toContain("0%");
    expect(container.textContent).toContain("Earlier in this chat");
    expect(container.textContent).toContain("52k / 200k · 26%");
    expect(container.textContent).toContain("What Bridge adds");
  });

  it("opens a window's detail with pressure, composition, consumers and forecast", async () => {
    vi.spyOn(bridgeApi, "contextWindows").mockResolvedValue(result);
    await mount();
    const card = [...container.querySelectorAll("button")].find(button => button.textContent?.includes("Chat · claude-opus-5-5"))!;
    await act(async () => card.click());
    const pressure = container.querySelector('[aria-label="Context pressure"]')!;
    expect(pressure.textContent).toContain("High pressure");
    expect(pressure.className).toContain("border-warning/40");
    expect(pressure.textContent).toContain("Auto-compacts at ~186k");
    expect(container.textContent).toContain("Messages");
    expect(container.textContent).toContain("Not attributed");
    expect(container.textContent).toContain("44k"); // 164k − 70k − 50k
    expect(container.textContent).toContain("MCP · railway");
    expect(container.textContent).toContain("47 tools · every turn");
    expect(container.textContent).toContain("About 3 turns until it compacts");
    expect(container.textContent).toContain("compacts this window itself");
    await act(async () => [...container.querySelectorAll("button")].find(button => button.textContent === "Windows")!.click());
    expect(container.textContent).toContain("Context windows");
  });

  it("says when a harness reports fullness but not contents", async () => {
    vi.spyOn(bridgeApi, "contextWindows").mockResolvedValue(result);
    await mount({ detailRequest: { sessionId: "w1", nonce: 1 } });
    expect(container.textContent).toContain("reports how full the window is but not what is in it");
    expect(container.textContent).not.toContain("What Bridge adds");
  });

  it("opens the requested window from outside", async () => {
    vi.spyOn(bridgeApi, "contextWindows").mockResolvedValue(result);
    await mount({ detailRequest: { sessionId: "chat", nonce: 1 } });
    expect(container.querySelector('[aria-label="Context pressure"]')).not.toBeNull();
  });

  it("stops polling and says so when the daemon lacks the method", async () => {
    const fetch = vi.spyOn(bridgeApi, "contextWindows").mockRejectedValue(new Error("method_not_found: sessions/get_context_windows"));
    await mount();
    expect(fetch).toHaveBeenCalledTimes(1);
    expect(container.textContent).toContain("Context is unavailable");
  });

  it("does not fetch while hidden", async () => {
    const fetch = vi.spyOn(bridgeApi, "contextWindows").mockResolvedValue(result);
    await mount({ visible: false });
    expect(fetch).not.toHaveBeenCalled();
    expect(container.textContent).toContain("Reading context windows");
  });
});
