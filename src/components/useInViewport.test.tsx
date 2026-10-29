// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { useInViewport } from "./useInViewport";

class MockIntersectionObserver {
  static instances: MockIntersectionObserver[] = [];
  readonly callback: IntersectionObserverCallback;
  readonly observed: Element[] = [];
  disconnected = false;

  constructor(callback: IntersectionObserverCallback) {
    this.callback = callback;
    MockIntersectionObserver.instances.push(this);
  }
  observe(element: Element) { this.observed.push(element); }
  unobserve() { /* one-shot hook never unobserves */ }
  disconnect() { this.disconnected = true; }
  takeRecords(): IntersectionObserverEntry[] { return []; }
  trigger(isIntersecting: boolean) {
    this.callback([{ isIntersecting } as IntersectionObserverEntry], this as unknown as IntersectionObserver);
  }
}

function Probe() {
  const [ref, visible] = useInViewport<HTMLDivElement>();
  return <div ref={ref} data-visible={visible ? "yes" : "no"} />;
}

describe("useInViewport", () => {
  let container: HTMLDivElement;
  let root: Root | undefined;
  const mount = async () => {
    (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    container = document.createElement("div");
    document.body.appendChild(container);
    root = createRoot(container);
    await act(async () => root!.render(<Probe />));
    return container.firstElementChild as HTMLElement;
  };

  afterEach(async () => {
    if (root) await act(async () => root!.unmount());
    root = undefined;
    container?.remove();
    vi.unstubAllGlobals();
    MockIntersectionObserver.instances = [];
  });

  it("returns visible when IntersectionObserver is unavailable", async () => {
    const element = await mount();
    expect(element.dataset.visible).toBe("yes");
  });

  it("starts hidden, turns visible on an intersecting entry, and stays visible", async () => {
    vi.stubGlobal("IntersectionObserver", MockIntersectionObserver);
    MockIntersectionObserver.instances = [];
    const element = await mount();
    const observer = MockIntersectionObserver.instances[0];
    expect(observer.observed).toEqual([element]);
    expect(element.dataset.visible).toBe("no");

    await act(async () => observer.trigger(false));
    expect(element.dataset.visible).toBe("no");

    await act(async () => observer.trigger(true));
    expect(element.dataset.visible).toBe("yes");
    expect(observer.disconnected).toBe(true);

    // Leaving the viewport must not flip it back: colour already paid for is
    // kept, and no second tokenization is scheduled.
    await act(async () => observer.trigger(false));
    expect(element.dataset.visible).toBe("yes");
  });

  it("disconnects on unmount and ignores a late entry", async () => {
    vi.stubGlobal("IntersectionObserver", MockIntersectionObserver);
    MockIntersectionObserver.instances = [];
    const element = await mount();
    const observer = MockIntersectionObserver.instances[0];
    expect(element.dataset.visible).toBe("no");

    await act(async () => root!.unmount());
    root = undefined;
    expect(observer.disconnected).toBe(true);
    expect(() => observer.trigger(true)).not.toThrow();
  });
});
