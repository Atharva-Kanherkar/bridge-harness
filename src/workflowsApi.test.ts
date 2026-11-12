import { describe, expect, it } from "vitest";
import { bridgeApi } from "./api";
import type { WorkflowDefinition } from "./protocol/generated/protocol";

const definition = (): WorkflowDefinition => ({
  name: "Release notes",
  projectId: null,
  steps: [{ harness: "claude", model: "sonnet", instructions: "Write release notes from merged PRs." }],
  trigger: { kind: "manual" },
  grants: [{ server: "claude.ai Slack", read: true, write: "never" }],
  alwaysAllow: [],
  files: "none",
  budget: { maxWallSeconds: 600, maxTurns: 24, maxToolCalls: 64, maxOutputTokens: null, costCeilingMicrousd: null },
  expectedResult: "",
  notify: { onSuccess: true },
});

describe("workflows in mock mode", () => {
  it("creates, lists, approves, narrows, widens and deletes like the Rust store", async () => {
    const created = await bridgeApi.saveWorkflow(null, definition());
    expect(created.needsApproval).toBe(true);
    expect((await bridgeApi.listWorkflows()).workflows.map(row => row.id)).toContain(created.workflow.id);

    const approved = await bridgeApi.approveWorkflow(created.workflow.id, created.workflow.revisionId);
    expect(approved.workflow.approved).toBe(true);

    const reworded = await bridgeApi.saveWorkflow(created.workflow.id, { ...definition(), name: "Release notes v2" });
    expect(reworded.needsApproval).toBe(false);
    expect(reworded.workflow.revision).toBe(2);

    const widened = await bridgeApi.saveWorkflow(created.workflow.id, { ...definition(), grants: [{ server: "claude.ai Slack", read: true, write: "ask" }] });
    expect(widened.needsApproval).toBe(true);
    await expect(bridgeApi.approveWorkflow(created.workflow.id, reworded.workflow.revisionId)).rejects.toThrow(/changed since/);

    await bridgeApi.deleteWorkflow(created.workflow.id);
    expect((await bridgeApi.listWorkflows()).workflows.map(row => row.id)).not.toContain(created.workflow.id);
  });

  it("offers Codex and OpenCode without connectors, with the gate's reason", async () => {
    const options = await bridgeApi.workflowOptions();
    const codex = options.harnesses.find(harness => harness.id === "codex");
    expect(codex?.connectorsAllowed).toBe(false);
    expect(codex?.connectorsReason).toMatch(/per-tool/);
    expect(options.servers.length).toBeGreaterThan(0);
  });
});

describe("workflow runs in mock mode", () => {
  it("makes one run per request id and lists it", async () => {
    const created = await bridgeApi.saveWorkflow(null, definition());
    await expect(bridgeApi.runWorkflow(created.workflow.id, "r1")).rejects.toThrow(/approval/);
    const dry = await bridgeApi.runWorkflow(created.workflow.id, "dry-1", true);
    expect(dry.created).toBe(true);
    expect(dry.run.dryRun).toBe(true);
    const again = await bridgeApi.runWorkflow(created.workflow.id, "dry-1", true);
    expect(again.created).toBe(false);
    expect(again.run.id).toBe(dry.run.id);
    const runs = await bridgeApi.listWorkflowRuns({ workflowId: created.workflow.id, limit: 10 });
    expect(runs.runs.map(run => run.id)).toEqual([dry.run.id]);
  });

  it("keeps settings in range", async () => {
    const written = await bridgeApi.writeWorkflowSettings({ paused: false, quotaReservePercent: 40 });
    expect(written.configured).toBe(true);
    await expect(bridgeApi.writeWorkflowSettings({ paused: false, quotaReservePercent: 95 })).rejects.toThrow(/between 0 and 90/);
  });
});
