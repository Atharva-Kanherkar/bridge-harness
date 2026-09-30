// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { BrowserCloneSnapshot } from "../types";
import { bridgeApi } from "../api";
import { CLONE_POLL_HIDDEN_MS, CLONE_POLL_VISIBLE_MS, CloneSurface, type CloneSupervision } from "./CloneSurface";

// Contract: testing/feat-dock-clone.md §1.

vi.mock("../api", () => ({
  bridgeApi: {
    browserCloneState: vi.fn(),
    requestClone: vi.fn(),
    readCloneSettings: vi.fn(),
    resolveCloneRequest: vi.fn(),
    cloneInput: vi.fn(),
    takeoverBrowserClone: vi.fn(),
    handBackBrowserClone: vi.fn(),
    destroyBrowserClone: vi.fn(),
    resolveBrowserCloneApproval: vi.fn(),
  },
}));

const state = vi.mocked(bridgeApi.browserCloneState);

const clone = (overrides: Partial<BrowserCloneSnapshot> = {}): BrowserCloneSnapshot => ({
  status: "acting", cloneId: "clone-1", domain: "example.com", signInPath: "import", pendingRequest: null, waitingReason: null,
  expiresAt: new Date(Date.now() + 25 * 60_000).toISOString(),
  screenshot: "data:image/png;base64,AAAA", screenshotRedactedRegions: 0, pendingApproval: null,
  ...overrides,
});
const none = () => clone({ status: "none", cloneId: null, domain: null, signInPath: null, expiresAt: null, screenshot: null });
const requested = () => clone({ status: "requested", cloneId: null, domain: "youtube.com", pendingRequest: "youtube.com", pendingRequestId: "request-1", expiresAt: null, screenshot: null });
const approval = { id: "a1", commandId: "c1", action: "click", domain: "example.com", effect: "Submit a payment form", createdAt: "now" };

let container: HTMLDivElement;
let root: Root;

async function render(props: Partial<Parameters<typeof CloneSurface>[0]> = {}) {
  await act(async () => {
    root.render(<CloneSurface onError={() => undefined} {...props} />);
  });
}

const tick = async (ms: number) => {
  await act(async () => {
    await vi.advanceTimersByTimeAsync(ms);
  });
};
const button = (label: string) => [...container.querySelectorAll("button")].find(node => node.textContent?.trim() === label);
const press = async (label: string) => {
  const target = button(label);
  if (!target) throw new Error(`no "${label}" button in: ${container.textContent}`);
  await act(async () => { target.click(); });
};

beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  vi.useFakeTimers();
  state.mockReset().mockResolvedValue(none());
  vi.mocked(bridgeApi.readCloneSettings).mockReset().mockResolvedValue({ connected: true, settings: { defaultSignInPath: "import", ttlMinutes: 30 } });
  vi.mocked(bridgeApi.requestClone).mockReset().mockResolvedValue(clone());
  vi.mocked(bridgeApi.resolveCloneRequest).mockReset().mockResolvedValue(clone());
  vi.mocked(bridgeApi.cloneInput).mockReset().mockResolvedValue();
  vi.mocked(bridgeApi.takeoverBrowserClone).mockReset().mockResolvedValue();
  vi.mocked(bridgeApi.handBackBrowserClone).mockReset().mockResolvedValue();
  vi.mocked(bridgeApi.destroyBrowserClone).mockReset().mockResolvedValue();
  vi.mocked(bridgeApi.resolveBrowserCloneApproval).mockReset().mockResolvedValue();
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

describe("CloneSurface as a dock tenant", () => {
  it("switches polling to the new chat without displaying the old chat's frame", async () => {
    state.mockImplementation(async (sessionId) => clone({ domain: `${sessionId}.test` }));
    await render({ sessionId: "first" });
    expect(container.textContent).toContain("first.test");
    await render({ sessionId: "second" });
    expect(state).toHaveBeenLastCalledWith("second");
    expect(container.textContent).toContain("second.test");
    expect(container.textContent).not.toContain("first.test");
  });

  it("reports failed takeover input without an unhandled rejection", async () => {
    const onError = vi.fn();
    state.mockResolvedValue(clone({ status: "taken_over" }));
    vi.mocked(bridgeApi.cloneInput).mockRejectedValue(new Error("clone expired"));
    await render({ sessionId: "session-t", onError });
    const img = container.querySelector<HTMLImageElement>("[data-clone-viewport] img")!;
    img.getBoundingClientRect = () => ({ left: 0, top: 0, width: 200, height: 100, right: 200, bottom: 100, x: 0, y: 0, toJSON() {} });
    await act(async () => { img.dispatchEvent(new MouseEvent("click", { bubbles: true, clientX: 100, clientY: 50 })); });
    expect(onError).toHaveBeenCalledWith("clone expired");
  });

  it("fills its host instead of positioning itself", async () => {
    await render();
    const rootNode = container.firstElementChild as HTMLElement;
    expect(rootNode.className).toContain("h-full");
    expect(rootNode.className).not.toContain("flex-[0_0_48%]");
    expect(rootNode.className).not.toContain("border-l");
  });

  it("shows the live view with Take over and Destroy for a running clone", async () => {
    state.mockResolvedValue(clone());
    await render();
    expect(container.querySelector("[data-clone-viewport] img")?.getAttribute("alt")).toBe("Live view of the browser clone");
    expect(button("Take over")).toBeDefined();
    expect(button("Destroy")).toBeDefined();
    expect(container.textContent).toContain("example.com");
    expect(container.textContent).toContain("Acting");
  });

  it("offers a start control, and no takeover/destroy, when no clone is running", async () => {
    await render();
    expect(container.textContent).toContain("throwaway copy of your browser");
    expect(button("Start clone")).toBeDefined();
    expect(button("Take over")).toBeUndefined();
    expect(button("Destroy")).toBeUndefined();
  });

  it("shows an Allow/Deny card when the agent asks for a clone", async () => {
    state.mockResolvedValue(requested());
    await render({ sessionId: "session-9" });
    expect(container.textContent).toContain("The agent wants a browser");
    expect(container.textContent).toContain("youtube.com");
    expect(button("Allow")).toBeDefined();
    expect(button("Deny")).toBeDefined();
  });

  it("allowing the request calls resolveCloneRequest", async () => {
    const resolve = vi.mocked(bridgeApi.resolveCloneRequest);
    resolve.mockResolvedValue(clone());
    state.mockResolvedValue(requested());
    await render({ sessionId: "session-9" });
    await act(async () => { button("Allow")!.click(); });
    expect(resolve).toHaveBeenCalledWith("session-9", true, "request-1", { defaultSignInPath: "import", ttlMinutes: 30, agentVision: true });
  });

  it("forwards a click on the frame to the page while taken over", async () => {
    const cloneInput = vi.mocked(bridgeApi.cloneInput);
    state.mockResolvedValue(clone({ status: "taken_over" }));
    await render({ sessionId: "session-t" });
    const img = container.querySelector<HTMLImageElement>("[data-clone-viewport] img")!;
    img.getBoundingClientRect = () => ({ left: 0, top: 0, width: 200, height: 100, right: 200, bottom: 100, x: 0, y: 0, toJSON() {} });
    await act(async () => { img.dispatchEvent(new MouseEvent("click", { bubbles: true, clientX: 100, clientY: 50 })); });
    expect(cloneInput).toHaveBeenCalledWith("session-t", { kind: "click", x: 0.5, y: 0.5 });
  });

  it("types into the page from the keyboard while taken over, in one burst per word", async () => {
    const cloneInput = vi.mocked(bridgeApi.cloneInput);
    state.mockResolvedValue(clone({ status: "taken_over" }));
    await render({ sessionId: "session-k" });
    const viewport = container.querySelector<HTMLElement>("[data-clone-viewport]")!;
    expect(viewport.tabIndex).toBe(0);
    for (const key of ["h", "u", "n", "t", "e", "r", "2"]) await act(async () => { viewport.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true })); });
    expect(cloneInput).not.toHaveBeenCalled();
    await act(async () => { viewport.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true })); });
    expect(cloneInput.mock.calls).toEqual([["session-k", { kind: "type", text: "hunter2" }], ["session-k", { kind: "key", key: "Enter" }]]);
  });

  it("shows whether the agent can see the page", async () => {
    state.mockResolvedValue(clone({ agentVision: false }));
    await render();
    expect(container.textContent).toContain("Agent reads text only");
    state.mockResolvedValue(clone({ agentVision: true }));
    await tick(CLONE_POLL_VISIBLE_MS);
    expect(container.textContent).toContain("Agent sees the page");
  });

  it("does not forward clicks unless taken over", async () => {
    const cloneInput = vi.mocked(bridgeApi.cloneInput);
    cloneInput.mockClear();
    state.mockResolvedValue(clone({ status: "acting" }));
    await render({ sessionId: "session-t" });
    const img = container.querySelector<HTMLImageElement>("[data-clone-viewport] img")!;
    await act(async () => { img.dispatchEvent(new MouseEvent("click", { bubbles: true, clientX: 10, clientY: 10 })); });
    expect(cloneInput).not.toHaveBeenCalled();
  });

  it("starts a clone for the typed site", async () => {
    const requestClone = vi.mocked(bridgeApi.requestClone);
    requestClone.mockResolvedValue(clone());
    await render({ sessionId: "session-7" });
    const input = container.querySelector<HTMLInputElement>("input[aria-label='Site to clone']")!;
    const setValue = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
    await act(async () => {
      setValue.call(input, "youtube.com");
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await act(async () => {
      button("Start clone")!.click();
    });
    expect(requestClone).toHaveBeenCalledWith("session-7", "youtube.com", "chrome", "import");
  });

  it("polls at the foreground cadence while visible", async () => {
    await render({ visible: true });
    expect(state).toHaveBeenCalledTimes(1);
    await tick(CLONE_POLL_VISIBLE_MS);
    expect(state).toHaveBeenCalledTimes(2);
    await tick(CLONE_POLL_VISIBLE_MS);
    expect(state).toHaveBeenCalledTimes(3);
  });

  it("drops to the background cadence while hidden with a live clone", async () => {
    state.mockResolvedValue(clone());
    await render({ visible: true });
    await tick(0);
    await render({ visible: false });
    const after = state.mock.calls.length;
    await tick(CLONE_POLL_HIDDEN_MS - 1);
    expect(state.mock.calls.length).toBe(after);
    await tick(1);
    expect(state.mock.calls.length).toBe(after + 1);
  });

  it("does not poll while hidden without a clone", async () => {
    await render({ visible: true });
    await tick(0);
    await render({ visible: false });
    const frozen = state.mock.calls.length;
    await tick(CLONE_POLL_HIDDEN_MS * 4);
    expect(state.mock.calls.length).toBe(frozen);
  });

  it("reports supervision upward, marking waiting_for_you and pending approvals", async () => {
    const seen: CloneSupervision[] = [];
    state.mockResolvedValue(clone());
    await render({ onSupervisionChange: supervision => seen.push(supervision) });
    await tick(0);
    expect(seen.at(-1)).toEqual({ status: "acting", attention: false });

    state.mockResolvedValue(clone({ status: "waiting_for_you", waitingReason: "Enter the 2FA code" }));
    await tick(CLONE_POLL_VISIBLE_MS);
    expect(seen.at(-1)).toEqual({ status: "waiting_for_you", attention: true });

    state.mockResolvedValue(clone({ pendingApproval: approval }));
    await tick(CLONE_POLL_VISIBLE_MS);
    expect(seen.at(-1)).toEqual({ status: "acting", attention: true });
  });

  it("names why it is waiting and puts Take over first", async () => {
    state.mockResolvedValue(clone({ status: "waiting_for_you", waitingReason: "Enter the 2FA code" }));
    await render();
    const notice = container.querySelector('[role="status"]');
    expect(notice?.textContent).toContain("waiting for you");
    expect(notice?.textContent).toContain("Enter the 2FA code");
    expect(container.textContent).toContain("Waiting for you");
    expect(button("Take over")).toBeDefined();
  });

  it("walks take over, hand back, and destroy through their states", async () => {
    let current = clone({ status: "waiting_for_you", waitingReason: "Sign in" });
    state.mockImplementation(async () => structuredClone(current));
    vi.mocked(bridgeApi.takeoverBrowserClone).mockImplementation(async () => { current = { ...current, status: "taken_over", waitingReason: null }; });
    vi.mocked(bridgeApi.handBackBrowserClone).mockImplementation(async () => { current = { ...current, status: "acting" }; });
    vi.mocked(bridgeApi.destroyBrowserClone).mockImplementation(async () => { current = { ...none(), status: "destroyed" }; });
    const seen: CloneSupervision[] = [];
    await render({ onSupervisionChange: supervision => seen.push(supervision) });
    expect(seen.at(-1)).toEqual({ status: "waiting_for_you", attention: true });

    await press("Take over");
    expect(bridgeApi.takeoverBrowserClone).toHaveBeenCalledTimes(1);
    expect(container.textContent).toContain("You’re in control");
    expect(button("Take over")).toBeUndefined();
    // The person is at the wheel, so nothing is waiting on them any more.
    expect(seen.at(-1)).toEqual({ status: "taken_over", attention: false });

    await press("Hand back");
    expect(bridgeApi.handBackBrowserClone).toHaveBeenCalledTimes(1);
    expect(seen.at(-1)).toEqual({ status: "acting", attention: false });
    expect(button("Take over")).toBeDefined();

    // Destroy asks first: one click is a question, not the act.
    await press("Destroy");
    expect(bridgeApi.destroyBrowserClone).not.toHaveBeenCalled();
    await press("Cancel");
    expect(button("Destroy")).toBeDefined();
    await press("Destroy");
    await press("Destroy clone");
    expect(bridgeApi.destroyBrowserClone).toHaveBeenCalledTimes(1);
    expect(container.textContent).toContain("Clone destroyed");
    expect(container.querySelector("[data-clone-viewport]")).toBeNull();
    expect(seen.at(-1)).toEqual({ status: "destroyed", attention: false });
  });

  it("stops polling at the hidden cadence once the clone is destroyed", async () => {
    state.mockResolvedValue({ ...none(), status: "destroyed" });
    await render({ visible: true });
    await tick(0);
    await render({ visible: false });
    const frozen = state.mock.calls.length;
    await tick(CLONE_POLL_HIDDEN_MS * 2);
    expect(state.mock.calls.length).toBe(frozen);
  });

  it("holds the pending approval in front of the surface until it is resolved", async () => {
    let current = clone({ pendingApproval: approval });
    state.mockImplementation(async () => structuredClone(current));
    vi.mocked(bridgeApi.resolveBrowserCloneApproval).mockImplementation(async () => { current = { ...current, pendingApproval: null }; });
    await render();
    const region = container.querySelector('[role="region"][aria-label="Sensitive clone action"]');
    expect(region?.textContent).toContain("Submit a payment form");
    await press("Approve once");
    expect(bridgeApi.resolveBrowserCloneApproval).toHaveBeenCalledWith("a1", true);
    expect(container.querySelector('[role="region"]')).toBeNull();
  });

  it("reports a failed action and leaves the clone where it was", async () => {
    const onError = vi.fn();
    state.mockResolvedValue(clone());
    vi.mocked(bridgeApi.takeoverBrowserClone).mockRejectedValue(new Error("clone is gone"));
    await render({ onError });
    await press("Take over");
    expect(onError).toHaveBeenCalledWith("clone is gone");
    expect(container.textContent).toContain("Acting");
  });
});
