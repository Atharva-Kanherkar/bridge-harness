// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it, vi } from "vitest";
import { OrchestratorCreateDialog } from "./OrchestratorCreateDialog";

describe("OrchestratorCreateDialog", () => {
  it("makes isolated worktrees the primary choice", async () => {
    (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    const onCreateWorktree = vi.fn();
    const onUseCurrentFolder = vi.fn();
    const container = document.createElement("div");
    const root = createRoot(container);
    await act(async () => root.render(
      <OrchestratorCreateDialog
        open
        workspaceTitle="Payments"
        canCreateWorktree
        onCreateWorktree={onCreateWorktree}
        onUseCurrentFolder={onUseCurrentFolder}
        onClose={vi.fn()}
      />,
    ));

    expect(container.textContent).toContain("Create an isolated worktree?");
    const primary = [...container.querySelectorAll("button")].find(button => button.textContent?.includes("Create worktree"))!;
    await act(async () => primary.click());
    expect(onCreateWorktree).toHaveBeenCalledOnce();
    expect(onUseCurrentFolder).not.toHaveBeenCalled();
    await act(async () => root.unmount());
  });

  it("disables worktree creation until a Git repository is connected", async () => {
    const onClose = vi.fn();
    const container = document.createElement("div");
    const root = createRoot(container);
    await act(async () => root.render(
      <OrchestratorCreateDialog open workspaceTitle="Scratch" canCreateWorktree={false} onCreateWorktree={vi.fn()} onUseCurrentFolder={vi.fn()} onClose={onClose} />,
    ));

    expect(container.textContent).toContain("Connect a Git repository");
    expect([...container.querySelectorAll("button")].find(button => button.textContent?.includes("Create worktree"))?.disabled).toBe(true);
    const close = container.querySelector<HTMLButtonElement>('button[aria-label="Cancel new orchestrator"]')!;
    await act(async () => close.click());
    expect(onClose).toHaveBeenCalledOnce();
    await act(async () => root.unmount());
  });
});
