// @vitest-environment jsdom
// The chip is audit-backed and absent at zero; its disclosure names each
// injected item and why it was selected.
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { MemoryPacketAudit } from "../types";
import { MemoryUsedChip } from "./MemoryUsedChip";

const audit = (count: number): MemoryPacketAudit => ({
  sessionId: "s",
  selected: Array.from({ length: count }, (_, index) => ({
    recordId: `r-${index}`,
    body: `Pinned fact ${index}`,
    kind: "preference",
    reason: "explicit pin",
  })),
  tokenEstimate: count * 12,
});

let container: HTMLDivElement;
let root: Root;

function mount(value: MemoryPacketAudit | null, open = false, onToggle = () => {}) {
  act(() => {
    root.render(<MemoryUsedChip audit={value} open={open} onToggle={onToggle} />);
  });
}

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

describe("MemoryUsedChip", () => {
  it("does not render at zero, or without an audit", () => {
    mount(null);
    expect(container.textContent).toBe("");
    mount(audit(0));
    expect(container.textContent).toBe("");
  });

  it("counts from the audit and disclosure lists each item with its reason", () => {
    mount(audit(2), true);
    expect(container.textContent).toContain("Memory used (2)");
    expect(container.textContent).toContain("Pinned fact 0");
    expect(container.textContent).toContain("explicit pin");
    const button = container.querySelector("button")!;
    expect(button.getAttribute("aria-expanded")).toBe("true");
  });

  it("closed keeps the count visible and the items private", () => {
    const onToggle = vi.fn();
    mount(audit(3), false, onToggle);
    expect(container.textContent).toContain("Memory used (3)");
    expect(container.textContent).not.toContain("Pinned fact 0");
    act(() => {
      container.querySelector("button")!.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    expect(onToggle).toHaveBeenCalledOnce();
  });
});
