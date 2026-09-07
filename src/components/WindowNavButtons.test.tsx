// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { WindowNavButtons } from "./WindowNavButtons";

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
  vi.restoreAllMocks();
});

describe("WindowNavButtons", () => {
  it("toggles the panel and walks history from the chevrons", () => {
    const onToggleCollapsed = vi.fn();
    const onBack = vi.fn();
    const onForward = vi.fn();
    act(() => {
      root.render(
        <WindowNavButtons
          collapsed={false}
          onToggleCollapsed={onToggleCollapsed}
          canBack
          canForward
          onBack={onBack}
          onForward={onForward}
        />,
      );
    });
    const panel = container.querySelector<HTMLButtonElement>('button[aria-label="Hide sidebar"]')!;
    const back = container.querySelector<HTMLButtonElement>('button[aria-label="Back"]')!;
    const forward = container.querySelector<HTMLButtonElement>('button[aria-label="Forward"]')!;
    act(() => {
      panel.dispatchEvent(new MouseEvent("click", { bubbles: true }));
      back.dispatchEvent(new MouseEvent("click", { bubbles: true }));
      forward.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    expect(onToggleCollapsed).toHaveBeenCalledTimes(1);
    expect(onBack).toHaveBeenCalledTimes(1);
    expect(onForward).toHaveBeenCalledTimes(1);
  });

  it("spreads the panel left and the chevrons right", () => {
    act(() => {
      root.render(
        <div className="flex">
          <WindowNavButtons
            spread
            collapsed={false}
            onToggleCollapsed={() => {}}
            canBack
            canForward
            onBack={() => {}}
            onForward={() => {}}
          />
        </div>,
      );
    });
    const panel = container.querySelector<HTMLButtonElement>('button[aria-label="Hide sidebar"]')!;
    const chevrons = container.querySelector<HTMLButtonElement>('button[aria-label="Back"]')!.parentElement!;
    expect(panel.compareDocumentPosition(chevrons) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(chevrons.className).toContain("flex");
    expect(chevrons.parentElement?.className).toContain("ml-auto");
  });

  it("disables chevrons when there is nowhere to go", () => {
    act(() => {
      root.render(
        <WindowNavButtons
          collapsed
          onToggleCollapsed={() => {}}
          canBack={false}
          canForward={false}
          onBack={() => {}}
          onForward={() => {}}
        />,
      );
    });
    expect(container.querySelector<HTMLButtonElement>('button[aria-label="Back"]')?.disabled).toBe(true);
    expect(container.querySelector<HTMLButtonElement>('button[aria-label="Forward"]')?.disabled).toBe(true);
    expect(container.querySelector('button[aria-label="Show sidebar"]')).toBeTruthy();
  });
});
