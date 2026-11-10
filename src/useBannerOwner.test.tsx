// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { bridgeApi } from "./api";
import { useBannerOwner } from "./useBannerOwner";

const settle = () => act(async () => { await Promise.resolve(); await Promise.resolve(); });

function Probe({ focused, seen }: { focused: boolean; seen: boolean[] }) {
  seen.push(useBannerOwner(focused, "me", 10_000));
  return null;
}

describe("useBannerOwner", () => {
  let container: HTMLDivElement;
  let root: Root;
  let seen: boolean[];

  beforeEach(() => {
    (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    vi.useFakeTimers();
    // A browser tab that has not been asked yet: it could post after asking.
    vi.stubGlobal("Notification", { permission: "default" });
    seen = [];
    container = document.createElement("div");
    document.body.append(container);
    root = createRoot(container);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
    vi.useRealTimers();
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  const render = (focused: boolean) => act(async () => { root.render(<Probe focused={focused} seen={seen} />); });

  it("reports on mount, on a focus change and on the interval, and follows the answer", async () => {
    const report = vi.spyOn(bridgeApi, "reportPresence").mockResolvedValue({ bannerOwner: false });
    await render(false);
    await settle();
    expect(report).toHaveBeenLastCalledWith("me", { focused: false, canPostBanners: true });
    expect(seen.at(-1)).toBe(false);

    report.mockResolvedValue({ bannerOwner: true });
    await render(true);
    await settle();
    expect(report).toHaveBeenLastCalledWith("me", { focused: true, canPostBanners: true });
    expect(seen.at(-1)).toBe(true);

    const calls = report.mock.calls.length;
    await act(async () => { vi.advanceTimersByTime(10_000); });
    await settle();
    expect(report.mock.calls.length).toBe(calls + 1);
  });

  it("reports gone when the window closes", async () => {
    const report = vi.spyOn(bridgeApi, "reportPresence").mockResolvedValue({ bannerOwner: true });
    await render(false);
    await act(async () => { window.dispatchEvent(new Event("pagehide")); });
    expect(report).toHaveBeenLastCalledWith("me", { focused: false, gone: true });
  });

  it("owns banners when a report fails, so nothing goes silent", async () => {
    vi.spyOn(bridgeApi, "reportPresence").mockRejectedValue(new Error("method_not_found"));
    await render(false);
    await settle();
    expect(seen.at(-1)).toBe(true);
  });

  it("drops a late answer that a newer report superseded", async () => {
    const answers: ((value: { bannerOwner: boolean }) => void)[] = [];
    vi.spyOn(bridgeApi, "reportPresence").mockImplementation(() => new Promise(resolve => { answers.push(resolve); }));
    await render(false);
    await render(true);
    expect(answers).toHaveLength(2);
    await act(async () => { answers[1]({ bannerOwner: true }); });
    await act(async () => { answers[0]({ bannerOwner: false }); });
    expect(seen.at(-1)).toBe(true);
  });

  it("tells core a browser tab whose person denied notifications cannot post banners", async () => {
    const report = vi.spyOn(bridgeApi, "reportPresence").mockResolvedValue({ bannerOwner: false });
    vi.stubGlobal("Notification", { permission: "denied" });
    await render(false);
    await settle();
    expect(report).toHaveBeenLastCalledWith("me", { focused: false, canPostBanners: false });
  });
});

