// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ReadTrailResult } from "@/protocol/generated/protocol";
import { bridgeApi } from "@/api";
import { mockReadTrail, resetMockTrails } from "@/trailsMock";
import { TrailPeek, TrailPeekCard, resetTrailPeekCache } from "./TrailPeek";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLDivElement;
let root: Root;
beforeEach(() => {
  resetMockTrails();
  resetTrailPeekCache();
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
});
afterEach(() => {
  act(() => root.unmount());
  host.remove();
  document.body.innerHTML = "";
  vi.restoreAllMocks();
});

const wait = (ms = 0) => act(async () => { await new Promise(resolve => setTimeout(resolve, ms)); for (let i = 0; i < 5; i++) await Promise.resolve(); });
const card = () => document.querySelector<HTMLElement>("[data-trail-peek]");
const summary = () => mockReadTrail("trail-archive").trail.summary;

function controlledRead() {
  const calls: Array<{ id: string; refresh?: boolean; resolve: () => void; reject: (error: Error) => void }> = [];
  const spy = vi.spyOn(bridgeApi, "readTrail").mockImplementation((id, refresh) => new Promise<ReadTrailResult>((resolve, reject) => {
    calls.push({ id, refresh, resolve: () => resolve(mockReadTrail(id)), reject });
  }));
  return { calls, spy };
}

function peek(onOpen = vi.fn()) {
  act(() => root.render(<TrailPeek trail={summary()} trigger={<button type="button">Trail</button>} onOpen={onOpen} delay={0} />));
  return { onOpen, trigger: host.querySelector<HTMLButtonElement>("button")! };
}

async function openWith(trigger: HTMLButtonElement) {
  act(() => trigger.focus());
  await wait(20);
}

async function close(trigger: HTMLButtonElement) {
  act(() => trigger.blur());
  await wait(400);
}

describe("TrailPeek", () => {
  it("opens on focus with the summary at once, then the latest events", async () => {
    const { calls } = controlledRead();
    const { trigger } = peek();
    expect(card()).toBeNull();
    await openWith(trigger);
    expect(card()).not.toBeNull();
    // The summary paints before the read answers; the events wait in a labelled skeleton.
    expect(card()!.textContent).toContain("Archive a chat from the sidebar");
    expect(card()!.textContent).toContain("Needs you");
    expect(card()!.textContent).toContain("vitest is failing on #1012");
    expect(card()!.querySelector("[role=status][aria-label='Loading trail']")).not.toBeNull();
    // A local read: the card never waits on GitHub.
    expect(calls).toHaveLength(1);
    expect(calls[0]).toMatchObject({ id: "trail-archive" });
    expect(calls[0].refresh).toBeFalsy();
    await act(async () => { calls[0].resolve(); });
    await wait();
    expect(card()!.querySelector("[role=status]")).toBeNull();
    const latest = [...card()!.querySelectorAll("[data-peek-event]")];
    expect(latest.map(item => item.getAttribute("data-peek-event"))).toEqual(["checks", "message", "pullRequest", "chat"]);
    expect(latest[0].textContent).toContain("Checks on #1012");
    expect(card()!.querySelector("[aria-label='Open on this trail']")?.textContent).toContain("vitest failing on #1012");
  });

  it("shows the last events at once on a second open and reads again behind them", async () => {
    const { calls } = controlledRead();
    const { trigger } = peek();
    await openWith(trigger);
    await act(async () => { calls[0].resolve(); });
    await wait();
    await close(trigger);
    expect(card()).toBeNull();
    await openWith(trigger);
    expect(card()!.querySelectorAll("[data-peek-event]")).toHaveLength(4);
    expect(card()!.querySelector("[role=status]")).toBeNull();
    expect(calls).toHaveLength(2);
  });

  it("says the read failed and keeps the summary", async () => {
    const { calls } = controlledRead();
    const { trigger } = peek();
    await openWith(trigger);
    await act(async () => { calls[0].reject(new Error("database is locked")); });
    await wait();
    expect(card()!.textContent).toContain("Could not read this trail.");
    expect(card()!.textContent).toContain("Archive a chat from the sidebar");
    expect(card()!.querySelector("[role=status]")).toBeNull();
  });

  it("opens the trail from Open trail and closes", async () => {
    const { calls } = controlledRead();
    const { trigger, onOpen } = peek();
    await openWith(trigger);
    await act(async () => { calls[0].resolve(); });
    await wait();
    const open = [...card()!.querySelectorAll("button")].find(item => item.textContent?.startsWith("Open trail"))!;
    expect(open.textContent).toContain("6 items");
    act(() => open.click());
    expect(onOpen).toHaveBeenCalledWith("trail-archive");
    await wait(400);
    expect(card()).toBeNull();
  });
});

describe("TrailPeekCard", () => {
  it("lists two open items and counts the rest", () => {
    const detail = mockReadTrail("trail-archive").trail;
    detail.open = [1, 2, 3, 4].map(index => ({ ...detail.open[0], label: `open ${index}` }));
    act(() => root.render(<TrailPeekCard trail={detail.summary} detail={detail} failed={false} onOpen={vi.fn()} />));
    const items = [...host.querySelectorAll("[aria-label='Open on this trail'] li")].map(item => item.textContent);
    expect(items).toEqual(["open 1", "open 2"]);
    expect(host.textContent).toContain("and 2 more");
  });

  it("says when a trail has nothing on it yet", () => {
    const detail = mockReadTrail("trail-archive").trail;
    detail.events = [];
    detail.open = [];
    act(() => root.render(<TrailPeekCard trail={detail.summary} detail={detail} failed={false} onOpen={vi.fn()} />));
    expect(host.textContent).toContain("Nothing on this trail yet.");
    expect(host.querySelector("[aria-label='Open on this trail']")).toBeNull();
  });
});
