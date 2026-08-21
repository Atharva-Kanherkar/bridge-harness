// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it, vi } from "vitest";
import { SteerComposer } from "./WorkerDetail";

/// Set a controlled textarea's value the way a keystroke would.
///
/// React tracks the last value it wrote and skips the change event when a test
/// assigns `.value` directly, so the native setter is what makes the synthetic
/// onChange actually fire.
async function type(box: HTMLTextAreaElement, text: string) {
  const setter = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!;
  await act(async () => {
    setter.call(box, text);
    box.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

async function mount(node: React.ReactElement) {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => root.render(node));
  return { container, unmount: () => act(async () => root.unmount()) };
}

describe("SteerComposer", () => {
  it("sends what the user typed and clears the box", async () => {
    const onSteer = vi.fn().mockResolvedValue(undefined);
    const { container, unmount } = await mount(<SteerComposer sessionId="w1" steerable onSteer={onSteer}/>);
    const box = container.querySelector<HTMLTextAreaElement>("textarea")!;
    await type(box, "  use the existing store  ");
    const submit = container.querySelector<HTMLButtonElement>('button[type="submit"]')!;
    expect(submit.disabled).toBe(false);
    await act(async () => submit.click());
    // Trimmed: leading whitespace is a typo, not guidance.
    expect(onSteer).toHaveBeenCalledWith("w1", "use the existing store");
    expect(container.querySelector<HTMLTextAreaElement>("textarea")!.value).toBe("");
    await unmount();
  });

  it("refuses to send an empty steer", async () => {
    const onSteer = vi.fn().mockResolvedValue(undefined);
    const { container, unmount } = await mount(<SteerComposer sessionId="w1" steerable onSteer={onSteer}/>);
    const submit = container.querySelector<HTMLButtonElement>('button[type="submit"]')!;
    expect(submit.disabled).toBe(true);
    await act(async () => submit.click());
    expect(onSteer).not.toHaveBeenCalled();
    await unmount();
  });

  it("keeps the draft and shows why when the steer is refused", async () => {
    // The backend gate can refuse between render and submit — the worker
    // reported in the meantime. Losing the typed words on top of that would be
    // the second insult.
    const onSteer = vi.fn().mockRejectedValue(new Error("This worker already reported its typed result"));
    const { container, unmount } = await mount(<SteerComposer sessionId="w1" steerable onSteer={onSteer}/>);
    const box = container.querySelector<HTMLTextAreaElement>("textarea")!;
    await type(box, "narrow the scope");
    await act(async () => container.querySelector<HTMLButtonElement>('button[type="submit"]')!.click());
    expect(container.textContent).toContain("already reported its typed result");
    expect(container.querySelector<HTMLTextAreaElement>("textarea")!.value).toBe("narrow the scope");
    await unmount();
  });

  it("explains itself instead of offering a box a worker cannot take", async () => {
    const onSteer = vi.fn();
    const { container, unmount } = await mount(<SteerComposer sessionId="w1" steerable={false} onSteer={onSteer}/>);
    expect(container.querySelector("textarea")).toBeNull();
    expect(container.textContent).toContain("Its typed result is final");
    await unmount();
  });

  it("takes a custom label so each surface can name the action in its own words", async () => {
    const { container, unmount } = await mount(<SteerComposer sessionId="w1" steerable onSteer={async () => {}} label="Redirect this worker…"/>);
    expect(container.querySelector<HTMLTextAreaElement>("textarea")!.placeholder).toBe("Redirect this worker…");
    await unmount();
  });
});
