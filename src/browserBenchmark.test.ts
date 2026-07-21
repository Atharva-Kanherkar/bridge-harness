import { describe, expect, it } from "vitest";
import fixture from "../testing/fixtures/browser-bridge-benchmark-v1.json";
import { summarizeBrowserBenchmark, validateBrowserBenchmarkCoverage, type BrowserBenchmarkRun } from "./browserBenchmark";

const runs = fixture.runs as BrowserBenchmarkRun[];

describe("browser bridge benchmark", () => {
  it("requires equivalent evidence-backed runs across all four browser approaches", () => {
    expect(validateBrowserBenchmarkCoverage(runs)).toEqual([]);
    expect(summarizeBrowserBenchmark(runs).map(summary => summary.provider)).toEqual(["attached_tab", "playwright_mcp", "browser_use", "computer_use"]);
  });

  it("tracks duplicate side effects as a first-class failure metric", () => {
    const invalid = [...runs, { ...runs[0], scenario: "duplicate", success: true, evidenceVerified: false, duplicateSideEffects: 1 }];
    expect(validateBrowserBenchmarkCoverage(invalid)).toContain("duplicate/attached_tab claims success without verified evidence");
  });
});
