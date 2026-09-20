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
let store: Map<string, string>;

const settle = async (rounds = 3) => {
  for (let i = 0; i < rounds; i++) {
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 0)); });
  }
};

beforeEach(async () => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  await bridgeApi.resetModelProfiles();
  vi.stubGlobal("ResizeObserver", MockResizeObserver);
  store = new Map<string, string>();
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

const activePane = () => container.querySelector<HTMLElement>('aside[aria-label="Dock"] div[role="tablist"][aria-label="Dock panes"] button[aria-selected="true"]')?.getAttribute("aria-label");
/** The chooser is a dialog, and dialogs portal out of the App container. */
const chooser = () => document.querySelector<HTMLElement>('[aria-label="Open GitHub link"]');
const chooserButton = (label: string) => [...(chooser()?.querySelectorAll("button") ?? [])]
  .find(button => button.textContent?.includes(label)) as HTMLButtonElement | undefined;

// The mock backend resolves every workspace to this repository.
const repo = "https://github.com/Atharva-Kanherkar/bridge-harness";

describe("clicking a GitHub link", () => {
  it("asks where to open a link the pane could render, rather than deciding", async () => {
    await openWorkspaceSession();

    await clickLink(`${repo}/pull/42`);

    // Nothing has opened yet — the question is the whole response to the click.
    expect(chooser(), "no chooser appeared").not.toBeNull();
    expect(chooser()!.textContent).toContain("Pull request #42");
    expect(chooser()!.textContent).toContain(`${repo}/pull/42`);
    expect(activePane()).not.toBe("GitHub");
    expect(open).not.toHaveBeenCalled();
  });

  it("opens in the dock when that is the choice", async () => {
    await openWorkspaceSession();
    await clickLink(`${repo}/pull/42`);

    await click(chooserButton("Open in Bridge")!);

    expect(activePane()).toBe("GitHub");
    expect(chooser()).toBeNull();
    expect(open).not.toHaveBeenCalled();
  });

  it("opens in the browser when that is the choice", async () => {
    await openWorkspaceSession();
    await clickLink(`${repo}/pull/42`);

    await click(chooserButton("Open in browser")!);

    expect(open).toHaveBeenCalledWith(`${repo}/pull/42`);
    expect(chooser()).toBeNull();
    expect(activePane()).not.toBe("GitHub");
  });

  it("still leaves for a repository this workspace is not on", async () => {
    await openWorkspaceSession();

    await clickLink("https://github.com/someone/else/pull/42");

    expect(chooser(), "an unroutable link should not ask").toBeNull();
    expect(activePane()).not.toBe("GitHub");
    expect(open).toHaveBeenCalledWith("https://github.com/someone/else/pull/42");
  });

  it("still leaves for a GitHub page the pane has no view for", async () => {
    await openWorkspaceSession();

    await clickLink(`${repo}/commit/9f8e7d6`);

    expect(chooser(), "an unroutable link should not ask").toBeNull();
    expect(activePane()).not.toBe("GitHub");
    expect(open).toHaveBeenCalledWith(`${repo}/commit/9f8e7d6`);
  });

  it("still leaves from a chat with no worktree to route into", async () => {
    // The default landing view has no workspace session selected.
    await clickLink(`${repo}/pull/42`);

    expect(chooser(), "a chat with no worktree should not ask").toBeNull();
    expect(activePane()).not.toBe("GitHub");
    expect(open).toHaveBeenCalledWith(`${repo}/pull/42`);
  });

  it("persists the pane it opened under the workspace's own dock key", async () => {
    await openWorkspaceSession();

    await clickLink(`${repo}/pull/42`);
    await click(chooserButton("Open in Bridge")!);

    // The dock dispatcher is memoized on the dock key, which is undefined
    // until a session is selected — capturing it once would leave the pane
    // open on screen but unwritten, so it would not come back after a restart.
    expect(activePane()).toBe("GitHub");
    const persisted = [...store.entries()].find(([key]) => key.startsWith("bridge.dock.v1."));
    expect(persisted, "the dock wrote nothing").toBeDefined();
    expect(JSON.parse(persisted![1])).toMatchObject({ open: true, pane: "github" });
  });

  it("re-reads the repository per click, so a remote that has changed stops routing", async () => {
    await openWorkspaceSession();
    await clickLink(`${repo}/pull/42`);
    await click(chooserButton("Open in Bridge")!);
    expect(activePane()).toBe("GitHub");

    // The workspace is moved onto a different repository. The pane resolves
    // its repository server-side at call time, so a remembered identity would
    // route this link into a pane bound to the new repo and open the same
    // number there — a different pull request entirely.
    vi.spyOn(bridgeApi, "githubStatus").mockResolvedValue({
      availability: { status: "available" },
      repository: { host: "github.com", owner: "someone", name: "elsewhere" },
    });
    open.mockClear();

    await clickLink(`${repo}/pull/99`);

    expect(chooser(), "a link to the old repository should not ask").toBeNull();
    expect(open).toHaveBeenCalledWith(`${repo}/pull/99`);
  });

  it("does not open a pull request in a repository that changed while the chooser was open", async () => {
    await openWorkspaceSession();
    await clickLink(`${repo}/pull/42`);
    expect(chooser()).not.toBeNull();

    vi.spyOn(bridgeApi, "githubStatus").mockResolvedValue({
      availability: { status: "available" },
      repository: { host: "github.com", owner: "someone", name: "elsewhere" },
    });

    await click(chooserButton("Open in Bridge")!);

    expect(activePane()).not.toBe("GitHub");
    expect(open).toHaveBeenCalledWith(`${repo}/pull/42`);
  });
});
