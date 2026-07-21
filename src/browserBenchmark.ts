export type BrowserBenchmarkProvider = "attached_tab" | "playwright_mcp" | "browser_use" | "computer_use";

export interface BrowserBenchmarkRun {
  scenario: string; provider: BrowserBenchmarkProvider; success: boolean; latencyMs: number;
  inputTokens: number; outputTokens: number; directCostUsd: number; interventions: number;
  approvals: number; duplicateSideEffects: number; evidenceVerified: boolean;
}

export interface BrowserBenchmarkSummary {
  provider: BrowserBenchmarkProvider; runs: number; successRate: number; verifiedRate: number;
  medianLatencyMs: number; averageInputTokens: number; averageCostUsd: number;
  interventionRate: number; duplicateSideEffects: number;
}

export function summarizeBrowserBenchmark(runs: BrowserBenchmarkRun[]): BrowserBenchmarkSummary[] {
  const providers: BrowserBenchmarkProvider[] = ["attached_tab", "playwright_mcp", "browser_use", "computer_use"];
  return providers.map(provider => {
    const selected = runs.filter(run => run.provider === provider);
    const latencies = selected.map(run => run.latencyMs).sort((a, b) => a - b);
    return {
      provider,
      runs: selected.length,
      successRate: ratio(selected.filter(run => run.success).length, selected.length),
      verifiedRate: ratio(selected.filter(run => run.evidenceVerified).length, selected.length),
      medianLatencyMs: latencies.length ? latencies[Math.floor((latencies.length - 1) / 2)] : 0,
      averageInputTokens: average(selected.map(run => run.inputTokens)),
      averageCostUsd: average(selected.map(run => run.directCostUsd)),
      interventionRate: ratio(selected.filter(run => run.interventions > 0).length, selected.length),
      duplicateSideEffects: selected.reduce((total, run) => total + run.duplicateSideEffects, 0),
    };
  });
}

export function validateBrowserBenchmarkCoverage(runs: BrowserBenchmarkRun[]): string[] {
  const errors: string[] = [];
  const scenarios = new Set(runs.map(run => run.scenario));
  for (const scenario of scenarios) {
    for (const provider of ["attached_tab", "playwright_mcp", "browser_use", "computer_use"] as const) {
      if (!runs.some(run => run.scenario === scenario && run.provider === provider)) errors.push(`${scenario} is missing ${provider}`);
    }
  }
  for (const run of runs) {
    if (run.latencyMs < 0 || run.inputTokens < 0 || run.outputTokens < 0 || run.directCostUsd < 0) errors.push(`${run.scenario}/${run.provider} has negative measurements`);
    if (run.success && !run.evidenceVerified) errors.push(`${run.scenario}/${run.provider} claims success without verified evidence`);
  }
  return errors;
}

function average(values: number[]): number { return values.length ? values.reduce((sum, value) => sum + value, 0) / values.length : 0; }
function ratio(numerator: number, denominator: number): number { return denominator ? numerator / denominator : 0; }
