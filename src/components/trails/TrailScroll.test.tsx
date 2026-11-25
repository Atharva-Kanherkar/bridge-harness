// @vitest-environment jsdom
import { Profiler, act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { MotionGlobalConfig } from "framer-motion";
import { TrailPage } from "./TrailPage";
import { TrailsList } from "./TrailsList";
import { TrailsScreenSkeleton } from "./TrailSkeletons";
import { TRAIL_BAR_THRESHOLD, riseDelay } from "./TrailParts";
import { mockListTrails, mockReadTrail, resetMockTrails } from "@/trailsMock";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLDivElement;
let root: Root;
beforeEach(() => {
  MotionGlobalConfig.skipAnimations = true;
  resetMockTrails();
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
});
afterEach(() => {
  act(() => root.unmount());
  host.remove();
  document.body.innerHTML = "";
  vi.unstubAllGlobals();
});

const scroller = () => host.querySelector<HTMLDivElement>("[data-trail-scroll]")!;
const bar = () => host.querySelector<HTMLDivElement>("[data-trail-bar]")!;

/** jsdom has no layout: give the scroller a size and move it by hand. */
function scrollTo(top: number, { height = 600, content = 2400 } = {}) {
  const node = scroller();
  Object.defineProperty(node, "clientHeight", { configurable: true, value: height });
  Object.defineProperty(node, "scrollHeight", { configurable: true, value: content });
  act(() => {
    node.scrollTop = top;
    node.dispatchEvent(new Event("scroll"));
  });
}

function trailPage(trailId = "trail-archive", handlers: Record<string, unknown> = {}) {
  const props = {
    trail: mockReadTrail(trailId).trail, loading: false, onBack: vi.fn(), onContinue: vi.fn(), onOpenEvent: vi.fn(), onRemove: vi.fn(),
    onRename: vi.fn(async () => undefined), onArchive: vi.fn(), onRefresh: vi.fn(), onAdd: vi.fn(), ...handlers,
  };
  act(() => root.render(<TrailPage {...props} />));
  return props;
}

function list(onCreate = vi.fn(async () => undefined)) {
  act(() => root.render(
    <TrailsList trails={mockListTrails(true).trails} loading={false} workspaces={[]} showArchived={false} onToggleArchived={vi.fn()} onOpen={vi.fn()} onCreate={onCreate} />,
  ));
}

describe("Trails scrolling", () => {
  it("gives the list its own scroller as its outermost element", () => {
    list();
    const node = scroller();
    expect(host.firstElementChild).toBe(node);
    expect(node.className).toMatch(/\boverflow-y-auto\b/);
    expect(node.className).toMatch(/\bmin-h-0\b/);
    expect(node.className).toMatch(/\bflex-1\b/);
  });

  it("gives a trail's page the same scroller when loaded, while loading, and before anything is known", () => {
    trailPage();
    expect(host.firstElementChild).toBe(scroller());
    act(() => root.render(
      <TrailPage trail={null} summary={mockReadTrail("trail-archive").trail.summary} loading onBack={vi.fn()} onContinue={vi.fn()} onOpenEvent={vi.fn()} onRemove={vi.fn()} onRename={vi.fn(async () => undefined)} onArchive={vi.fn()} onRefresh={vi.fn()} onAdd={vi.fn()} />,
    ));
    expect(host.firstElementChild).toBe(scroller());
    act(() => root.render(
      <TrailPage trail={null} summary={null} loading onBack={vi.fn()} onContinue={vi.fn()} onOpenEvent={vi.fn()} onRemove={vi.fn()} onRename={vi.fn(async () => undefined)} onArchive={vi.fn()} onRefresh={vi.fn()} onAdd={vi.fn()} />,
    ));
    expect(host.firstElementChild).toBe(scroller());
  });

  it("lays the Suspense fallback out in the same scroller", () => {
    act(() => root.render(<TrailsScreenSkeleton trailOpen={false} />));
    expect(host.firstElementChild).toBe(scroller());
    act(() => root.render(<TrailsScreenSkeleton trailOpen />));
    expect(host.firstElementChild).toBe(scroller());
  });

  it("goes back to the top when the page shows another trail", () => {
    trailPage("trail-archive");
    scrollTo(900);
    expect(scroller().scrollTop).toBe(900);
    const other = mockListTrails(true).trails.find(trail => trail.id !== "trail-archive")!;
    trailPage(other.id);
    expect(scroller().scrollTop).toBe(0);
    expect(bar().dataset.shown).toBe("false");
  });
});

describe("the condensed bar", () => {
  it("stays hidden, unreachable and inert at the top, shows past the threshold, and hides again", () => {
    trailPage();
    expect(bar().dataset.shown).toBe("false");
    expect(bar().getAttribute("aria-hidden")).toBe("true");
    expect(bar().hasAttribute("inert")).toBe(true);

    scrollTo(TRAIL_BAR_THRESHOLD + 1);
    expect(bar().dataset.shown).toBe("true");
    expect(bar().hasAttribute("aria-hidden")).toBe(false);
    expect(bar().hasAttribute("inert")).toBe(false);

    scrollTo(TRAIL_BAR_THRESHOLD);
    expect(bar().dataset.shown).toBe("false");
    expect(bar().hasAttribute("inert")).toBe(true);
  });

  it("names the trail and continues it from the bar", () => {
    const props = trailPage();
    scrollTo(400);
    expect(bar().textContent).toContain(mockReadTrail("trail-archive").trail.summary.title);
    const continueButton = [...bar().querySelectorAll("button")].find(button => /Continue/.test(button.textContent ?? ""))!;
    act(() => continueButton.click());
    expect(props.onContinue).toHaveBeenCalledTimes(1);
  });

  it("names the list and opens the new trail form from the bar", () => {
    list();
    scrollTo(400);
    expect(bar().querySelector("button")?.textContent).toBe("Trails");
    const newTrail = [...bar().querySelectorAll("button")].find(button => /New trail/.test(button.textContent ?? ""))!;
    act(() => newTrail.click());
    expect(host.querySelector("form[aria-label='New trail']")).not.toBeNull();
  });

  it.each([
    [false, "smooth"],
    [true, "auto"],
  ])("scrolls back to the top from its title (reduced motion %s → %s)", (reduce, behavior) => {
    vi.stubGlobal("matchMedia", (query: string) => ({ matches: reduce && query.includes("reduce"), media: query, addEventListener() {}, removeEventListener() {} }));
    const spy = vi.fn();
    HTMLElement.prototype.scrollTo = spy as unknown as typeof HTMLElement.prototype.scrollTo;
    try {
      trailPage();
      scrollTo(800);
      act(() => bar().querySelector<HTMLButtonElement>("button[data-tip='Back to top']")!.click());
      expect(spy).toHaveBeenCalledWith({ top: 0, behavior });
    } finally {
      delete (HTMLElement.prototype as { scrollTo?: unknown }).scrollTo;
    }
  });

  it("writes scroll progress to a CSS variable and renders React only when the bar flips", async () => {
    let commits = 0;
    const props = {
      trail: mockReadTrail("trail-archive").trail, loading: false, onBack: vi.fn(), onContinue: vi.fn(), onOpenEvent: vi.fn(), onRemove: vi.fn(),
      onRename: vi.fn(async () => undefined), onArchive: vi.fn(), onRefresh: vi.fn(), onAdd: vi.fn(),
    };
    act(() => root.render(<Profiler id="trail" onRender={() => { commits++; }}><TrailPage {...props} /></Profiler>));
    // Let the page's own brief read land first: it renders once on its own.
    await act(async () => { for (let i = 0; i < 6; i++) await Promise.resolve(); });
    const settled = commits;
    scrollTo(200, { height: 600, content: 1400 });
    expect(scroller().style.getPropertyValue("--trail-scroll-progress")).toBe("0.2500");
    expect(commits - settled).toBe(1);
    scrollTo(400, { height: 600, content: 1400 });
    scrollTo(800, { height: 600, content: 1400 });
    expect(scroller().style.getPropertyValue("--trail-scroll-progress")).toBe("1.0000");
    expect(commits - settled).toBe(1);
  });
});

describe("entrance motion", () => {
  it("cascades list rows and caps the delay", () => {
    list();
    const rows = [...host.querySelectorAll<HTMLLIElement>("li.trail-rise")];
    expect(rows.length).toBeGreaterThan(1);
    expect(rows[0].style.getPropertyValue("--trail-rise-delay")).toBe("0ms");
    expect(rows[1].style.getPropertyValue("--trail-rise-delay")).toBe("32ms");
    expect(riseDelay(12)).toEqual(riseDelay(40));
  });

  it("rises and reveals every timeline event in order across days", () => {
    trailPage();
    const events = [...host.querySelectorAll<HTMLLIElement>("li[data-event-kind]")];
    expect(events.length).toBeGreaterThan(1);
    const delays = events.map(event => {
      expect(event.classList.contains("trail-reveal")).toBe(true);
      const rise = event.querySelector<HTMLElement>(".trail-rise")!;
      return parseInt(rise.style.getPropertyValue("--trail-rise-delay"), 10);
    });
    expect(delays).toEqual([...delays].sort((a, b) => a - b));
    expect(new Set(delays.slice(0, 12)).size).toBe(Math.min(12, delays.length));
  });

  it("is switched off under reduced motion", () => {
    const css = readFileSync(join(process.cwd(), "src/index.css"), "utf8");
    expect(css).toMatch(/@supports \(animation-timeline: view\(\)\)\s*{\s*\.trail-reveal/);
    expect(css).toMatch(/@media \(prefers-reduced-motion: reduce\)\s*{\s*\.trail-reveal { animation: none; }/);
    trailPage();
    expect(host.querySelector("[data-trail-progress]")!.className).toMatch(/\bmotion-reduce:hidden\b/);
  });
});
