// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Workspace } from "../types";
import { NewChatDialog, type NewChatDialogProps } from "./NewChatDialog";

const workspace = (id: string, overrides: Partial<Workspace> = {}): Workspace => ({
  id,
  title: id,
  path: `/Users/atharva/Documents/${id}`,
  projectId: `project-${id}`,
  branch: "main",
  dirtyFiles: 0,
  additions: 0,
  deletions: 0,
  status: "ready",
  createdAt: "2026-08-01T00:00:00Z",
  ...overrides,
} as Workspace);

const noop = () => {};

const props = (overrides: Partial<NewChatDialogProps> = {}): NewChatDialogProps => ({
  open: true,
  workspaces: [workspace("harness"), workspace("agentclash")],
  busy: false,
  onClose: noop,
  onStart: noop,
  ...overrides,
});

let container: HTMLDivElement;
let root: Root;

function mount(overrides: Partial<NewChatDialogProps> = {}) {
  act(() => {
    root.render(<NewChatDialog {...props(overrides)} />);
  });
}

const options = () => [...document.querySelectorAll<HTMLButtonElement>('[role="radio"]')];
const optionByText = (needle: string) => options().find(option => option.textContent?.includes(needle))!;
const worktreeBox = () => document.querySelector<HTMLInputElement>('input[type="checkbox"]');
const startButton = () =>
  [...document.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent?.startsWith("Start"))!;
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

describe("NewChatDialog", () => {
  it("renders nothing while closed", () => {
    mount({ open: false });
    expect(options()).toHaveLength(0);
  });

  it("offers No project plus every project, with No project chosen by default", () => {
    mount();
    // Each row's first span is its name; the second carries the branch summary.
    expect(options().map(option => option.querySelector("span span")?.textContent)).toEqual(["No project", "harness", "agentclash"]);
    expect(optionByText("No project").getAttribute("aria-checked")).toBe("true");
  });

  it("starts a plain chat when no project is chosen", () => {
    const onStart = vi.fn();
    mount({ onStart });
    click(startButton());
    expect(onStart).toHaveBeenCalledWith({ workspaceId: null, worktree: false });
  });

  it("asks about a worktree only once a project is chosen", () => {
    mount();
    expect(worktreeBox()).toBeNull();
    click(optionByText("harness"));
    expect(worktreeBox()).not.toBeNull();
  });

  it("reports the project and the worktree choice", () => {
    const onStart = vi.fn();
    mount({ onStart });
    click(optionByText("agentclash"));
    // A real click is what drives a controlled checkbox; assigning .checked is
    // invisible to React.
    click(worktreeBox()!);
    click(startButton());
    expect(onStart).toHaveBeenCalledWith({ workspaceId: "agentclash", worktree: true });
  });

  it("names the project on the confirm button", () => {
    mount();
    expect(startButton().textContent).toContain("Start chat");
    click(optionByText("harness"));
    expect(startButton().textContent).toContain("Start in harness");
  });

  it("refuses a worktree for a project with no repository behind it", () => {
    const onStart = vi.fn();
    mount({ workspaces: [workspace("folder-only", { projectId: null })], onStart });
    click(optionByText("folder-only"));
    expect(worktreeBox()!.disabled).toBe(true);
    expect(document.body.textContent).toContain("Connect a Git repository");
    click(startButton());
    expect(onStart).toHaveBeenCalledWith({ workspaceId: "folder-only", worktree: false });
  });

  it("preselects the project it was opened from", () => {
    mount({ initialWorkspaceId: "agentclash" });
    expect(optionByText("agentclash").getAttribute("aria-checked")).toBe("true");
    expect(optionByText("No project").getAttribute("aria-checked")).toBe("false");
  });

  it("forgets the previous answer between visits", () => {
    mount({ initialWorkspaceId: null });
    click(optionByText("harness"));
    expect(optionByText("harness").getAttribute("aria-checked")).toBe("true");
    mount({ open: false });
    mount({ open: true, initialWorkspaceId: null });
    expect(optionByText("No project").getAttribute("aria-checked")).toBe("true");
  });

  it("closes on Escape", () => {
    const onClose = vi.fn();
    mount({ onClose });
    act(() => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    });
    expect(onClose).toHaveBeenCalled();
  });

  it("holds the confirm while a create is in flight", () => {
    mount({ busy: true });
    expect(startButton().disabled).toBe(true);
    expect(startButton().textContent).toContain("Starting…");
  });
});
