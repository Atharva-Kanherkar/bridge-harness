// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { bridgeApi } from "../../api";
import type { WorktreeInventoryEntry, WorktreeUsage } from "../../types";
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
  expect(text).toContain("across 4 checkouts");
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

it("offers Delete for a checkout Bridge could not prove safe, and never for one it flatly retains", async () => {
  await render();
  const rows = [...host.querySelectorAll("li")];
  const atRisk = rows.find(row => row.textContent?.includes("bridge/worker-scratch"));
  const retained = rows.find(row => row.textContent?.includes("bridge/worker-1w"));

  expect(atRisk?.textContent).toContain("Delete");
  expect(retained?.textContent).not.toContain("Delete");
});

it("deletes a checkout only after the destructive confirmation, and drops the row", async () => {
  await render();
  await act(async () => {
    button("Delete")?.click();
  });
  expect(host.textContent).toContain("Bridge could not prove this checkout is safe to remove");
  await act(async () => { button("Delete anyway")?.click(); });
  expect(host.textContent).toContain("Deleted");
  expect(host.textContent).not.toContain("bridge/worker-scratch");
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


it("confirms deletion of an unreadable checkout and refreshes rows and totals", async () => {
  const entry: WorktreeInventoryEntry = {
    id: "orphan", kind: "worker", repoRoot: "/repo", path: "/worktrees/workers/repo/orphan",
    branch: "orphan-branch", ownerSessionId: null, ownerWorkspaceId: null,
    state: "unverifiable", disposition: "unverifiable", retainedReason: "git cannot read this checkout",
    assessedAt: null, sizeBytes: 4096, sizeMeasuredAt: null, createdAt: "2026-09-01", lastUsedAt: "2026-09-01", idleSeconds: 500,
  };
  const usage: WorktreeUsage = {
    totalCount: 1, totalBytes: 4096, reclaimableCount: 0, reclaimableBytes: 0, retainedCount: 1,
    maxTotalBytes: 10240, maxPerRepo: 12, workerIdleTtlSeconds: 86400, orchestratorIdleTtlSeconds: 86400, githubIdleTtlSeconds: 86400,
    repositories: [{ repoRoot: "/repo", count: 1, sizeBytes: 4096, reclaimableBytes: 0, overBudget: false }],
  };
  const inventory = vi.spyOn(bridgeApi, "listWorktrees").mockResolvedValue([entry]);
  const totals = vi.spyOn(bridgeApi, "worktreeUsage").mockResolvedValue(usage);
  const reclaim = vi.spyOn(bridgeApi, "reclaimWorktree").mockResolvedValue({ reclaimed: true, bytesFreed: 4096, disposition: "unverifiable", detail: null });
  await render();
  await act(async () => { button("Delete")!.click(); });
  expect(host.textContent).toContain(entry.path);
  expect(reclaim).not.toHaveBeenCalled();
  await act(async () => { button("Cancel")!.click(); });
  expect(reclaim).not.toHaveBeenCalled();
  await act(async () => { button("Delete")!.click(); });
  inventory.mockResolvedValue([]);
  totals.mockResolvedValue({ ...usage, totalCount: 0, totalBytes: 0, retainedCount: 0, repositories: [] });
  await act(async () => { button("Delete anyway")!.click(); });
  expect(reclaim).toHaveBeenCalledWith("orphan", true);
  expect(host.textContent).toContain("Deleted 4.0 KiB");
  expect(host.textContent).toContain("across 0 checkouts");
  expect(host.textContent).not.toContain("orphan-branch");
});

it("explains external counts, unknown sizes and the scope of retention", async () => {
  const entries = await bridgeApi.listWorktrees();
  const external = entries.find(entry => entry.state === "external")!;
  vi.spyOn(bridgeApi, "listWorktrees").mockResolvedValue([{ ...external, sizeBytes: null }]);
  await render();
  expect(host.textContent).toContain("including 1 external");
  expect(host.textContent).toContain("1 checkout not yet measured");
  expect(host.textContent).toContain("targets at most");
  expect(host.textContent).toContain("Confirmed Delete can discard dirty or unreadable checkouts");
});
