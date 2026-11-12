// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it } from "vitest";
import { Skeleton, SkeletonCards, SkeletonRegion, SkeletonRows, SkeletonText, skeletonDelay } from "./skeleton";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const mounted: { root: Root; host: HTMLDivElement }[] = [];

function render(node: React.ReactNode): HTMLDivElement {
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  act(() => root.render(node));
  mounted.push({ root, host });
  return host;
}

afterEach(() => {
  for (const { root, host } of mounted.splice(0)) {
    act(() => root.unmount());
    host.remove();
  }
});

const bars = (scope: ParentNode) => [...scope.querySelectorAll<HTMLElement>(".skeleton-shimmer")];

describe("skeleton kit", () => {
  it("names the wait once for assistive tech and hides every bar", () => {
    const view = render(<SkeletonRegion label="Loading things"><Skeleton /><Skeleton index={1} /></SkeletonRegion>);
    const region = view.querySelector('[role="status"]')!;
    expect(region.getAttribute("aria-label")).toBe("Loading things");
    expect(region.getAttribute("aria-busy")).toBe("true");
    expect(region.querySelector(".sr-only")?.textContent).toBe("Loading things");
    expect(region.textContent).toBe("Loading things");
    expect(region.classList.contains("animate-skeleton-in")).toBe(true);
    expect(bars(region)).toHaveLength(2);
    expect(bars(region).every(bar => bar.getAttribute("aria-hidden") === "true")).toBe(true);
  });

  it("puts the label first so a divided region never rules under its last row", () => {
    const view = render(<SkeletonRows label="Loading rows" rows={3} />);
    const region = view.querySelector('[role="status"]')!;
    expect(region.firstElementChild?.classList.contains("sr-only")).toBe(true);
    expect(region.lastElementChild?.hasAttribute("data-skeleton-row")).toBe(true);
  });

  it("draws the requested rows with their lead, meta and trailing shapes", () => {
    const view = render(<SkeletonRows label="Loading apps" rows={5} lead="tile" trailing />);
    const rows = [...view.querySelectorAll("[data-skeleton-row]")];
    expect(rows).toHaveLength(5);
    // lead + title + meta + trailing
    expect(bars(rows[0])).toHaveLength(4);
    expect(bars(rows[0])[0].className).toContain("size-9");
    expect(bars(rows[0])[3].className).toContain("rounded-md");

    const plain = render(<SkeletonRows label="Loading models" rows={2} meta={false} />);
    expect(bars(plain.querySelector("[data-skeleton-row]")!)).toHaveLength(1);
  });

  it("frames rows like a settings group, or not at all inside one", () => {
    const framed = render(<SkeletonRows label="Loading settings" />).querySelector('[role="status"]')!;
    expect(framed.className).toContain("rounded-xl");
    expect(framed.className).toContain("border-border-card");
    expect(framed.className).toContain("divide-y");

    const bare = render(<SkeletonRows label="Loading agents" framed={false} />).querySelector('[role="status"]')!;
    expect(bare.className).not.toContain("rounded-xl");
    expect(bare.className).toContain("divide-y");
  });

  it("drops its own status region when a parent region already names the wait", () => {
    const view = render(<SkeletonRegion label="Loading settings"><SkeletonRows rows={2} /></SkeletonRegion>);
    expect(view.querySelectorAll('[role="status"]')).toHaveLength(1);
    expect(view.querySelectorAll("[data-skeleton-row]")).toHaveLength(2);
  });

  it("staggers row by row, shares a delay within a row, and caps the stagger", () => {
    const view = render(<SkeletonRows label="Loading list" rows={10} lead="dot" />);
    const rows = [...view.querySelectorAll("[data-skeleton-row]")];
    const delays = rows.map(row => new Set(bars(row).map(bar => bar.className.match(/\[animation-delay:\d+ms\]/)?.[0])));
    expect(delays.every(set => set.size === 1)).toBe(true);
    expect([...delays[0]][0]).toBe("[animation-delay:0ms]");
    expect([...delays[1]][0]).toBe("[animation-delay:90ms]");
    expect([...delays[9]][0]).toBe(skeletonDelay(7));
    expect(skeletonDelay(-3)).toBe("[animation-delay:0ms]");
  });

  it("ends prose on a ragged line", () => {
    const view = render(<SkeletonRegion label="Loading note"><SkeletonText lines={4} /></SkeletonRegion>);
    const lines = bars(view);
    expect(lines).toHaveLength(4);
    expect(lines[3].className).toContain("w-3/5");
    expect(lines[0].className).toContain("w-full");
  });

  it("lays cards out in a grid", () => {
    const view = render(<SkeletonCards label="Loading insights" count={6} />);
    const region = view.querySelector('[role="status"]')!;
    expect(region.className).toContain("grid");
    expect(bars(region)).toHaveLength(6);
  });

  it("lets a caller reshape a bar without losing the shimmer", () => {
    const view = render(<SkeletonRegion label="Loading"><Skeleton className="h-10 rounded-2xl" /></SkeletonRegion>);
    const [bar] = bars(view);
    expect(bar.className).toContain("h-10");
    expect(bar.className).not.toMatch(/\bh-3\b/);
    expect(bar.className).toContain("rounded-2xl");
  });
});
