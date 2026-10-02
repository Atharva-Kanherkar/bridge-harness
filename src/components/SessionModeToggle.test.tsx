// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { SessionModeToggle, sessionModeDescription } from "./SessionModeToggle";
import type { WorkspaceSessionKind } from "../types";

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

function render(value: WorkspaceSessionKind, onChange = vi.fn(), disabled = false) {
  act(() => root.render(<SessionModeToggle value={value} onChange={onChange} disabled={disabled} describedBy="mode-hint" />));
  return onChange;
}
const radios = () => [...container.querySelectorAll<HTMLButtonElement>('[role="radio"]')];

describe("SessionModeToggle", () => {
  it("is a labelled radiogroup reflecting the value", () => {
    render("orchestrator");
    const group = container.querySelector('[role="radiogroup"]')!;
    expect(group.getAttribute("aria-label")).toBe("Chat mode");
    expect(group.getAttribute("aria-describedby")).toBe("mode-hint");
    expect(radios().map(radio => [radio.textContent, radio.getAttribute("aria-checked"), radio.tabIndex]))
      .toEqual([["Orchestrator", "true", 0], ["Direct", "false", -1]]);
    expect(radios()[1].title).toBe(sessionModeDescription("direct"));
  });

  it("clicking an option selects it", () => {
    const onChange = render("orchestrator");
    act(() => radios()[1].click());
    expect(onChange).toHaveBeenCalledWith("direct");
  });

  it("arrow keys move the selection and wrap", () => {
    const onChange = render("direct");
    const key = (target: HTMLElement, name: string) => act(() => { target.dispatchEvent(new KeyboardEvent("keydown", { key: name, bubbles: true })); });
    key(radios()[1], "ArrowRight");
    expect(onChange).toHaveBeenLastCalledWith("orchestrator");
    key(radios()[1], "ArrowLeft");
    expect(onChange).toHaveBeenLastCalledWith("orchestrator");
    key(radios()[0], "ArrowLeft");
    expect(onChange).toHaveBeenLastCalledWith("direct");
  });

  it("disabled blocks changes", () => {
    const onChange = render("orchestrator", vi.fn(), true);
    expect(radios().every(radio => radio.disabled)).toBe(true);
    act(() => radios()[1].click());
    expect(onChange).not.toHaveBeenCalled();
  });

  it("explains both modes", () => {
    expect(sessionModeDescription("orchestrator")).toContain("delegates to workers");
    expect(sessionModeDescription("direct")).toContain("no Bridge orchestration");
  });
});
