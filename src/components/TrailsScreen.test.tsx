// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { TrailsScreen } from "./TrailsScreen";
import { resetMockTrails } from "../trailsMock";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLDivElement;
let root: Root;
beforeEach(() => {
  resetMockTrails();
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
});
afterEach(() => {
  act(() => root.unmount());
  host.remove();
  vi.restoreAllMocks();
});

const settle = () => act(async () => { for (let i = 0; i < 5; i++) await Promise.resolve(); });

function screen(props: Partial<Parameters<typeof TrailsScreen>[0]> = {}) {
  const onSelectTrail = vi.fn();
  act(() => root.render(<TrailsScreen
    trailId={null}
    onSelectTrail={onSelectTrail}
    workspaces={[{ id: "demo-1", title: "Build session supervisor" }]}
    harnesses={[{ id: "codex", label: "Codex" }, { id: "claude", label: "Claude" }]}
    onOpenSession={vi.fn()}
    onOpenNote={vi.fn()}
    onOpenUrl={vi.fn()}
    onStart={vi.fn(async () => undefined)}
    {...props}
  />));
  return { onSelectTrail };
}

const button = (name: RegExp) => [...host.querySelectorAll("button")].find(item => name.test(item.textContent ?? "") || name.test(item.getAttribute("aria-label") ?? ""));

describe("TrailsScreen list", () => {
  it("renders sections with counts, needs you first", async () => {
    screen();
    await settle();
    const headings = [...host.querySelectorAll("h2")].map(item => item.textContent);
    expect(headings).toEqual(["Needs you · 2", "Shipped · 1"]);
    expect(host.textContent).toContain("vitest is failing on #1012");
    expect(host.textContent).toContain("Build session supervisor");
  });

  it("opens a trail from its row", async () => {
    const { onSelectTrail } = screen();
    await settle();
    act(() => button(/Archive a chat from the sidebar/)!.click());
    expect(onSelectTrail).toHaveBeenCalledWith("trail-archive");
  });

  it("creates a trail from the New trail form and opens it", async () => {
    const { onSelectTrail } = screen();
    await settle();
    act(() => button(/^New trail$/)!.click());
    const input = host.querySelector<HTMLInputElement>("input[aria-label='Trail title']")!;
    const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
    act(() => { setter.call(input, "  Reply watch v2 "); input.dispatchEvent(new Event("input", { bubbles: true })); });
    await act(async () => { host.querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })); });
    await settle();
    expect(onSelectTrail).toHaveBeenCalledTimes(1);
    expect(onSelectTrail.mock.calls[0][0]).toMatch(/^trail-/);
  });

  it("shows one line and New trail when there are none", async () => {
    resetMockTrails();
    const { mockUpdateTrail, mockListTrails } = await import("../trailsMock");
    for (const item of mockListTrails().trails) mockUpdateTrail(item.id, null, true);
    screen();
    await settle();
    expect(host.textContent).toContain("No trails yet.");
    expect(button(/^New trail$/)).toBeTruthy();
  });
});

describe("TrailsScreen selection", () => {
  it("drops a slower response for a trail the person left, so actions hit the trail on screen", async () => {
    const api = (await import("../api")).bridgeApi;
    const { mockReadTrail } = await import("../trailsMock");
    const pending = new Map<string, Array<{ resolve: () => void; reject: (error: Error) => void }>>();
    vi.spyOn(api, "readTrail").mockImplementation(id => new Promise((resolve, reject) => {
      const list = pending.get(id) ?? [];
      list.push({ resolve: () => resolve(mockReadTrail(id)), reject });
      pending.set(id, list);
    }));
    const update = vi.spyOn(api, "updateTrail").mockResolvedValue({ trail: mockReadTrail("trail-reply").trail.summary });
    screen({ trailId: "trail-archive" });
    await settle();
    screen({ trailId: null });
    await settle();
    screen({ trailId: "trail-reply" });
    await settle();
    // B answers first, then A's slow GitHub refresh lands.
    await act(async () => { pending.get("trail-reply")!.shift()!.resolve(); });
    await settle();
    await act(async () => { pending.get("trail-archive")!.shift()!.resolve(); });
    await settle();
    expect(host.querySelector("h1")?.textContent).toBe("Outgoing: reply watch");
    act(() => button(/^Archive trail$/)!.click());
    expect(update).toHaveBeenCalledWith("trail-reply", { archived: true });
    // An older request for the same trail loses to a newer one, errors
    // included: the open's GitHub refresh (older) against the read a rename
    // makes (newer).
    screen({ trailId: "trail-archive" });
    await settle();
    await act(async () => { pending.get("trail-archive")!.shift()!.resolve(); });
    await settle();
    expect(pending.get("trail-archive")).toHaveLength(1);
    act(() => host.querySelector<HTMLButtonElement>("h1 button")!.click());
    const input = host.querySelector<HTMLInputElement>("input[aria-label='Trail title']")!;
    const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
    act(() => { setter.call(input, "Archive from the row"); input.dispatchEvent(new Event("input", { bubbles: true })); });
    await act(async () => { input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true })); });
    await settle();
    const [older, newer] = pending.get("trail-archive")!.splice(0, 2);
    expect(newer).toBeTruthy();
    await act(async () => { newer.resolve(); });
    await settle();
    await act(async () => { older.reject(new Error("GitHub timed out")); });
    await settle();
    expect(host.textContent).not.toContain("GitHub timed out");
    expect(host.querySelector("h1")?.textContent).toBe("Archive a chat from the sidebar");
    // The refresh settled, so Refresh stops spinning.
    expect(button(/^Refresh from GitHub$/)!.getAttribute("aria-busy")).toBe("false");
  });
});

describe("TrailsScreen loading", () => {
  type Pending = { id: string; refresh: boolean; resolve: () => void; reject: (error: Error) => void };
  async function controlled() {
    const api = (await import("../api")).bridgeApi;
    const { mockReadTrail } = await import("../trailsMock");
    const calls: Pending[] = [];
    const read = vi.spyOn(api, "readTrail").mockImplementation((id, refresh = false) => new Promise((resolve, reject) => {
      calls.push({ id, refresh, resolve: () => resolve(mockReadTrail(id)), reject });
    }));
    return { calls, read };
  }
  const refreshButton = () => button(/^Refresh from GitHub$/)!;

  it("paints the header from a known summary, then reads locally, then from GitHub", async () => {
    const { calls } = await controlled();
    const { mockListTrails } = await import("../trailsMock");
    const known = mockListTrails().trails;
    screen({ trailId: "trail-archive", knownTrails: known });
    await settle();
    // Before any read answers: the title and reason from the summary, the timeline a labelled skeleton.
    expect(host.querySelector("h1")?.textContent).toBe("Archive a chat from the sidebar");
    expect(host.textContent).toContain("vitest is failing on #1012");
    expect(host.querySelector("[role=status][aria-label='Loading trail']")).not.toBeNull();
    expect(calls.map(call => call.refresh)).toEqual([false]);
    await act(async () => { calls[0].resolve(); });
    await settle();
    // The local read painted the timeline; the GitHub read started after it.
    expect(host.querySelector("[role=status][aria-label='Loading trail']")).toBeNull();
    expect(host.querySelector("ol[aria-label=Timeline]")).not.toBeNull();
    expect(calls.map(call => call.refresh)).toEqual([false, true]);
  });

  it("keeps the page and spins Refresh during the GitHub read", async () => {
    const { calls } = await controlled();
    screen({ trailId: "trail-archive" });
    await settle();
    await act(async () => { calls[0].resolve(); });
    await settle();
    expect(refreshButton().getAttribute("aria-busy")).toBe("true");
    expect(refreshButton().disabled).toBe(true);
    expect(refreshButton().querySelector("svg")?.getAttribute("class")).toContain("animate-spin");
    // The page stays: no skeleton while GitHub answers.
    expect(host.querySelector("ol[aria-label=Timeline]")).not.toBeNull();
    expect(host.querySelector("[role=status]")).toBeNull();
    await act(async () => { calls[1].reject(new Error("gh: rate limited")); });
    await settle();
    expect(refreshButton().getAttribute("aria-busy")).toBe("false");
    expect(refreshButton().disabled).toBe(false);
    expect(host.querySelector("ol[aria-label=Timeline]")).not.toBeNull();
    expect(host.textContent).toContain("gh: rate limited");
    // Refresh by hand reads from GitHub again.
    act(() => refreshButton().click());
    expect(calls.at(-1)?.refresh).toBe(true);
    expect(refreshButton().getAttribute("aria-busy")).toBe("true");
  });

  it("reads once more when GitHub's answer lost to a newer local read", async () => {
    const { calls } = await controlled();
    const api = (await import("../api")).bridgeApi;
    vi.spyOn(api, "updateTrail").mockResolvedValue({ trail: (await import("../trailsMock")).mockReadTrail("trail-archive").trail.summary });
    screen({ trailId: "trail-archive" });
    await settle();
    await act(async () => { calls[0].resolve(); });
    await settle();
    // A rename's local read overtakes the GitHub read.
    act(() => host.querySelector<HTMLButtonElement>("h1 button")!.click());
    const input = host.querySelector<HTMLInputElement>("input[aria-label='Trail title']")!;
    const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
    act(() => { setter.call(input, "Renamed"); input.dispatchEvent(new Event("input", { bubbles: true })); });
    await act(async () => { input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true })); });
    await settle();
    expect(calls.map(call => call.refresh)).toEqual([false, true, false]);
    await act(async () => { calls[2].resolve(); });
    await settle();
    await act(async () => { calls[1].resolve(); });
    await settle();
    // One catch-up local read, and no more.
    expect(calls.map(call => call.refresh)).toEqual([false, true, false, false]);
    await act(async () => { calls[3].resolve(); });
    await settle();
    expect(calls).toHaveLength(4);
  });

  it("shows the error with Retry when the first read fails", async () => {
    const { calls } = await controlled();
    screen({ trailId: "trail-archive" });
    await settle();
    await act(async () => { calls[0].reject(new Error("database is locked")); });
    await settle();
    await act(async () => { calls[1].reject(new Error("database is locked")); });
    await settle();
    expect(host.querySelector("[role=alert]")?.textContent).toContain("database is locked");
    act(() => button(/^Retry$/)!.click());
    expect(calls.at(-1)).toMatchObject({ id: "trail-archive", refresh: false });
    await act(async () => { calls.at(-1)!.resolve(); });
    await settle();
    expect(host.querySelector("[role=alert]")).toBeNull();
    expect(host.querySelector("h1")?.textContent).toBe("Archive a chat from the sidebar");
  });
});
