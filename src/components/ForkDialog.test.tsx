// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it, vi } from "vitest";
import { ForkDialog } from "./ForkDialog";

type Container = HTMLDivElement & { textContent: string };

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

async function renderFork(props: Partial<Parameters<typeof ForkDialog>[0]> = {}) {
  const container = document.createElement("div");
  const root = createRoot(container);
  const onFork = vi.fn();
  const onClose = vi.fn();
  const base = {
    open: true,
    sessionLabel: "Orchestrator",
    sessionId: "session-1",
    entryId: "entry-5a",
    busy: false,
    error: null as string | null,
    onFork,
    onClose,
    ...props,
  };
  await act(async () => root.render(<ForkDialog {...base} />));
  const buttons = (label: string) => [...container.querySelectorAll("button")].find(button => button.textContent?.includes(label));
  const click = (label: string) => act(async () => buttons(label)!.click());
  return { container, onFork, onClose, buttons, click };
}

describe("ForkDialog", () => {
  it("pre-seeds the fork point and defaults to the shared worktree", async () => {
    const { container, onFork, click } = await renderFork();
    expect(container.textContent).toContain("New branch of Orchestrator from this message");
    expect((container.querySelector<HTMLInputElement>('input[aria-label="Fork title"]'))!.value).toBe("");
    await click("Create fork");
    expect(onFork).toHaveBeenCalledWith(null, "shared");
  });

  it("submits the title and the new-worktree policy and shows the consequence", async () => {
    const { container, onFork, click } = await renderFork();
    const input = container.querySelector<HTMLInputElement>('input[aria-label="Fork title"]')!;
    await act(async () => {
      const nativeSet = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
      nativeSet.call(input, "Agentic tangent");
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await act(async () => {
      const radios = container.querySelectorAll('input[name="fork-worktree"]');
      (radios[1] as HTMLInputElement).click();
    });
    expect(container.textContent).toContain("A Git worktree on a new");
    await click("Create fork");
    expect(onFork).toHaveBeenCalledWith("Agentic tangent", "new");
  });

  it("states the shared-worktree hazard", async () => {
    const { container } = await renderFork();
    expect(container.textContent).toContain("Two sessions writing the same files");
  });

  it("keeps the dialog open and shows the error when the fork is rejected", async () => {
    const { container, onClose, click } = await renderFork({ error: "Worker sessions cannot be forked" });
    expect(container.textContent).toContain("Worker sessions cannot be forked");
    await click("Cancel");
    expect(onClose).toHaveBeenCalledOnce();
  });

  it("disables submit while busy", async () => {
    const { container } = await renderFork({ busy: true });
    expect([...container.querySelectorAll("button")].find(button => button.textContent?.includes("Forking…"))?.disabled).toBe(true);
  });
});