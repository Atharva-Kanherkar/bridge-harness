// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { BrowserCloneSnapshot } from "../types";
import { bridgeApi } from "../api";
import { BrowserPane } from "./BrowserPane";

// Contract: testing/feat-unify-browser-pane.md §2.

vi.mock("../api", () => ({
  bridgeApi: { browserCloneState: vi.fn(), readCloneSettings: vi.fn(), requestClone: vi.fn(), resolveCloneRequest: vi.fn(), cloneInput: vi.fn(), takeoverBrowserClone: vi.fn(), handBackBrowserClone: vi.fn(), destroyBrowserClone: vi.fn(), resolveBrowserCloneApproval: vi.fn() },
}));

const state = vi.mocked(bridgeApi.browserCloneState);
const clone = (overrides: Partial<BrowserCloneSnapshot> = {}): BrowserCloneSnapshot => ({
  status: "acting", cloneId: "clone-1", domain: "github.com", signInPath: "import", pendingRequest: null, waitingReason: null,
  expiresAt: new Date(Date.now() + 25 * 60_000).toISOString(), screenshot: "data:image/png;base64,AAAA", screenshotRedactedRegions: 0, pendingApproval: null,
  ...overrides,
});
const none = () => clone({ status: "none", cloneId: null, domain: null, signInPath: null, expiresAt: null, screenshot: null });

let container: HTMLDivElement;
let root: Root;
const render = (onSupervisionChange?: () => void) => act(async () => { root.render(<BrowserPane sessionId="s1" onError={() => undefined} onSupervisionChange={onSupervisionChange} />); });
const hidden = (node: Element | null) => !!node?.closest(".hidden");
const address = () => container.querySelector('input[aria-label="Address"]');
const viewport = () => container.querySelector("[data-clone-viewport]");

beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  vi.useFakeTimers();
  state.mockReset().mockResolvedValue(none());
  vi.mocked(bridgeApi.readCloneSettings).mockReset().mockResolvedValue({ connected: true, settings: { defaultSignInPath: "import", ttlMinutes: 30 } });
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

describe("BrowserPane", () => {
  it("is your own browser when there is no clone", async () => {
    await render();
    expect(hidden(address())).toBe(false);
    expect(viewport()).toBeNull();
  });

  it("shows the throwaway copy in the same pane once the agent has one, and returns when it is gone", async () => {
    await render();
    state.mockResolvedValue(clone());
    await act(async () => { await vi.advanceTimersByTimeAsync(800); });
    expect(hidden(viewport())).toBe(false);
    expect(hidden(address())).toBe(true);

    state.mockResolvedValue(none());
    await act(async () => { await vi.advanceTimersByTimeAsync(800); });
    expect(hidden(address())).toBe(false);
  });

  it("lets you open a throwaway copy yourself and step back out", async () => {
    await render();
    await act(async () => { container.querySelector<HTMLButtonElement>('button[aria-label="Open a throwaway copy of a site"]')!.click(); });
    expect(container.querySelector('input[aria-label="Site to clone"]')).not.toBeNull();
    expect(hidden(address())).toBe(true);
    await act(async () => { [...container.querySelectorAll("button")].find(node => node.textContent === "Back to browser")!.click(); });
    expect(hidden(address())).toBe(false);
  });

  it("reports the clone's supervision upward even while it is hidden", async () => {
    const onSupervisionChange = vi.fn();
    state.mockResolvedValue(clone({ status: "waiting_for_you" }));
    await render(onSupervisionChange);
    expect(onSupervisionChange).toHaveBeenCalledWith({ status: "waiting_for_you", attention: true });
  });
});
