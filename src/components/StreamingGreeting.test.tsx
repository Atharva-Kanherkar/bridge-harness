// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { StreamingGreeting, tokenBoundaries } from "./StreamingGreeting";
import type { GreetingPart } from "../greetings";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const line = (before: string, after = "?"): GreetingPart[] => [
  { text: before, kind: "text" },
  { text: "harness", kind: "project" },
  { text: after, kind: "text" },
];

describe("tokenBoundaries", () => {
  it("lands short words whole and splits long ones into model-sized pieces", () => {
    const text = "What should we refactor in harness?";
    const ends = tokenBoundaries(text);
    expect(ends.at(-1)).toBe(text.length);
    expect(ends).toEqual([...ends].sort((a, b) => a - b));
    expect(text.slice(0, ends[0])).toBe("What");
    expect(ends.some(end => text.slice(0, end) === "What should we refa")).toBe(true);
  });
});

describe("StreamingGreeting", () => {
  let container: HTMLDivElement;
  let root: Root;
  beforeEach(() => {
    vi.useFakeTimers();
    container = document.createElement("div");
    document.body.append(container);
    root = createRoot(container);
  });
  afterEach(() => {
    act(() => root.unmount());
    container.remove();
    vi.useRealTimers();
  });

  const visible = () => [...container.querySelectorAll("span[aria-hidden='true'].col-start-1:not(.invisible) span:not(.invisible)")]
    .filter(node => node.children.length === 0 && !node.className.includes("animate-caret"))
    .map(node => node.textContent).join("");
  // Each timer schedules the next only after React commits, so step and flush.
  const advance = (ms: number) => {
    for (let left = ms; left > 0; left -= 10) act(() => vi.advanceTimersByTime(Math.min(10, left)));
  };
  const spoken = () => container.querySelector(".sr-only")?.textContent;

  it("streams a line token by token, then rotates to the next", () => {
    act(() => root.render(<StreamingGreeting lines={[line("What should we build in "), line("Where are the sharp edges in ")]} projectClassName="font-medium" />));
    expect(visible()).toBe("");
    expect(spoken()).toBe("What should we build in harness?");
    advance(300);
    expect(visible()).toBe("What");
    expect(container.querySelector(".animate-caret-blink")).not.toBeNull();
    // Stream to the end, then hold the finished line before fading it.
    for (let step = 0; step < 400 && visible() !== "What should we build in harness?"; step += 1) advance(10);
    expect(visible()).toBe("What should we build in harness?");
    expect(container.querySelector(".font-medium")?.textContent).toBe("harness");
    expect(container.querySelector(".animate-caret-blink")).toBeNull();
    advance(3380);
    expect(container.querySelector(".opacity-0")).toBeNull();
    advance(30);
    expect(container.querySelector(".opacity-0")).not.toBeNull();
    advance(320);
    expect(spoken()).toBe("Where are the sharp edges in harness?");
    expect(visible()).toBe("");
    advance(2500);
    expect(visible()).toBe("Where are the sharp edges in harness?");
  });

  it("keeps a single line on screen once streamed", () => {
    act(() => root.render(<StreamingGreeting lines={[line("What's next for ")]} />));
    advance(20000);
    expect(visible()).toBe("What's next for harness?");
    expect(container.querySelector(".opacity-0")).toBeNull();
  });
});
