// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Session, Workspace } from "../types";
import { ProjectsScreen, shortPath, type ProjectsScreenProps } from "./ProjectsScreen";

const workspace = (id: string, overrides: Partial<Workspace> = {}): Workspace => ({
  id,
  title: id,
  path: `/Users/atharva/Documents/${id}`,
  branch: "main",
  dirtyFiles: 0,
  additions: 0,
  deletions: 0,
  status: "ready",
  createdAt: "2026-08-01T00:00:00Z",
  ...overrides,
} as Workspace);

const session = (id: string, overrides: Partial<Session> = {}): Session => ({
  id,
  workspaceId: "harness",
  harness: "codex",
  label: id,
  title: null,
  model: null,
  status: "idle",
  startedAt: "2026-08-19T10:00:00Z",
  endedAt: null,
  contextPercent: null,
  usagePercent: null,
  metricSource: "reported",
  restorationMode: "fresh",
  continuationFidelity: "native",
  ...overrides,
} as Session);

const noop = () => {};

const props = (overrides: Partial<ProjectsScreenProps> = {}): ProjectsScreenProps => ({
  workspaces: [workspace("harness")],
  chats: [],
  activeSessionId: undefined,
  busy: false,
  onOpenSession: noop,
  onNewWorkspace: noop,
  onNewWorkspaceSession: noop,
  onConnectFolder: noop,
  ...overrides,
});

let container: HTMLDivElement;
let root: Root;

function mount(overrides: Partial<ProjectsScreenProps> = {}) {
  act(() => {
    root.render(<ProjectsScreen {...props(overrides)} />);
  });
  return container;
}

const text = () => container.textContent ?? "";
const cards = () => [...container.querySelectorAll("section")];
const buttonByText = (needle: string) =>
  [...container.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent?.includes(needle));
const click = (element: Element) => {
  act(() => {
    element.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  });
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

describe("ProjectsScreen", () => {
  it("gives every workspace a card with its branch and chat count", () => {
    mount({
      workspaces: [workspace("harness"), workspace("agentclash", { branch: "feat/router" })],
      chats: [session("a"), session("b")],
    });
    expect(cards()).toHaveLength(2);
    expect(text()).toContain("agentclash");
    expect(text()).toContain("feat/router");
    expect(text()).toContain("2 chats");
  });

  it("counts one chat in the singular", () => {
    mount({ chats: [session("a")] });
    expect(text()).toContain("1 chat");
  });

  it("reports the diff when dirty and says clean when not", () => {
    mount({ workspaces: [workspace("harness", { dirtyFiles: 3, additions: 42, deletions: 7 })] });
    expect(text()).toContain("3 changed");
    expect(text()).toContain("+42");
    expect(text()).toContain("−7");
    mount();
    expect(text()).toContain("clean");
  });

  it("lists at most six chats, newest first, and counts the rest", () => {
    // Distinct ascending stamps, so "newest first" is actually exercised.
    mount({
      chats: Array.from({ length: 8 }, (_, index) =>
        session(`c${index}`, { title: `Chat ${index}`, startedAt: `2026-08-19T1${index}:00:00Z` })),
    });
    const titles = [...container.querySelectorAll("section button span:last-child")].map(node => node.textContent);
    expect(titles).toEqual(["Chat 7", "Chat 6", "Chat 5", "Chat 4", "Chat 3", "Chat 2"]);
    expect(text()).toContain("+2 more");
  });

  it("keeps the identifying tail of a long path and the whole path in the tooltip", () => {
    mount({ workspaces: [workspace("harness", { path: "/Users/atharva/Documents/harness" })] });
    const line = [...container.querySelectorAll("p")].find(node => node.textContent?.includes("harness"))!;
    expect(line.textContent).toBe("…/atharva/Documents/harness");
    expect(line.getAttribute("title")).toBe("/Users/atharva/Documents/harness");
    // A leading slash must not end up on the end, which is what clipping with
    // `direction: rtl` did.
    expect(line.textContent!.endsWith("/")).toBe(false);
  });

  it("leaves a short path alone", () => {
    expect(shortPath("/srv/app")).toBe("/srv/app");
    expect(shortPath("/Users/atharva/Documents/harness")).toBe("…/atharva/Documents/harness");
  });

  it("says so when a project has no agents yet", () => {
    mount();
    expect(text()).toContain("No agents here yet");
  });

  it("offers Connect folder only when the workspace has no path", () => {
    mount();
    expect(buttonByText("Connect folder")).toBeUndefined();
    mount({ workspaces: [workspace("harness", { path: null })] });
    expect(buttonByText("Connect folder")).toBeDefined();
  });

  it("shows one empty state, and still the new-project action, with no workspaces", () => {
    mount({ workspaces: [] });
    expect(text()).toContain("No projects yet");
    expect(buttonByText("New project")).toBeDefined();
    expect(cards()).toHaveLength(0);
  });

  it("opens the chat that was pressed", () => {
    const onOpenSession = vi.fn();
    mount({ chats: [session("a", { title: "Token cost report" })], onOpenSession });
    click(buttonByText("Token cost report")!);
    expect(onOpenSession).toHaveBeenCalledWith("a");
  });

  it("asks for a new agent in the project whose card was pressed", () => {
    const onNewWorkspaceSession = vi.fn();
    mount({ workspaces: [workspace("harness"), workspace("agentclash")], onNewWorkspaceSession });
    click([...container.querySelectorAll<HTMLButtonElement>("button")].filter(button => button.textContent?.includes("New agent"))[1]);
    expect(onNewWorkspaceSession).toHaveBeenCalledWith("agentclash");
  });

  it("holds New agent back while a create is in flight", () => {
    mount({ busy: true });
    expect(buttonByText("New agent")!.disabled).toBe(true);
  });

  it("marks the open chat as active", () => {
    mount({ chats: [session("a", { title: "Open one" })], activeSessionId: "a" });
    expect(buttonByText("Open one")!.className).toContain("bg-accent");
  });

  it("keeps every colour on a semantic token", () => {
    mount({
      workspaces: [workspace("harness", { dirtyFiles: 2, additions: 1, deletions: 1 })],
      chats: [session("a", { status: "working" })],
    });
    const html = container.innerHTML;
    expect(html).toContain("bg-card");
    expect(html).toContain("bg-success");
    expect(html).not.toMatch(/bg-\[#|bg-white\/|emerald-|amber-/);
  });
});
