// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { QUICK_HIDE_EVENT, QUICK_SUBMIT_EVENT, QuickPrompt } from "./QuickPrompt";

let host: HTMLDivElement;
let root: Root;
beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(() => { act(() => root.unmount()); host.remove(); });

const type = (textarea: HTMLTextAreaElement, value: string) => {
  const setter = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!;
  setter.call(textarea, value);
  textarea.dispatchEvent(new Event("input", { bubbles: true }));
};
const key = (textarea: HTMLTextAreaElement, init: KeyboardEventInit) => textarea.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, ...init }));

it("hands trimmed text to the main window on Enter and clears", async () => {
  const emit = vi.fn(async () => {});
  await act(async () => { root.render(<QuickPrompt emit={emit} />); });
  const textarea = host.querySelector("textarea")!;
  await act(async () => { type(textarea, "  fix the flaky test  "); });
  await act(async () => { key(textarea, { key: "Enter" }); });
  expect(emit).toHaveBeenCalledWith(QUICK_SUBMIT_EVENT, { text: "fix the flaky test" });
  expect(textarea.value).toBe("");
});

it("keeps Shift+Enter as a new line and never sends an empty prompt", async () => {
  const emit = vi.fn(async () => {});
  await act(async () => { root.render(<QuickPrompt emit={emit} />); });
  const textarea = host.querySelector("textarea")!;
  await act(async () => { key(textarea, { key: "Enter" }); });
  await act(async () => { type(textarea, "line one"); key(textarea, { key: "Enter", shiftKey: true }); });
  expect(emit).not.toHaveBeenCalled();
});

it("hides on Escape without sending", async () => {
  const emit = vi.fn(async () => {});
  await act(async () => { root.render(<QuickPrompt emit={emit} />); });
  const textarea = host.querySelector("textarea")!;
  await act(async () => { type(textarea, "draft"); key(textarea, { key: "Escape" }); });
  expect(emit).toHaveBeenCalledWith(QUICK_HIDE_EVENT);
  expect(emit).not.toHaveBeenCalledWith(QUICK_SUBMIT_EVENT, expect.anything());
});
