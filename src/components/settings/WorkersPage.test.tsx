// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { bridgeApi } from "../../api";
import { WorkersPage } from "./WorkersPage";

let host: HTMLDivElement;
let root: Root;
beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(() => { act(() => root.unmount()); host.remove(); vi.restoreAllMocks(); });
const flush = () => act(async () => { await new Promise(resolve => setTimeout(resolve, 0)); });

it("saves a switch on change and never reports a failed save as successful", async () => {
  const save = vi.spyOn(bridgeApi, "saveWorkerSettings").mockRejectedValueOnce(new Error("Could not persist"));
  await act(async () => { root.render(<WorkersPage adapters={[]} />); });
  await flush();
  const toggle = host.querySelector<HTMLButtonElement>('[role="switch"][aria-label="Automatic retry"]')!;
  await act(async () => { toggle.click(); });
  await flush();
  expect(save).toHaveBeenCalledWith(expect.any(String), expect.objectContaining({ automaticRetry: false }));
  expect(host.querySelector('[role="alert"]')?.textContent).toContain("Could not persist");
  expect(host.textContent).not.toContain("Saved");
  expect([...host.querySelectorAll("button")].map(button => button.textContent)).not.toContain("Save task settings");
});

it("saves a valid number on change, and never sends one the host would refuse", async () => {
  const save = vi.spyOn(bridgeApi, "saveWorkerSettings").mockImplementation(async (_workspace, settings) => settings);
  await act(async () => { root.render(<WorkersPage adapters={[]} />); });
  await flush();
  const field = host.querySelector<HTMLInputElement>('input[aria-label="Stop waiting after (seconds)"]')!;
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
  await act(async () => { setter.call(field, ""); field.dispatchEvent(new Event("input", { bubbles: true })); field.dispatchEvent(new Event("focusout", { bubbles: true })); });
  await flush();
  expect(save).not.toHaveBeenCalled();
  expect(field.getAttribute("aria-invalid")).toBe("true");
  await act(async () => { setter.call(field, "900"); field.dispatchEvent(new Event("input", { bubbles: true })); field.dispatchEvent(new Event("focusout", { bubbles: true })); });
  await flush();
  expect(save).toHaveBeenCalledTimes(1);
  expect(save).toHaveBeenCalledWith(expect.any(String), expect.objectContaining({ stallTimeoutSeconds: 900 }));
  expect(host.textContent).toContain("Saved");
});

it("no longer holds the reviewer settings: they live in the Reviews app", async () => {
  const reviewer = vi.spyOn(bridgeApi, "reviewerSettings");
  await act(async () => { root.render(<WorkersPage adapters={[]} />); });
  expect(host.querySelector("[data-reviewer-settings]")).toBeNull();
  expect(host.textContent).not.toContain("Reviewer instructions");
  expect(reviewer).not.toHaveBeenCalled();
});
