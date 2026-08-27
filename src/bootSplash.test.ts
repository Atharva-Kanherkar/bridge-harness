// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { dismissBootSplash } from "./bootSplash";

function mountSplash(): HTMLElement {
  const splash = document.createElement("div");
  splash.id = "boot-splash";
  document.body.appendChild(splash);
  return splash;
}

async function afterTwoFrames(): Promise<void> {
  await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
}

afterEach(() => {
  document.body.innerHTML = "";
  vi.unstubAllGlobals();
  vi.useRealTimers();
});

describe("dismissBootSplash", () => {
  it("does not throw when #boot-splash is absent", () => {
    expect(() => dismissBootSplash()).not.toThrow();
  });

  it("adds the hide class once the app has painted, without removing it yet", async () => {
    const splash = mountSplash();
    dismissBootSplash();
    await afterTwoFrames();
    expect(splash.classList.contains("boot-splash-hide")).toBe(true);
    expect(document.getElementById("boot-splash")).not.toBeNull();
  });

  it("removes the element once transitionend fires", async () => {
    const splash = mountSplash();
    dismissBootSplash();
    await afterTwoFrames();
    splash.dispatchEvent(new Event("transitionend"));
    expect(document.getElementById("boot-splash")).toBeNull();
  });

  it("removes the element via a fallback timeout even if transitionend never fires", () => {
    vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => {
      cb(0);
      return 0;
    });
    vi.useFakeTimers();
    mountSplash();
    dismissBootSplash();
    vi.advanceTimersByTime(500);
    expect(document.getElementById("boot-splash")).toBeNull();
  });

  it("is safe to call twice", async () => {
    const splash = mountSplash();
    dismissBootSplash();
    dismissBootSplash();
    await afterTwoFrames();
    expect(() => splash.dispatchEvent(new Event("transitionend"))).not.toThrow();
    expect(document.getElementById("boot-splash")).toBeNull();
  });
});
