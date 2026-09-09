// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { StoragePage } from "./StoragePage";

let host: HTMLDivElement;
let root: Root;
beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
});
afterEach(() => {
  act(() => root.unmount());
  host.remove();
  vi.restoreAllMocks();
});

/** Let the mount effect's two fetches settle. */
async function render() {
  await act(async () => {
    root.render(<StoragePage />);
  });
}

const button = (label: string) =>
  [...host.querySelectorAll("button")].find(node => node.textContent?.trim() === label);

it("shows what the worktrees cost against the cap", async () => {
  await render();
  const text = host.textContent ?? "";
  expect(text).toContain("2.6 GiB");
  expect(text).toContain("across 3 checkouts");
  expect(text).toContain("10.0 GiB per repository");
});

it("offers Reclaim only for a checkout that can be proven expendable", async () => {
  await render();
  const rows = [...host.querySelectorAll("li")];
  const reclaimable = rows.find(row => row.textContent?.includes("bridge/demo-session2"));
  const retained = rows.find(row => row.textContent?.includes("bridge/worker-1w"));

  expect(reclaimable?.querySelector("button")).toBeTruthy();
  expect(retained?.querySelector("button")).toBeNull();
  // The reason replaces the control rather than sitting in a tooltip.
  expect(retained?.textContent).toContain("has not been adopted or discarded yet");
});

it("never offers to reclaim a checkout Bridge did not create", async () => {
  await render();
  const external = [...host.querySelectorAll("li")]
    .find(row => row.textContent?.includes("chore/hand-made"));
  expect(external?.querySelector("button")).toBeNull();
  expect(external?.textContent).toContain("Not created by Bridge");
});

it("reports what a reclaim freed and drops the row", async () => {
  await render();
  await act(async () => {
    button("Reclaim")?.click();
  });
  expect(host.textContent).toContain("Confirm cleanup");
  await act(async () => { button("Confirm cleanup")?.click(); });
  expect(host.textContent).toContain("Reclaimed 2.5 GiB");
  expect(host.textContent).not.toContain("bridge/demo-session2");
});

it("says plainly when a sweep could reclaim nothing", async () => {
  await render();
  // The one reclaimable row goes first, so the second sweep has nothing left.
  await act(async () => {
    button("Sweep")?.click();
  });
  await act(async () => { button("Confirm cleanup")?.click(); });
  await act(async () => {
    button("Sweep")?.click();
  });
  await act(async () => { button("Confirm cleanup")?.click(); });
  expect(host.textContent).toContain("Nothing could be reclaimed safely");
});
