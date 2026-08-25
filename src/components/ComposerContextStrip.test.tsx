// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Workspace } from "../types";
import { ComposerContextStrip } from "./ComposerContextStrip";

const workspace = (overrides: Partial<Workspace> = {}): Workspace => ({
  id: "ws-1",
  title: "bridge-harness",
  branch: "feat/cursor-sidebar-dev",
  projectId: "proj-1",
  status: "ready",
  dirtyFiles: 0,
  ...overrides,
} as Workspace);

let container: HTMLDivElement;
let root: Root;

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

function mount(overrides: Partial<Parameters<typeof ComposerContextStrip>[0]> = {}) {
  const onSelectWorkspace = vi.fn();
  const onToggleWorktree = vi.fn();
  const onRequestBranches = vi.fn();
  const onSelectBranch = vi.fn();
  act(() => {
    root.render(
      <ComposerContextStrip
        workspaces={[workspace(), workspace({ id: "ws-2", title: "travel-assistance", branch: "main" })]}
        workspace={workspace()}
        worktree={false}
        locked={false}
        branches={["feat/cursor-sidebar-dev", "main"]}
        onSelectWorkspace={onSelectWorkspace}
        onRequestBranches={onRequestBranches}
        onSelectBranch={onSelectBranch}
        onToggleWorktree={onToggleWorktree}
        {...overrides}
      />,
    );
  });
  return { onSelectWorkspace, onToggleWorktree, onRequestBranches, onSelectBranch };
}

describe("ComposerContextStrip", () => {
  it("shows repo, branch, worktree, and This Mac", () => {
    mount();
    const text = container.textContent ?? "";
    expect(text).toContain("bridge-harness");
    expect(text).toContain("feat/cursor-sidebar-dev");
    expect(text).toContain("On branch");
    expect(text).toContain("This Mac");
    expect(container.querySelector('[aria-label="Chat context"]')).toBeTruthy();
  });

  it("keeps Cloud and SSH visible but disabled", () => {
    mount();
    act(() => {
      [...container.querySelectorAll("button")].find(button => button.textContent?.includes("This Mac"))!.click();
    });
    const menu = document.querySelector('[role="menu"][aria-label="Agent host"]')!;
    const cloud = [...menu.querySelectorAll("button")].find(button => button.textContent?.includes("Cloud"))!;
    const ssh = [...menu.querySelectorAll("button")].find(button => button.textContent?.includes("SSH"))!;
    expect(cloud.disabled).toBe(true);
    expect(ssh.disabled).toBe(true);
    expect(menu.textContent).toContain("Not wired up yet");
  });

  it("locks the repo menu and worktree chip after the first turn", () => {
    const { onToggleWorktree, onSelectWorkspace } = mount({ locked: true });
    const repo = [...container.querySelectorAll("button")].find(button => button.textContent?.includes("bridge-harness"))!;
    expect(repo.disabled).toBe(true);
    const branch = [...container.querySelectorAll("button")].find(button => button.textContent?.includes("feat/cursor-sidebar-dev"))!;
    expect(branch.disabled).toBe(true);
    expect(container.querySelector('button[aria-pressed]')).toBeNull();
    expect(onToggleWorktree).not.toHaveBeenCalled();
    expect(onSelectWorkspace).not.toHaveBeenCalled();
  });

  it("toggles worktree while unlocked and a repo can isolate", () => {
    const { onToggleWorktree } = mount();
    act(() => {
      [...container.querySelectorAll("button")].find(button => button.textContent?.includes("On branch"))!.click();
    });
    expect(onToggleWorktree).toHaveBeenCalledOnce();
  });

  it("loads local branches and switches to a selected branch", () => {
    const { onRequestBranches, onSelectBranch } = mount();
    const trigger = [...container.querySelectorAll<HTMLButtonElement>("button")]
      .find(button => button.textContent?.includes("feat/cursor-sidebar-dev"))!;
    act(() => trigger.click());
    expect(onRequestBranches).toHaveBeenCalledOnce();
    const menu = document.querySelector('[role="menu"][aria-label="Branch"]')!;
    const main = [...menu.querySelectorAll<HTMLButtonElement>("button")]
      .find(button => button.textContent?.includes("main"))!;
    act(() => main.click());
    expect(onSelectBranch).toHaveBeenCalledWith("main");
  });

  it("moves focus through menu items with the keyboard and restores the trigger", async () => {
    mount();
    const trigger = [...container.querySelectorAll<HTMLButtonElement>("button")]
      .find(button => button.textContent?.includes("bridge-harness"))!;
    act(() => trigger.click());
    await act(async () => new Promise(resolve => setTimeout(resolve, 20)));

    const menu = document.querySelector<HTMLElement>('[role="menu"][aria-label="Repository"]')!;
    const items = [...menu.querySelectorAll<HTMLButtonElement>('[role="menuitemradio"]')];
    expect(document.activeElement).toBe(items[0]);

    act(() => {
      items[0].dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true }));
    });
    expect(document.activeElement).toBe(items[1]);

    act(() => {
      items[1].dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    });
    await act(async () => new Promise(resolve => setTimeout(resolve, 20)));
    expect(document.querySelector('[role="menu"][aria-label="Repository"]')).toBeNull();
    expect(document.activeElement).toBe(trigger);
  });

  it("focuses branches that arrive after the menu opens", async () => {
    mount({ branches: [] });
    const trigger = [...container.querySelectorAll<HTMLButtonElement>("button")]
      .find(button => button.textContent?.includes("feat/cursor-sidebar-dev"))!;
    act(() => trigger.click());
    mount({ branches: [], branchBusy: true });
    await act(async () => new Promise(resolve => setTimeout(resolve, 20)));

    mount({ branches: ["feat/cursor-sidebar-dev", "main"], branchBusy: false });
    await act(async () => new Promise(resolve => setTimeout(resolve, 20)));
    const firstBranch = document.querySelector<HTMLButtonElement>(
      '[role="menu"][aria-label="Branch"] [role="menuitemradio"]',
    );
    expect(document.activeElement).toBe(firstBranch);
  });

  it("restores the branch trigger when loading is dismissed with Escape", async () => {
    mount({ branches: [] });
    const trigger = [...container.querySelectorAll<HTMLButtonElement>("button")]
      .find(button => button.textContent?.includes("feat/cursor-sidebar-dev"))!;
    act(() => {
      trigger.focus();
      trigger.click();
    });
    mount({ branches: [], branchBusy: true });
    act(() => {
      document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    });
    await act(async () => new Promise(resolve => setTimeout(resolve, 20)));
    expect(document.querySelector('[role="menu"][aria-label="Branch"]')).toBeNull();
    expect(document.activeElement).toBe(trigger);
  });

  it("does not switch the workspace root while using an isolated worktree", () => {
    const { onRequestBranches } = mount({ worktree: true });
    const branch = [...container.querySelectorAll<HTMLButtonElement>("button")]
      .find(button => button.textContent?.includes("feat/cursor-sidebar-dev"))!;
    expect(branch.disabled).toBe(true);
    act(() => branch.click());
    expect(onRequestBranches).not.toHaveBeenCalled();
  });
});
