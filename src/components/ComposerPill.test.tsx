// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ComposerPill, type ComposerPillProps } from "./ComposerPill";

// The composer is the user's only steering wheel over a working agent, so the
// coverage here is about what its controls *do*, not how they look: the `+`
// performs the action its label names, and a draft is never collateral damage.

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.append(container);
  act(() => {
    root = createRoot(container);
  });
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

const props = (overrides: Partial<ComposerPillProps> = {}): ComposerPillProps => ({
  value: "",
  onChange: () => {},
  onSubmit: () => {},
  ...overrides,
});

function render(overrides: Partial<ComposerPillProps> = {}) {
  act(() => root.render(<ComposerPill {...props(overrides)} />));
}

const plus = () => container.querySelector<HTMLButtonElement>('button[aria-label="New workspace"]')!;
const textarea = () => container.querySelector<HTMLTextAreaElement>("textarea")!;
const stop = () => container.querySelector<HTMLButtonElement>('button[aria-label="Stop"]');

describe("ComposerPill", () => {
  it("runs the named + action and leaves a non-empty draft alone", () => {
    const onPlusClick = vi.fn();
    const onChange = vi.fn();
    render({ value: "keep this draft", onPlusClick, onChange });

    act(() => plus().click());

    expect(onPlusClick).toHaveBeenCalledTimes(1);
    // A control labelled "New workspace" must not double as a draft eraser.
    expect(onChange).not.toHaveBeenCalled();
    expect(textarea().value).toBe("keep this draft");
  });

  it("keeps + reachable while the agent is working", () => {
    const onPlusClick = vi.fn();
    render({ value: "draft", working: true, onPlusClick, onStop: () => {} });

    expect(plus().disabled).toBe(false);
    act(() => plus().click());
    expect(onPlusClick).toHaveBeenCalledTimes(1);
  });

  it("disables + only when the composer itself is disabled or has no handler", () => {
    render({ onPlusClick: () => {}, disabled: true });
    expect(plus().disabled).toBe(true);

    render({});
    expect(plus().disabled).toBe(true);
  });

  it("keeps Stop reachable while working", () => {
    const onStop = vi.fn();
    render({ value: "draft", working: true, onStop });

    act(() => stop()!.click());
    expect(onStop).toHaveBeenCalledTimes(1);
  });
});
