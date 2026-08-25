// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { BrowserBridgeSnapshot } from "../types";
import { bridgeApi } from "../api";
import { BROWSER_POLL_HIDDEN_MS, BROWSER_POLL_VISIBLE_MS, BrowserSurface, type BrowserSupervision } from "./BrowserSurface";

// Contract: testing/feat-dock-browser.md §1.

vi.mock("../api", () => ({
  bridgeApi: {
    browserBridgeState: vi.fn(),
    takeoverBrowser: vi.fn(),
    browserAction: vi.fn(),
    installBrowserNativeHost: vi.fn(),
    detachBrowser: vi.fn(),
    setBrowserPermission: vi.fn(),
    resolveBrowserApproval: vi.fn(),
    configureRemoteBrowser: vi.fn(),
    startRemoteBrowser: vi.fn(),
  },
}));

const state = vi.mocked(bridgeApi.browserBridgeState);

const snapshot = (overrides: Partial<BrowserBridgeSnapshot> = {}): BrowserBridgeSnapshot => ({
  transportConnected: true, extensionId: "ext", extensionPath: "/ext",
  nativeHostInstalled: true, nativeHostManifestPath: "/manifest", tabs: [], lease: null, status: "not_attached",
  captureActive: false, captureError: null,
  screenshot: null, screenshotRedactedRegions: 0, elements: [], viewport: null, promptInjectionSuspected: false,
  tokenAccounting: { snapshots: 0, fullSnapshots: 0, deltaSnapshots: 0, serializedBytes: 0, estimatedInputTokens: 0, screenshotCount: 0 },
  promptInjectionSignals: [], pendingApproval: null, audit: [], debugEvents: [], siteMetrics: [], remoteProvider: null,
  ...overrides,
});

const leased = (overrides: Partial<BrowserBridgeSnapshot> = {}): BrowserBridgeSnapshot => snapshot({
  lease: { id: "lease-1", tabId: 1, domain: "example.com", permission: "read_only", grantedAt: "now", expiresAt: null },
  status: "reading",
  tabs: [{ id: 1, title: "Example", domain: "example.com", url: "https://example.com", attached: true }],
  ...overrides,
} as Partial<BrowserBridgeSnapshot>);

let container: HTMLDivElement;
let root: Root;

async function render(props: Partial<Parameters<typeof BrowserSurface>[0]> = {}) {
  await act(async () => {
    root.render(<BrowserSurface onError={() => undefined} {...props} />);
  });
}

const tick = async (ms: number) => {
  await act(async () => {
    await vi.advanceTimersByTimeAsync(ms);
  });
};

beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  vi.useFakeTimers();
  state.mockReset();
  state.mockResolvedValue(snapshot());
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
  vi.useRealTimers();
  vi.restoreAllMocks();
});

describe("BrowserSurface as a dock tenant", () => {
  it("fills its host instead of positioning itself", async () => {
    await render();
    const rootNode = container.firstElementChild as HTMLElement;
    expect(rootNode.className).toContain("h-full");
    expect(rootNode.className).not.toContain("flex-[0_0_48%]");
    expect(rootNode.className).not.toContain("border-l");
  });

  it("polls at the foreground cadence while visible", async () => {
    await render({ visible: true });
    expect(state).toHaveBeenCalledTimes(1);
    await tick(BROWSER_POLL_VISIBLE_MS);
    expect(state).toHaveBeenCalledTimes(2);
    await tick(BROWSER_POLL_VISIBLE_MS);
    expect(state).toHaveBeenCalledTimes(3);
  });

  it("drops to the background cadence while hidden with a lease", async () => {
    state.mockResolvedValue(leased());
    await render({ visible: true });
    await tick(0);
    await render({ visible: false });
    const after = state.mock.calls.length;
    await tick(BROWSER_POLL_HIDDEN_MS - 1);
    expect(state.mock.calls.length).toBe(after);
    await tick(1);
    expect(state.mock.calls.length).toBe(after + 1);
  });

  it("does not poll while hidden without a lease", async () => {
    await render({ visible: true });
    await tick(0);
    await render({ visible: false });
    const frozen = state.mock.calls.length;
    await tick(BROWSER_POLL_HIDDEN_MS * 4);
    expect(state.mock.calls.length).toBe(frozen);
  });

  it("reports supervision upward, marking waiting_for_you and pending approvals", async () => {
    const seen: BrowserSupervision[] = [];
    state.mockResolvedValue(leased());
    await render({ onSupervisionChange: supervision => seen.push(supervision) });
    await tick(0);
    expect(seen.at(-1)).toEqual({ status: "reading", attention: false });

    state.mockResolvedValue(leased({ status: "waiting_for_you" }));
    await tick(BROWSER_POLL_VISIBLE_MS);
    expect(seen.at(-1)).toEqual({ status: "waiting_for_you", attention: true });

    state.mockResolvedValue(leased({ pendingApproval: { id: "a1", commandId: "c1", action: "click", domain: "example.com", effect: "Submit a form", createdAt: "now" } } as unknown as Partial<BrowserBridgeSnapshot>));
    await tick(BROWSER_POLL_VISIBLE_MS);
    expect(seen.at(-1)).toEqual({ status: "reading", attention: true });
  });
});
