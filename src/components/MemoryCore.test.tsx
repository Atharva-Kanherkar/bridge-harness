// @vitest-environment jsdom
// Contract: testing/feat-memory-core.md — the full-screen memory-graph surface.
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { bridgeApi } from "../api";
import type { MemoryChangedPayload, MemoryRecord } from "../types";
import { MemoryCore } from "./MemoryCore";

const rec = (id: string, over: Partial<MemoryRecord> = {}): MemoryRecord => ({
  id, scopeKey: "account:local", kind: "fact", body: `body ${id}`,
  provenance: "user_explicit", status: "active",
  validFrom: "2026-08-24T00:00:00Z", createdAt: "2026-08-24T00:00:00Z", updatedAt: "2026-08-24T00:00:00Z",
  ...over,
});

let container: HTMLDivElement;
let root: Root;
let graph: MemoryRecord[];
let proposed: MemoryRecord[];
let memoryHandler: ((payload: MemoryChangedPayload) => void) | undefined;

const flush = async () => { await act(async () => {}); };
const mount = (overrides: Partial<Parameters<typeof MemoryCore>[0]> = {}) => {
  act(() => { root.render(<MemoryCore open onClose={() => {}} onError={() => {}} {...overrides} />); });
};
const nodes = () => [...document.querySelectorAll<SVGGElement>("g[data-node]")];
const click = (element: Element) => { act(() => { element.dispatchEvent(new MouseEvent("click", { bubbles: true })); }); };
const setSearch = (value: string) => {
  const input = document.querySelector<HTMLInputElement>('input[aria-label="Search memory"]')!;
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
  act(() => { setter.call(input, value); input.dispatchEvent(new Event("input", { bubbles: true })); });
};

beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
  graph = [
    rec("mem_pin", { body: "Prefers Tailwind v4", confidenceBps: 9600 }),
    rec("mem_old", { body: "old direction", status: "superseded" }),
    rec("mem_new", { body: "new direction", supersedes: "mem_old", provenance: "model_proposal", confidenceBps: 9000 }),
    rec("mem_prop", { body: "worker gh token", status: "proposed", provenance: "model_proposal", confidenceBps: 7100 }),
  ];
  proposed = [graph[3]];
  memoryHandler = undefined;
  vi.spyOn(bridgeApi, "memoryGraphRecords").mockImplementation(async () => graph.map(r => ({ ...r })));
  vi.spyOn(bridgeApi, "listMemoryRecords").mockImplementation(async (_scope, status) => ({
    scopeKey: "account:local",
    records: status === "proposed" ? proposed.map(r => ({ ...r })) : graph.filter(r => r.status === "active"),
  }));
  vi.spyOn(bridgeApi, "memoryRecallStats").mockResolvedValue({
    perRecord: [{ id: "mem_pin", recalls: 12, lastRecalledDay: 13, inPacketRatio: 0.8, daily: Array(14).fill(1) }],
    injectionsPerDay: Array(14).fill(2), budgetCharsUsed: 1200, budgetCharsMax: 4000,
  });
  vi.spyOn(bridgeApi, "memoryCoRecallPairs").mockResolvedValue([{ a: "mem_pin", b: "mem_new", weight: 4 }]);
  vi.spyOn(bridgeApi, "memoryConsolidationLog").mockResolvedValue([{ op: "merge", detail: "folded 2 notes", day: 12 }]);
  vi.spyOn(bridgeApi, "onMemoryChanged").mockImplementation(async handler => {
    memoryHandler = handler;
    return () => { memoryHandler = undefined; };
  });
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.restoreAllMocks();
});

describe("MemoryCore", () => {
  it("renders nothing when closed", () => {
    mount({ open: false });
    expect(document.querySelector('[aria-label="Memory Core"]')).toBeNull();
  });

  it("plots a node per non-tombstoned record with a state attribute", async () => {
    mount();
    await flush();
    const states = nodes().map(node => node.getAttribute("data-state")).sort();
    expect(states).toEqual(["active", "pinned", "proposed", "superseded"]);
  });

  it("populates the inspector on select, including lineage", async () => {
    mount();
    await flush();
    const newNode = nodes().find(node => node.getAttribute("data-node") === "mem_new")!;
    click(newNode);
    await flush();
    expect(document.body.textContent).toContain("new direction");
    expect(document.body.textContent).toContain("mem_old"); // lineage ancestor
  });

  it("accepts a proposed record through approveMemoryRecord", async () => {
    const approve = vi.spyOn(bridgeApi, "approveMemoryRecord").mockResolvedValue(graph[3]);
    mount();
    await flush();
    const acceptButton = [...document.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent?.includes("Accept"))!;
    click(acceptButton);
    await flush();
    expect(approve).toHaveBeenCalledWith("mem_prop");
  });

  it("forgets the selected record through deleteMemoryRecord", async () => {
    const del = vi.spyOn(bridgeApi, "deleteMemoryRecord").mockResolvedValue(graph[0]);
    mount();
    await flush();
    click(nodes().find(node => node.getAttribute("data-node") === "mem_pin")!);
    await flush();
    const forget = [...document.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent?.includes("Forget"))!;
    click(forget);
    await flush();
    expect(del).toHaveBeenCalledWith("mem_pin");
  });

  it("dims nodes that fall out of the search", async () => {
    mount();
    await flush();
    setSearch("tailwind");
    await flush();
    const pin = nodes().find(node => node.getAttribute("data-node") === "mem_pin")!;
    const other = nodes().find(node => node.getAttribute("data-node") === "mem_prop")!;
    expect(Number(pin.getAttribute("opacity"))).toBeGreaterThan(0.5);
    expect(Number(other.getAttribute("opacity"))).toBeLessThan(0.5);
  });

  it("closes on Escape", async () => {
    const onClose = vi.fn();
    mount({ onClose });
    await flush();
    act(() => { window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" })); });
    expect(onClose).toHaveBeenCalled();
  });

  it("refetches on memory-changed behind a read generation", async () => {
    mount();
    await flush();
    expect(bridgeApi.memoryGraphRecords).toHaveBeenCalledTimes(1);
    act(() => memoryHandler?.({ scopeKey: "account:local" }));
    await flush();
    expect(bridgeApi.memoryGraphRecords).toHaveBeenCalledTimes(2);
  });
});
