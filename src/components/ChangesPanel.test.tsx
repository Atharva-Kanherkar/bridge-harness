// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Workspace } from "../types";
import { ChangesPanel, STATS_REFRESH_DEBOUNCE_MS } from "./ChangesPanel";
import { bridgeApi } from "../api";

// Contract: testing/feat-dock-changes.md §1–§4.

const workspace = (overrides: Partial<Workspace> = {}): Workspace => ({
  id: "demo-1",
  projectId: "demo-project",
  city: "Kyoto",
  title: "Build session supervisor",
  branch: "bridge/session-supervisor",
  path: "/Users/you/bridge/Kyoto",
  status: "working",
  dirtyFiles: 4,
  additions: 284,
  deletions: 31,
  createdAt: new Date().toISOString(),
  ...overrides,
});

let container: HTMLDivElement;
let root: Root;

async function mount(element: React.ReactElement) {
  await act(async () => root.render(element));
  await act(async () => {
    await new Promise(resolve => setTimeout(resolve, 0));
  });
}

const click = async (element: Element) => {
  await act(async () => {
    element.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  });
};

const button = (label: string) => container.querySelector<HTMLButtonElement>(`button[aria-label="${label}"]`);
const scrollRoot = () => container.firstElementChild as HTMLElement;

beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
  vi.restoreAllMocks();
});

describe("ChangesPanel in the dock", () => {
  it("renders the review with its rank line, totals, and viewed counter", async () => {
    await mount(<ChangesPanel workspace={workspace()} />);
    expect(container.textContent).toContain("CHANGES");
    expect(container.textContent).toContain("4 files changed");
    expect(container.textContent).toContain("+303");
    expect(container.textContent).toContain("0/4 viewed");
  });

  it("fills its host instead of centering a reading column", async () => {
    await mount(<ChangesPanel workspace={workspace()} />);
    expect(scrollRoot().className).not.toContain("mx-auto");
    expect(scrollRoot().className).not.toContain("max-w-3xl");
    expect(scrollRoot().className).toContain("w-full");
  });

  it("states the diff basis: branch, uncommitted vs HEAD, base commit", async () => {
    await mount(<ChangesPanel workspace={workspace()} />);
    expect(container.textContent).toContain("bridge/session-supervisor");
    expect(container.textContent).toContain("uncommitted vs HEAD");
    expect(container.textContent).toContain("a1b2c3d4");
  });

  it("reloads once, in place, after stats drift settles", async () => {
    const spy = vi.spyOn(bridgeApi, "workspaceChanges");
    await mount(<ChangesPanel workspace={workspace()} />);
    expect(spy).toHaveBeenCalledTimes(1);

    const row = button("Mark src-tauri/bridge-core/src/policy.rs as viewed")!;
    await click(row);
    const expander = [...container.querySelectorAll<HTMLButtonElement>("button[aria-expanded]")][0];
    await click(expander);
    expect(expander.getAttribute("aria-expanded")).toBe("true");
    const rootBefore = scrollRoot();
    rootBefore.scrollTop = 120;

    await mount(<ChangesPanel workspace={workspace({ dirtyFiles: 5, additions: 300 })} />);
    expect(spy).toHaveBeenCalledTimes(1);
    await act(async () => {
      await new Promise(resolve => setTimeout(resolve, STATS_REFRESH_DEBOUNCE_MS + 40));
    });
    expect(spy).toHaveBeenCalledTimes(2);

    expect(scrollRoot()).toBe(rootBefore);
    expect(scrollRoot().scrollTop).toBe(120);
    expect([...container.querySelectorAll("button[aria-expanded]")][0].getAttribute("aria-expanded")).toBe("true");
    expect(button("Mark src-tauri/bridge-core/src/policy.rs as not viewed")).not.toBeNull();
  });

  it("does not refetch for identical stats", async () => {
    const spy = vi.spyOn(bridgeApi, "workspaceChanges");
    await mount(<ChangesPanel workspace={workspace()} />);
    await mount(<ChangesPanel workspace={workspace()} />);
    await act(async () => {
      await new Promise(resolve => setTimeout(resolve, STATS_REFRESH_DEBOUNCE_MS + 40));
    });
    expect(spy).toHaveBeenCalledTimes(1);
  });

  it("offers a file quote action to its callback", async () => {
    const onQuote = vi.fn();
    await mount(<ChangesPanel workspace={workspace()} onQuote={onQuote} />);
    await click(button("Reference src-tauri/bridge-core/src/policy.rs in the composer")!);
    expect(onQuote).toHaveBeenCalledWith("src-tauri/bridge-core/src/policy.rs");
  });

  it("offers a hunk quote with the range from its own header", async () => {
    const onQuote = vi.fn();
    await mount(<ChangesPanel workspace={workspace()} onQuote={onQuote} />);
    const expander = [...container.querySelectorAll<HTMLButtonElement>("button[aria-expanded]")][0];
    await click(expander);
    const hunkQuote = button("Reference lines 10-30 in the composer")!;
    await click(hunkQuote);
    expect(onQuote).toHaveBeenCalledWith("src-tauri/bridge-core/src/policy.rs", { start: 10, end: 30 });
  });

  it("renders no quote or open affordance without the callbacks", async () => {
    await mount(<ChangesPanel workspace={workspace()} />);
    expect(button("Reference src-tauri/bridge-core/src/policy.rs in the composer")).toBeNull();
    expect(button("Open src-tauri/bridge-core/src/policy.rs in the Code pane")).toBeNull();
  });

  it("hands a file to the Code pane through its callback", async () => {
    const onOpenFile = vi.fn();
    await mount(<ChangesPanel workspace={workspace()} onOpenFile={onOpenFile} />);
    await click(button("Open src-tauri/bridge-core/src/policy.rs in the Code pane")!);
    expect(onOpenFile).toHaveBeenCalledWith("src-tauri/bridge-core/src/policy.rs");
  });
});
