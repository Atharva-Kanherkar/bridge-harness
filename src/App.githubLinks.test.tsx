// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const open = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/plugin-shell", () => ({ open }));

import { App } from "./App";
import { bridgeApi } from "./api";
import { installExternalLinkHandler } from "./externalLinks";

// A GitHub link is only worth routing if the whole path works — the document
// interceptor, the router App registers, the repository the workspace
// resolves to, and the dock. So this exercises the real App against the api
// layer's mock backend and clicks a real anchor, rather than reaching for the
// router directly.

class MockResizeObserver {
  observe() {}
  unobserve() {}
  disconnect() {}
}

let container: HTMLDivElement;
let root: Root;

const settle = async (rounds = 3) => {
  for (let i = 0; i < rounds; i++) {
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 0)); });
  }
};

beforeEach(async () => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  await bridgeApi.resetModelProfiles();
  vi.stubGlobal("ResizeObserver", MockResizeObserver);
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
  installExternalLinkHandler();
  open.mockClear();
  container = document.createElement("div");
  document.body.append(container);
  await act(async () => { root = createRoot(container); root.render(<App />); });
  await settle();
  const recommended = [...container.querySelectorAll("button")].find(button => button.textContent === "Use recommended defaults");
  if (recommended) { await act(async () => recommended.click()); await settle(); }
});

afterEach(() => {
  act(() => root?.unmount());
  container?.remove();
  document.querySelectorAll("a[data-test-link]").forEach(anchor => anchor.remove());
  vi.restoreAllMocks();
});

const chatRows = () => [...container.querySelectorAll<HTMLButtonElement>('button[title*=" — "]')];
const click = async (element: Element) => {
  await act(async () => { element.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, button: 0 })); });
  await settle(2);
};

/** Click a link the way a reader would: an anchor in the document, left button. */
async function clickLink(href: string) {
  const anchor = document.createElement("a");
  anchor.href = href;
  anchor.setAttribute("data-test-link", "");
  anchor.textContent = href;
  document.body.append(anchor);
  await click(anchor);
}

/** Land on a chat that has a worktree — the only place the GitHub pane exists. */
async function openWorkspaceSession() {
  for (const row of chatRows()) {
    await click(row);
    if (container.textContent?.includes("4 files")) return;
  }
  throw new Error("no workspace chat opened");
}

const activePane = () => container.querySelector<HTMLElement>('aside[aria-label="Dock"] button[role="tab"][aria-selected="true"]')?.getAttribute("aria-label");

// The mock backend resolves every workspace to this repository.
const repo = "https://github.com/Atharva-Kanherkar/bridge-harness";

describe("clicking a GitHub link", () => {
  it("opens the pull request in the dock instead of the browser", async () => {
    await openWorkspaceSession();

    await clickLink(`${repo}/pull/42`);

    expect(activePane()).toBe("GitHub");
    expect(open).not.toHaveBeenCalled();
  });

  it("still leaves for a repository this workspace is not on", async () => {
    await openWorkspaceSession();

    await clickLink("https://github.com/someone/else/pull/42");

    expect(activePane()).not.toBe("GitHub");
    expect(open).toHaveBeenCalledWith("https://github.com/someone/else/pull/42");
  });

  it("still leaves for a GitHub page the pane has no view for", async () => {
    await openWorkspaceSession();

    await clickLink(`${repo}/commit/9f8e7d6`);

    expect(activePane()).not.toBe("GitHub");
    expect(open).toHaveBeenCalledWith(`${repo}/commit/9f8e7d6`);
  });

  it("still leaves from a chat with no worktree to route into", async () => {
    // The default landing view has no workspace session selected.
    await clickLink(`${repo}/pull/42`);

    expect(activePane()).not.toBe("GitHub");
    expect(open).toHaveBeenCalledWith(`${repo}/pull/42`);
  });

  it("asks which repository the workspace is on once, not once per link", async () => {
    await openWorkspaceSession();
    const status = vi.spyOn(bridgeApi, "githubStatus");

    await clickLink(`${repo}/pull/42`);
    // The pane reads the status for itself when it mounts; what matters is
    // that the second link costs nothing.
    const settled = status.mock.calls.length;
    await clickLink(`${repo}/issues/7`);

    expect(activePane()).toBe("GitHub");
    expect(status.mock.calls.length).toBe(settled);
  });
});
