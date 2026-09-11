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
  it("shows repo, branch, worktree, and This computer", () => {
    mount();
    const text = container.textContent ?? "";
    expect(text).toContain("bridge-harness");
    expect(text).toContain("feat/cursor-sidebar-dev");
    expect(text).toContain("Work on branch");
    expect(text).toContain("This computer");
    expect(container.querySelector('[aria-label="Chat context"]')).toBeTruthy();
  });

  it("uses the same type size on every context chip", () => {
    mount();
    const buttons = [...container.querySelector('[aria-label="Chat context"]')!.querySelectorAll("button")];
    expect(buttons).toHaveLength(4);
    expect(buttons.every(button => button.className.includes("text-[12px]"))).toBe(true);
  });

  it("explains host availability without offering an unsupported switch", () => {
    mount();
    const host = container.querySelector<HTMLButtonElement>('[aria-label="Agent host: This computer"]')!;
    act(() => host.click());
    const menu = document.querySelector('[role="menu"][aria-label="Agent host"]')!;
    expect(menu.textContent).toContain("Cloud and SSH hosts are not available yet");
    const options = [...menu.querySelectorAll<HTMLButtonElement>('[role="menuitemradio"]')];
    expect(options.filter(option => !option.disabled).map(option => option.textContent)).toEqual(["This computer"]);
    act(() => options[0].click());
    expect(document.querySelector('[role="menu"][aria-label="Agent host"]')).toBeNull();
  });

  it("opens locked context menus and offers new settings without retargeting the chat", () => {
    const onNewChat = vi.fn();
    const { onToggleWorktree, onSelectWorkspace, onSelectBranch, onRequestBranches } = mount({ locked: true, onNewChat });
    for (const [text, label] of [["bridge-harness", "Repository"], ["feat/cursor-sidebar-dev", "Branch"], ["Work on branch", "Work mode"]]) {
      const trigger = [...container.querySelectorAll("button")].find(button => button.textContent?.includes(text))!;
      expect(trigger.disabled).toBe(false);
      act(() => trigger.click());
      const menu = document.querySelector(`[role="menu"][aria-label="${label}"]`)!;
      expect(menu).toBeTruthy();
      const choices = [...menu.querySelectorAll<HTMLButtonElement>('[role="menuitemradio"]')];
      expect(choices.every(choice => choice.disabled)).toBe(true);
      act(() => choices.forEach(choice => choice.click()));
      const newChat = menu.querySelector<HTMLButtonElement>('[role="menuitem"]')!;
      act(() => newChat.click());
      expect(document.querySelector(`[role="menu"][aria-label="${label}"]`)).toBeNull();
    }
    expect(onNewChat).toHaveBeenCalledTimes(3);
    expect(onToggleWorktree).not.toHaveBeenCalled();
    expect(onSelectWorkspace).not.toHaveBeenCalled();
    expect(onSelectBranch).not.toHaveBeenCalled();
    expect(onRequestBranches).not.toHaveBeenCalled();
  });

  it("changes work mode only when a different option is selected", () => {
    const { onToggleWorktree } = mount();
    const trigger = container.querySelector<HTMLButtonElement>('[aria-label="Work mode: Work on branch"]')!;
    act(() => trigger.click());
    let choices = [...document.querySelectorAll<HTMLButtonElement>('[role="menu"][aria-label="Work mode"] [role="menuitemradio"]')];
    act(() => choices[0].click());
    expect(onToggleWorktree).not.toHaveBeenCalled();
    act(() => trigger.click());
    choices = [...document.querySelectorAll<HTMLButtonElement>('[role="menu"][aria-label="Work mode"] [role="menuitemradio"]')];
    act(() => {
      choices[1].click();
    });
    expect(onToggleWorktree).toHaveBeenCalledOnce();
  });

  it("checks Git HEAD rather than a stale stored branch name", () => {
    const { onSelectBranch } = mount({
      workspace: workspace({ branch: "feat/cursor-sidebar-dev" }),
      currentBranch: "main",
      branches: ["feat/cursor-sidebar-dev", "main"],
    });
    const trigger = [...container.querySelectorAll<HTMLButtonElement>("button")]
      .find(button => button.textContent?.includes("main"))!;
    act(() => trigger.click());
    const menu = document.querySelector('[role="menu"][aria-label="Branch"]')!;
    const current = [...menu.querySelectorAll<HTMLButtonElement>("button")]
      .find(button => button.textContent === "main" || button.textContent?.includes("main"))!;
    act(() => current.click());
    expect(onSelectBranch).not.toHaveBeenCalled();
    act(() => trigger.click());
    const menuAgain = document.querySelector('[role="menu"][aria-label="Branch"]')!;
    const previous = [...menuAgain.querySelectorAll<HTMLButtonElement>("button")]
      .find(button => button.textContent?.includes("feat/cursor-sidebar-dev"))!;
    act(() => previous.click());
    expect(onSelectBranch).toHaveBeenCalledWith("feat/cursor-sidebar-dev");
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
    expect(branch.disabled).toBe(false);
    act(() => branch.click());
    expect(document.querySelector('[role="menu"][aria-label="Branch"]')?.textContent).toContain("isolated worktree");
    expect(onRequestBranches).not.toHaveBeenCalled();
  });
});
