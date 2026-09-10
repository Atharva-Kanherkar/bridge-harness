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
it("saves the selected workspace settings and never reports a failed save as successful", async () => {
  const save = vi.spyOn(bridgeApi, "saveWorkerSettings").mockRejectedValueOnce(new Error("Could not persist"));
  await act(async () => { root.render(<WorkersPage adapters={[]} />); });
  const toggle = host.querySelector<HTMLButtonElement>('[role="switch"][aria-label="Automatic retry"]')!;
  await act(async () => { toggle.click(); });
  await act(async () => { host.querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })); });
  expect(save).toHaveBeenCalledWith(expect.any(String), expect.objectContaining({ automaticRetry: false }));
  expect(host.querySelector('[role="alert"]')?.textContent).toContain("Could not persist");
  expect(host.querySelector('[role="status"]')?.textContent).not.toBe("Saved");
});
