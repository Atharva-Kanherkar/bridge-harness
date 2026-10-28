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
