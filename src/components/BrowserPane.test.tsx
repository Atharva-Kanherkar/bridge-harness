// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { bridgeApi } from "../api";
import { BrowserPane } from "./BrowserPane";

// Contract: testing/feat-plain-browser-only.md. The throwaway clone is off, so
// the pane is the plain browser and never asks the core about a clone.

vi.mock("../api", () => ({
  bridgeApi: { browserCloneState: vi.fn(), readCloneSettings: vi.fn(), requestClone: vi.fn(), cloneRequests: vi.fn() },
}));

let container: HTMLDivElement;
let root: Root;
const render = () => act(async () => { root.render(<BrowserPane sessionId="s1" />); });

beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  vi.useFakeTimers();
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
  it("is the plain browser, with no clone surface", async () => {
    await render();
    const address = container.querySelector('input[aria-label="Address"]');
    expect(address).not.toBeNull();
    expect(address!.closest(".hidden")).toBeNull();
    expect(container.querySelector("[data-clone-viewport]")).toBeNull();
    expect(container.querySelector('input[aria-label="Site to clone"]')).toBeNull();
  });

  it("offers no throwaway copy", async () => {
    await render();
    expect(container.querySelector('button[aria-label="Open a throwaway copy of a site"]')).toBeNull();
    expect(container.textContent).not.toMatch(/throwaway|clone/i);
  });

  it("never asks the core about a clone", async () => {
    await render();
    await act(async () => { await vi.advanceTimersByTimeAsync(5_000); });
    expect(bridgeApi.browserCloneState).not.toHaveBeenCalled();
    expect(bridgeApi.readCloneSettings).not.toHaveBeenCalled();
    expect(bridgeApi.requestClone).not.toHaveBeenCalled();
  });
});
