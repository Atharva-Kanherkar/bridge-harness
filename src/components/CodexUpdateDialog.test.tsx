// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { CodexUpdateDialog } from "./CodexUpdateDialog";

describe("Codex update dialog", () => {
  let host: HTMLDivElement;
  let root: Root;
  beforeEach(() => {
    (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    host = document.createElement("div");
    document.body.append(host);
    root = createRoot(host);
  });
  afterEach(async () => {
    await act(async () => root.unmount());
    host.remove();
  });

  it.each(["installing", "refreshing"] as const)("can close during %s without starting another installer", async phase => {
    const onOpenChange = vi.fn();
    const onConfirm = vi.fn();
    await act(async () => root.render(<CodexUpdateDialog open phase={phase} onOpenChange={onOpenChange} onConfirm={onConfirm} />));
    const dialog = document.querySelector('[role="dialog"]')!;
    expect(dialog.querySelector('[role="status"]')?.textContent).toContain(phase === "installing" ? "Downloading and installing" : "Installation finished");
    const close = [...dialog.querySelectorAll("button")].find(button => button.textContent === "Close")!;
    expect(close.disabled).toBe(false);
    await act(async () => close.click());
    expect(onOpenChange).toHaveBeenCalledWith(false);
    expect(onConfirm).not.toHaveBeenCalled();
  });

  it("waits for confirmation before running the installer", async () => {
    const onConfirm = vi.fn();
    await act(async () => root.render(<CodexUpdateDialog open phase={null} onOpenChange={() => {}} onConfirm={onConfirm} />));
    expect(onConfirm).not.toHaveBeenCalled();
    const yes = [...document.querySelectorAll("button")].find(button => button.textContent === "Yes")!;
    await act(async () => yes.click());
    expect(onConfirm).toHaveBeenCalledOnce();
  });
});
