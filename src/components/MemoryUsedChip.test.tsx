// @vitest-environment jsdom
// The chip is audit-backed and absent at zero; clicking it opens Memory.
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

function mount(value: MemoryPacketAudit | null, onOpenMemory = () => {}) {
  act(() => {
    root.render(<MemoryUsedChip audit={value} onOpenMemory={onOpenMemory} />);
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

  it("counts from the audit without rendering a floating memory block", () => {
    mount(audit(2));
    expect(container.textContent).toContain("Memory used (2)");
    expect(container.textContent).not.toContain("Pinned fact 0");
    expect(container.textContent).not.toContain("explicit pin");
    const button = container.querySelector("button")!;
    expect(button.getAttribute("aria-label")).toBe("Open Memory, 2 memories used in this session");
  });

  it("opens Bridge Memory when the chip is clicked", () => {
    const onOpenMemory = vi.fn();
    mount(audit(3), onOpenMemory);
    expect(container.textContent).toContain("Memory used (3)");
    act(() => {
      container.querySelector("button")!.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    expect(onOpenMemory).toHaveBeenCalledOnce();
  });
});
