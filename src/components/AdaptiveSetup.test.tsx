import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { ModelSetupWizard } from "./ModelSetupWizard";
import { LearningRunSummary, RouterSettingsDialog } from "./RouterSettingsDialog";
import type { AdapterDescriptor, LearningRun, LearningState } from "../types";
import { ModelProfileEditor } from "./ModelProfileEditor";
import { recommendedProfileDrafts } from "../modelProfiles";

const adapters: AdapterDescriptor[] = [{
  id: "catalog", label: "Catalog", available: true, version: "1", capabilities: [], unavailableReason: null, defaultModel: "balanced",
  models: [
    { id: "quick", label: "Quick", tier: "fast", defaultForTier: true },
    { id: "balanced", label: "Balanced", tier: "standard", defaultForTier: true },
    { id: "deep", label: "Deep", tier: "strong", defaultForTier: true },
  ],
}];

function learningState(run: Partial<LearningRun> = {}): LearningState {
  const latestRun: LearningRun = {
    id: "run", jobId: "default", triggerKind: "manual", idempotencyKey: "default:10:1",
    evidenceBoundary: 10, basePolicyVersion: 1, status: "completed",
    report: {
      reason: "candidate policy awaits approval", evidenceBoundary: 10, evidenceCount: 10,
      basePolicyVersion: 1, candidatePolicyVersion: 2, qualityBps: 9000,
      averageCostMicrousd: null, averageLatencyMs: 120, retryRateBps: 0,
      interventionRateBps: 0, averageConfidenceBps: 9000, costComplete: false,
      evaluatedSpendMicrousd: 0, evaluatedTokens: 0, replayPassed: true,
      promotionStatus: "awaiting_approval", policyDiff: {}, recommendationOnly: true,
    },
    candidatePolicyVersion: 2, cancellationRequested: false, leaseExpiresAt: null,
    replayPassed: true, promotionStatus: "awaiting_approval", duplicate: false,
    createdAt: "now", completedAt: "now", ...run,
  };
  return {
    schedule: { jobId: "default", enabled: false, cadenceMinutes: 1440, nextRunAt: null, runBudgetMicrousd: 100_000, runBudgetTokens: 50_000, mode: "ask" },
    latestRun,
    activePolicyVersion: 1,
    canaryPolicyVersion: null,
  };
}

describe("adaptive setup surfaces", () => {
  it("puts one-click recommended setup before advanced disclosure", () => {
    const html = renderToStaticMarkup(<ModelSetupWizard adapters={adapters} onComplete={() => undefined} onError={() => undefined} />);
    expect(html).toContain("Use recommended defaults");
    expect(html).toContain("Customize role profiles");
    expect(html).not.toContain("Advanced role profiles");
  });

  it("renders every advanced profile with catalog-supported models only", () => {
    const unavailable: AdapterDescriptor = { ...adapters[0], id: "offline", label: "Offline", available: false, models: [{ id: "unsupported", label: "Unsupported", tier: "standard", defaultForTier: true }] };
    const html = renderToStaticMarkup(<ModelProfileEditor profiles={recommendedProfileDrafts(adapters)} adapters={[...adapters, unavailable]} onChange={() => undefined} />);
    for (const label of ["Standard orchestrator", "Premium orchestrator", "Planner", "Implementer", "Verifier", "Reviewer", "Research", "Documentation", "Model evaluator"]) expect(html).toContain(label);
    expect(html).toContain("Catalog · Balanced");
    expect(html).not.toContain("Unsupported");
  });

  it("explains the single local runner and truthful provider limitations", () => {
    const html = renderToStaticMarkup(<RouterSettingsDialog open workspaceId="workspace" adapters={adapters} onClose={() => undefined} onError={() => undefined} />);
    expect(html).toContain("Run learning now");
    expect(html).toContain("Bridge cannot create or enumerate schedules");
    expect(html).toContain("Cloud Routines remain experimental");
    expect(html).toContain("Open Codex Scheduled setup");
    expect(html).toContain("Role model profiles");
  });

  it("renders duplicate and failed learning jobs as explicit states", () => {
    const duplicate = renderToStaticMarkup(<LearningRunSummary learning={learningState({ duplicate: true, status: "noop" })} />);
    expect(duplicate).toContain("duplicate · no-op");
    expect(duplicate).toContain("Cost comparison is unknown");

    const failed = learningState({ status: "failed", promotionStatus: "failed" });
    if (failed.latestRun?.report) failed.latestRun.report.reason = "database was locked";
    const failedHtml = renderToStaticMarkup(<LearningRunSummary learning={failed} />);
    expect(failedHtml).toContain("failed");
    expect(failedHtml).toContain("database was locked");
  });

  it("shows Ask approval, cancellation, canary, and rollback controls", () => {
    const ask = renderToStaticMarkup(<LearningRunSummary learning={learningState()} />);
    expect(ask).toContain("Approve replayed policy");
    expect(ask).toContain("Cancel candidate");

    const canary = learningState({ promotionStatus: "canary" });
    canary.activePolicyVersion = 2;
    canary.canaryPolicyVersion = 2;
    const canaryHtml = renderToStaticMarkup(<LearningRunSummary learning={canary} />);
    expect(canaryHtml).toContain("canary");
    expect(canaryHtml).toContain("Roll back to v1");
  });
});
