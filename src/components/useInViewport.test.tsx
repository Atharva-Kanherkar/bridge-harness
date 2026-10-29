// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { useInViewport } from "./useInViewport";

class MockIntersectionObserver {
  static instances: MockIntersectionObserver[] = [];
  readonly callback: IntersectionObserverCallback;
  readonly options: IntersectionObserverInit;
  readonly observed: Element[] = [];
  disconnected = false;

  constructor(callback: IntersectionObserverCallback, options: IntersectionObserverInit = {}) {
    this.callback = callback;
    this.options = options;
    MockIntersectionObserver.instances.push(this);
  }
  observe(element: Element) { this.observed.push(element); }
  unobserve() { /* one-shot hook never unobserves */ }
  disconnect() { this.disconnected = true; }
  takeRecords(): IntersectionObserverEntry[] { return []; }
  trigger(isIntersecting: boolean, target: Element = this.observed[0]) {
    this.callback([{ isIntersecting, target } as IntersectionObserverEntry], this as unknown as IntersectionObserver);
  }
}

function Probe({ label = "one" }: { label?: string }) {
  const [ref, visible] = useInViewport<HTMLDivElement>();
  return <div ref={ref} data-visible={visible ? "yes" : "no"} data-probe={label} />;
}

function ScrollProbe() {
  return <div data-scroll style={{ overflowY: "auto" }}><Probe label="a"/><Probe label="b"/></div>;
}

describe("useInViewport", () => {
  let container: HTMLDivElement;
  let root: Root | undefined;
  const mount = async (node = <Probe />) => {
    (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    container = document.createElement("div");
    document.body.appendChild(container);
    root = createRoot(container);
    await act(async () => root!.render(node));
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
    expect(observer.options.root).toBeNull();
    expect(observer.options.rootMargin).toBe("200px");
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

  it("measures the look-ahead against the transcript scroller and shares one observer", async () => {
    vi.stubGlobal("IntersectionObserver", MockIntersectionObserver);
    MockIntersectionObserver.instances = [];
    const scroller = await mount(<ScrollProbe />);
    expect(MockIntersectionObserver.instances).toHaveLength(1);
    const observer = MockIntersectionObserver.instances[0];
    // The default viewport root sees nothing: the scroller clips a fence
    // before the page does, so the 200px look-ahead has to grow the scroller.
    expect(observer.options.root).toBe(scroller);
    expect(observer.options.rootMargin).toBe("200px");

    const [first, second] = observer.observed;
    await act(async () => observer.trigger(true, first));
    expect(first.getAttribute("data-visible")).toBe("yes");
    expect(second.getAttribute("data-visible")).toBe("no");
    await act(async () => observer.trigger(true, second));
    expect(second.getAttribute("data-visible")).toBe("yes");
    expect(observer.disconnected).toBe(true);
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
