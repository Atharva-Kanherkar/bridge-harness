// @vitest-environment jsdom
// The usage heatmap: one cell per period, ink by share of the window's peak,
// weeks as columns for day windows and one row for the 24-hour window. It
// reads the same period report as the area chart and changes nothing about it.
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import type { PeriodReport } from "../usageReport";
import { dominantHarness, heatStep, layoutCells, UsageHeatmap, weekdayIndex } from "./UsageHeatmap";

function period(period: string, tokens: number, cost = tokens, harness = "claude"): PeriodReport {
  return { period, tokens, costMicrousd: cost, costByHarness: { [harness]: cost }, tokensByHarness: { [harness]: tokens } };
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

  it("lays days out as a calendar: Monday-first weekday columns, one row per week", () => {
    expect(weekdayIndex("2026-09-07")).toBe(0); // a Monday
    expect(weekdayIndex("2026-09-13")).toBe(6);
    const { cells, columns, rows } = layoutCells([period("2026-09-05", 1), period("2026-09-06", 2), period("2026-09-07", 3)], "day", "tokens");
    expect(columns).toBe(7);
    expect(rows).toBe(2);
    expect(cells[0]).toMatchObject({ column: 5, row: 0 });
    expect(cells[2]).toMatchObject({ column: 0, row: 1, value: 3, harness: "claude" });
  });

  it("names the harness that carried most of a period, by the chosen metric", () => {
    const mixed: PeriodReport = { period: "2026-09-01", tokens: 10, costMicrousd: 10, tokensByHarness: { codex: 7, claude: 3 }, costByHarness: { codex: 2, claude: 8 } };
    expect(dominantHarness(mixed, "tokens")).toBe("codex");
    expect(dominantHarness(mixed, "cost")).toBe("claude");
    expect(dominantHarness({ ...mixed, tokensByHarness: {} }, "tokens")).toBeNull();
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

  it("renders a cell per period, coloured by harness, with a readable label and a hover readout", () => {
    const periods = [period("2026-09-07", 0), period("2026-09-08", 500_000, 500_000, "codex"), period("2026-09-09", 1_000_000)];
    act(() => { root.render(<UsageHeatmap periods={periods} resolution="day" timeZone="UTC" metric="tokens" />); });
    const cells = container.querySelectorAll('[role="gridcell"]');
    expect(cells).toHaveLength(3);
    expect(cells[0].getAttribute("data-step")).toBe("0");
    expect(cells[2].getAttribute("data-step")).toBe("4");
    expect(cells[2].getAttribute("aria-label")).toBe("Sep 9: 1M, mostly Claude");
    // Colour follows the harness that did the work; an empty day is grey.
    expect(cells[0].className).toContain("bg-muted");
    expect(cells[1].className).toContain("bg-chart-codex");
    expect(cells[2].className).toContain("bg-chart-claude");
    act(() => { cells[1].dispatchEvent(new MouseEvent("mouseover", { bubbles: true })); });
    expect(container.querySelector('[role="tooltip"]')?.textContent).toContain("500K");
    expect(container.querySelector('[role="tooltip"]')?.textContent).toContain("mostly Codex");
    // Legend names the harnesses present and the depth ramp.
    expect(container.textContent).toContain("Claude");
    expect(container.textContent).toContain("less");
  });
});
