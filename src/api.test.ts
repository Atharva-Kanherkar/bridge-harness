import { describe, expect, it } from "vitest";
import { bridgeApi } from "./api";

describe("SQLite-shaped mock observability", () => {
  it("keeps completion contracts private by default and preserves waiver evidence", async () => {
    const planned = await bridgeApi.createCompletionPlan("session-1", ["User flow works"], ["src/App.tsx"], []);
    expect(planned.markdownCommitted).toBe(false);
    expect(planned.totalRequired).toBeGreaterThan(0);
    await expect(bridgeApi.waiveCompletion(planned.attemptId, ["user-journey"], "Browser unavailable")).rejects.toThrow("every unresolved required check");
    const waived = await bridgeApi.waiveCompletion(planned.attemptId, ["scrutiny", "user-journey"], "Browser unavailable");
    expect(waived.verdict).toBe("waived");
    expect(waived.waiverReason).toBe("Browser unavailable");
  });

  it("lets skills contribute verifier instructions without hiding missing tools", async () => {
    await bridgeApi.registerVerifierManifest("skill:review-checkpoint", { id:"browser-journey",kind:"user_testing",triggers:["frontend"],requiredCapabilities:["browser","network_inspection"],differentModelFamily:true,checks:["exercise user journey"],evidenceRequired:["trace"] });
    const [blocked] = await bridgeApi.verifierCandidates(["frontend"], ["browser"]);
    expect(blocked.eligible).toBe(false);
    expect(blocked.exclusionReasons[0]).toContain("network_inspection");
  });

  it("round-trips workspace learning-router preferences", async () => {
    const defaults = await bridgeApi.routerPreferences("demo-1");
    expect(defaults).toMatchObject({ mode: "shadow", minimumPassBps: 6500 });
    const saved = await bridgeApi.updateRouterPreferences("demo-1", {
      ...defaults,
      pinnedHarness: "claude",
      excludedModels: ["opus"],
    });
    expect(saved.pinnedHarness).toBe("claude");
    expect((await bridgeApi.routerPreferences("demo-1")).excludedModels).toEqual(["opus"]);
  });

  it("routes browser task classes through the documented exception order", async () => {
    const base = { structuredApiAvailable: false, needsUserAuth: false, needsIsolation: false, needsParallelism: false, needsGeoOrProxy: false, unattended: false, domControlAvailable: true, remoteProviderConfigured: false, taskClass: null };
    expect((await bridgeApi.routeBrowser({ ...base, structuredApiAvailable: true, needsUserAuth: true })).route).toBe("mcp_api");
    expect((await bridgeApi.routeBrowser({ ...base, needsUserAuth: true })).route).toBe("attached_tab");
    expect((await bridgeApi.routeBrowser({ ...base, needsUserAuth: true, unattended: true })).route).toBe("remote_browser");
    expect((await bridgeApi.routeBrowser({ ...base, needsIsolation: true })).route).toBe("local_headless");
    expect((await bridgeApi.routeBrowser({ ...base, domControlAvailable: false })).route).toBe("computer_use");
  });

  it("applies model changes to orchestrator chats and rejects active-turn switches", async () => {
    const created = await bridgeApi.createWorkspaceSession("demo-1");
    const orchestrator = [...created.sessions].reverse().find(session => session.workspaceId === "demo-1" && session.kind === "orchestrator")!;
    const changed = await bridgeApi.updateChatModel(orchestrator.id, "claude", "opus");
    expect(changed.sessions.find(session => session.id === orchestrator.id)).toMatchObject({ harness: "claude", model: "opus", status: "idle", providerSessionId: null, restorationMode: "fresh" });

    await expect(bridgeApi.updateChatModel("session-1", "claude", "opus")).rejects.toThrow("current response");
  });

  it("persists catalog-derived model setup as immutable versions", async () => {
    const recommended = await bridgeApi.recommendedModelProfiles();
    expect(recommended).toHaveLength(9);
    const first = await bridgeApi.saveModelProfiles(recommended);
    expect(first).toMatchObject({ complete: true, activeVersion: 1 });
    expect(first.profiles.find(profile => profile.purpose === "reviewer")?.canonicalRole).toBe("verification");
    const reset = await bridgeApi.resetModelProfiles();
    expect(reset.activeVersion).toBe(2);
  });

  it("uses one learning runner and reports duplicate triggers as no-ops", async () => {
    const first = await bridgeApi.runLearning("manual");
    expect(first).toMatchObject({ status: "noop", duplicate: false });
    expect(first.report?.recommendationOnly).toBe(true);
    const duplicate = await bridgeApi.runLearning("in_app");
    expect(duplicate.id).toBe(first.id);
    expect(duplicate.duplicate).toBe(true);
  });

  it("replays forks, compaction, queue conflict, restoration and conversation-only rewind", async () => {
    const initial = await bridgeApi.sessionForest("session-1");
    expect(initial.leaves.map(entry => entry.id)).toEqual(["entry-5a", "entry-raw"]);
    expect(initial.entries.some(entry => entry.kind === "compaction")).toBe(true);
    expect(initial.head?.restorationMode).toBe("hot");
    expect(initial.workerQueue[0].request.reason).toBe("owned_path_conflict");
    expect(initial.workerRuntimes.some(worker => worker.lastResult?.summary === "All 42 auth tests pass")).toBe(true);

    const entryCount = initial.entries.length;
    const rewound = await bridgeApi.activateSessionEntry("session-1", "entry-5a");
    expect(rewound.head?.activeEntryId).toBe("entry-5a");
    expect(rewound.entries).toHaveLength(entryCount);
    expect(rewound.reasons[0].body).toContain("files were not changed");

    await bridgeApi.compactSession("session-1");
    const compacted = await bridgeApi.sessionForest("session-1");
    expect(compacted.entries.filter(entry => entry.kind === "compaction")).toHaveLength(2);
    expect(compacted.head?.latestCheckpointEntryId).toMatch(/^checkpoint-/);
  });
});
