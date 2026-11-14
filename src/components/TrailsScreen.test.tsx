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
    // An older request for the same trail loses to a newer one, errors included.
    screen({ trailId: "trail-archive" });
    await settle();
    await act(async () => { pending.get("trail-archive")!.shift()!.resolve(); });
    await settle();
    act(() => button(/^Refresh from GitHub$/)!.click());
    act(() => button(/^Refresh from GitHub$/)!.click());
    const [older, newer] = pending.get("trail-archive")!.splice(0, 2);
    await act(async () => { newer.resolve(); });
    await settle();
    await act(async () => { older.reject(new Error("GitHub timed out")); });
    await settle();
    expect(host.textContent).not.toContain("GitHub timed out");
    expect(host.querySelector("h1")?.textContent).toBe("Archive a chat from the sidebar");
  });
});
