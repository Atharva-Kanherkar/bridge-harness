// @vitest-environment jsdom
// The composer's context ring: the selected session's window percent, its
// pressure tone, an honest empty state, and a click that opens the pane.
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ContextRing } from "./ContextRing";

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

function render(percent: number | null | undefined, onOpen = vi.fn()) {
  act(() => root.render(<ContextRing percent={percent} model="Opus 5.5" onOpen={onOpen} />));
  return container.querySelector("button")!;
}

describe("ContextRing", () => {
  it("shows the percent and names it for assistive tech", () => {
    const button = render(38.4);
    expect(button.textContent).toBe("38%");
    expect(button.getAttribute("aria-label")).toBe("Context window: 38% used");
    expect(button.title).toBe("38% of Opus 5.5's context window");
    expect(container.querySelectorAll("circle")).toHaveLength(2);
  });

  it("draws no fill and no zero when there is no reading", () => {
    for (const value of [null, undefined]) {
      const button = render(value);
      expect(button.textContent).toBe("–");
      expect(button.getAttribute("aria-label")).toBe("Context window: no reading yet");
      expect(button.title).toBe("No context reading yet");
      expect(container.querySelectorAll("circle")).toHaveLength(1);
    }
  });

  it("uses the pressure thresholds for its tone", () => {
    const tone = (percent: number) => render(percent).querySelector("span")!.className;
    expect(tone(74)).toContain("text-foreground");
    expect(tone(75)).toContain("text-warning");
    expect(tone(89)).toContain("text-warning");
    expect(tone(90)).toContain("text-destructive");
  });

  it("opens the pane when clicked", () => {
    const onOpen = vi.fn();
    const button = render(12, onOpen);
    act(() => button.click());
    expect(onOpen).toHaveBeenCalledTimes(1);
  });
});
