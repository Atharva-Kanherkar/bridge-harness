import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { ModelSetupWizard } from "./ModelSetupWizard";
import { evaluatorExecutionLabel, LearningRunSummary, RouterSettingsDialog, scheduleUserFieldsChanged } from "./RouterSettingsDialog";
import type { AdapterDescriptor, LearningRun, LearningSchedule, LearningState } from "../types";
import { ModelProfileEditor } from "./ModelProfileEditor";
import { recommendedProfileDrafts } from "../modelProfiles";

const adapters: AdapterDescriptor[] = [{
  id: "catalog", label: "Catalog", available: true, authState: "signed_in", version: "1", capabilities: [], unavailableReason: null, defaultModel: "balanced",
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
      evaluatedSpendMicrousd: 0, evaluatedTokens: 0, evaluationExecution: "deterministic_only", replayPassed: true,
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
    rollbackTargetVersion: null,
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
    expect(html).toContain("helper picker");
    expect(html).toContain("memory engine");
    expect(html).toContain("/memory");
    expect(html).toContain("/memories");
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
    expect(ask).toContain("deterministic only — no model evaluation requested");
    expect(ask).not.toContain("Evaluator usage:");

    const canary = learningState({ promotionStatus: "canary" });
    canary.activePolicyVersion = 2;
    canary.canaryPolicyVersion = 2;
    canary.rollbackTargetVersion = 1;
    const canaryHtml = renderToStaticMarkup(<LearningRunSummary learning={canary} />);
    expect(canaryHtml).toContain("canary");
    expect(canaryHtml).toContain("Roll back to v1");
  });

  it("rolls back to the live predecessor, not the latest run base", () => {
    const later = learningState({ basePolicyVersion: 2, promotionStatus: "noop" });
    later.activePolicyVersion = 2;
    later.rollbackTargetVersion = 1;
    if (later.latestRun?.report) later.latestRun.report.basePolicyVersion = 2;
    const html = renderToStaticMarkup(<LearningRunSummary learning={later} />);
    expect(html).toContain("Roll back to v1");
    expect(html).not.toContain("Roll back to v2");

    const noTarget = learningState();
    noTarget.activePolicyVersion = 2;
    noTarget.rollbackTargetVersion = null;
    expect(renderToStaticMarkup(<LearningRunSummary learning={noTarget} />)).not.toContain("Roll back");
  });

  it("a workspace with no runs yet says so instead of a lone button", () => {
    const state = learningState();
    const html = renderToStaticMarkup(
      <LearningRunSummary learning={{ ...state, latestRun: null }} />
    );
    expect(html).toContain("No learning runs yet");
    expect(html).toContain("conservative priors");
  });

  it("labels each evaluator execution state truthfully", () => {
    expect(evaluatorExecutionLabel("not_run")).toBe("not run");
    expect(evaluatorExecutionLabel("deferred")).toBe("not run");
    expect(evaluatorExecutionLabel("queued")).toBe("queued — bounded model evaluation has not run yet");
    expect(evaluatorExecutionLabel("executed")).toBe("bounded model evaluation executed");
    expect(evaluatorExecutionLabel("evaluation_failed")).toBe("bounded model evaluation reached no verdict");
    expect(evaluatorExecutionLabel("deterministic_only")).toBe("deterministic only — no model evaluation requested");
    expect(evaluatorExecutionLabel("reused_existing_evidence")).toBe("reused existing evidence");
  });

  it("treats schedule nextRunAt as runner-owned and the evaluator ceilings as user-editable", () => {
    const schedule: LearningSchedule = { jobId: "default", enabled: false, cadenceMinutes: 1440, nextRunAt: null, runBudgetMicrousd: 100_000, runBudgetTokens: 50_000, mode: "ask" };
    expect(scheduleUserFieldsChanged(schedule, { ...schedule, nextRunAt: "stale" })).toBe(false);
    expect(scheduleUserFieldsChanged(schedule, { ...schedule, runBudgetMicrousd: 0, runBudgetTokens: 0 })).toBe(true);
    expect(scheduleUserFieldsChanged(schedule, { ...schedule, mode: "automatic" })).toBe(true);
    expect(scheduleUserFieldsChanged(schedule, { ...schedule, enabled: true })).toBe(true);
    expect(scheduleUserFieldsChanged(schedule, { ...schedule, cadenceMinutes: 60 })).toBe(true);
  });
});
