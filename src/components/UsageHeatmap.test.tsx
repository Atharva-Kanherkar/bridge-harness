// @vitest-environment jsdom
// The usage heatmap: one cell per period, ink by share of the window's peak,
// weeks as columns for day windows and one row for the 24-hour window. It
// reads the same period report as the area chart and changes nothing about it.
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import type { PeriodReport } from "../usageReport";
import { heatStep, layoutCells, UsageHeatmap, weekdayIndex } from "./UsageHeatmap";

function period(period: string, tokens: number, cost = tokens): PeriodReport {
  return { period, tokens, costMicrousd: cost, costByHarness: {}, tokensByHarness: {} };
}

describe("heatmap layout", () => {
  it("steps values against the peak and treats zero as empty", () => {
    expect(heatStep(0, 100)).toBe(0);
    expect(heatStep(10, 100)).toBe(1);
    expect(heatStep(50, 100)).toBe(2);
    expect(heatStep(75, 100)).toBe(3);
    expect(heatStep(100, 100)).toBe(4);
    expect(heatStep(5, 0)).toBe(0);
  });

  it("places days in Monday-first weekday rows and week columns", () => {
    expect(weekdayIndex("2026-09-07")).toBe(0); // a Monday
    expect(weekdayIndex("2026-09-13")).toBe(6);
    const { cells, columns, rows } = layoutCells([period("2026-09-05", 1), period("2026-09-06", 2), period("2026-09-07", 3)], "day", "tokens");
    expect(rows).toBe(7);
    expect(columns).toBe(2);
    expect(cells[0]).toMatchObject({ column: 0, row: 5 });
    expect(cells[2]).toMatchObject({ column: 1, row: 0, value: 3 });
  });

  it("lays the hourly window out as one row and reads cost when asked", () => {
    const { cells, rows } = layoutCells([period("2026-09-09T00:00:00Z", 1, 700), period("2026-09-09T01:00:00Z", 2, 900)], "hour", "cost");
    expect(rows).toBe(1);
    expect(cells.map(cell => cell.value)).toEqual([700, 900]);
    expect(cells[1].column).toBe(1);
  });
});

describe("UsageHeatmap", () => {
  let container: HTMLDivElement;
  let root: Root;
  beforeEach(() => {
    (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    container = document.createElement("div");
    document.body.append(container);
    root = createRoot(container);
  });
  afterEach(() => { act(() => root.unmount()); container.remove(); });

  it("renders a cell per period with a readable label and a hover readout", () => {
    const periods = [period("2026-09-07", 0), period("2026-09-08", 500_000), period("2026-09-09", 1_000_000)];
    act(() => { root.render(<UsageHeatmap periods={periods} resolution="day" timeZone="UTC" metric="tokens" />); });
    const cells = container.querySelectorAll('[role="gridcell"]');
    expect(cells).toHaveLength(3);
    expect(cells[0].getAttribute("data-step")).toBe("0");
    expect(cells[2].getAttribute("data-step")).toBe("4");
    expect(cells[2].getAttribute("aria-label")).toBe("Sep 9: 1M");
    act(() => { cells[1].dispatchEvent(new MouseEvent("mouseover", { bubbles: true })); });
    expect(container.querySelector('[role="tooltip"]')?.textContent).toContain("500K");
    expect(container.textContent).toContain("Each cell is a day");
    // Sequential ramp: one hue, no series colour.
    expect(container.innerHTML).not.toMatch(/chart-(codex|claude|cursor|opencode)/);
  });
});
