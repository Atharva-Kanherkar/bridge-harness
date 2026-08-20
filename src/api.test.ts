import { describe, expect, it, vi } from "vitest";
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
    const created = await bridgeApi.createWorkspaceSession("demo-1", true);
    const orchestrator = [...created.sessions].reverse().find(session => session.workspaceId === "demo-1" && session.kind === "orchestrator")!;
    expect(orchestrator.cwd).toMatch(/^\/tmp\/bridge\/worktrees\//);
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
    const first = await bridgeApi.runLearning("manual", "w");
    expect(first).toMatchObject({ status: "noop", duplicate: false });
    expect(first.report?.recommendationOnly).toBe(true);
    const duplicate = await bridgeApi.runLearning("in_app", "w");
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

describe("the Work board", () => {
  it("returns a board the screen can actually render without the desktop app", async () => {
    // vitest and a `bun run dev` preview both take this path, so a fallback that
    // returned an empty board would make every state below the empty one
    // undevelopable.
    const board = await bridgeApi.workBoard();
    expect(board.facts.length).toBeGreaterThan(0);
    // Empty until slice 5; populated now, because the task half of the board has to be
    // developable without the desktop app the same way the facts half is.
    expect(board.tasks.length).toBeGreaterThan(0);
    expect(board.latestRun).toBeNull();
    expect(board.suggestions.state).toBe("not_configured");
  });

  it("covers every fact kind and every freshness, so no state is only reachable in production", async () => {
    const board = await bridgeApi.workBoard();
    expect(new Set(board.facts.map(fact => fact.kind))).toEqual(
      new Set(["failed_completion_check", "actionable_approval", "blocked_worker_queue_item", "workspace_behind_base"]),
    );
    // Every freshness, not just stale: an unknown reading is a state the screen has a
    // whole row treatment for, and it was unreachable in the fallback until review
    // pointed out the claim above did not match the data.
    expect(new Set(board.facts.map(fact => fact.freshness))).toEqual(
      new Set(["live", "stale", "unknown"]),
    );
    // Every fact carries the action its kind implies, and no action is missing.
    for (const fact of board.facts) {
      expect(fact.action.kind).toBeTruthy();
      expect(fact.observedAt).toMatch(/^\d{4}-\d{2}-\d{2}T/);
    }
  });

  it("hands back a fresh copy, so a caller cannot mutate the next read", async () => {
    const first = await bridgeApi.workBoard();
    first.facts.length = 0;
    const second = await bridgeApi.workBoard();
    expect(second.facts.length).toBeGreaterThan(0);
  });

  it("stamps its timestamps at read time, not at import", async () => {
    // Frozen timestamps would age a preview's 'just now' into hours while freshness
    // stayed 'live', so the screen would contradict itself the longer it stayed open.
    const first = await bridgeApi.workBoard();
    await new Promise(resolve => setTimeout(resolve, 12));
    const second = await bridgeApi.workBoard();
    expect(Date.parse(second.generatedAt)).toBeGreaterThan(Date.parse(first.generatedAt));
    const live = (board: Awaited<ReturnType<typeof bridgeApi.workBoard>>) =>
      board.facts.find(fact => fact.freshness === "live")!.observedAt;
    expect(Date.parse(live(second))).toBeGreaterThan(Date.parse(live(first)));
  });

  it("orders facts the way the backend does, blocking before attention", async () => {
    const board = await bridgeApi.workBoard();
    const severities = board.facts.map(fact => fact.severity);
    const firstAttention = severities.indexOf("attention");
    expect(firstAttention).toBeGreaterThan(0);
    expect(severities.slice(0, firstAttention).every(value => value === "blocking")).toBe(true);
  });
});

describe("suggested-task actions", () => {
  const taskId = "task-v1:slack-work-1";

  it("performs no connector write or provider turn", async () => {
    // The claim step 4 deferred here, where it can actually be made: a spy over the whole
    // api surface. Marking a task done must not close the thread it came from.
    const outward = ["sendTurn", "startChat", "startSession", "createWorkspaceSession", "resolveApproval", "interruptTurn", "refreshWorkspaceBase", "workspaceBaseDivergence"] as const;
    const spies = outward.map(name => vi.spyOn(bridgeApi, name));
    try {
      await bridgeApi.workTaskAction(taskId, "done");
      await bridgeApi.workTaskPin(taskId, true);
      await bridgeApi.workTaskPrepareSession(taskId, "codex", null);
      for (const spy of spies) {
        expect(spy, `${spy.getMockName()} must not be called by a task action`).not.toHaveBeenCalled();
      }
    } finally {
      spies.forEach(spy => spy.mockRestore());
    }
  });

  it("prepares a draft and returns no turn id", async () => {
    // The absence is the contract: a field naming a dispatched turn would mean this call
    // had already spoken to a model on the user's behalf.
    const prepared = await bridgeApi.workTaskPrepareSession(taskId, "codex", null);
    expect(prepared.sessionId).toBeTruthy();
    expect(prepared.draft).toContain("slack.message");
    expect(Object.keys(prepared)).toEqual(expect.arrayContaining(["sessionId", "title", "draft"]));
    expect(Object.keys(prepared)).not.toContain("turnId");
  });

  it("refuses a task that is not on the board rather than inventing one", async () => {
    await expect(bridgeApi.workTaskAction("v1:nope", "done")).rejects.toThrow("Task not found");
    await expect(bridgeApi.workTaskPrepareSession("v1:nope", "codex", null)).rejects.toThrow("Task not found");
  });

  it("carries a task's untrusted text into the draft as text", async () => {
    const prepared = await bridgeApi.workTaskPrepareSession(taskId, "codex", null);
    expect(prepared.draft).toContain("Priya is blocked on the migration flag you own");
    for (const framing of ["You must", "Your task is", "```"]) {
      expect(prepared.draft).not.toContain(framing);
    }
  });

  it("recalls only the requested session in the mock forest", async () => {
    const isolated = await bridgeApi.searchSessionEntries("session-1", "isolated workers");
    expect(isolated.hits.some(hit => hit.snippet.toLowerCase().includes("isolated"))).toBe(true);
    const other = await bridgeApi.searchSessionEntries("session-other", "isolated workers");
    expect(other.hits).toEqual([]);
    await expect(bridgeApi.searchSessionEntries("  ", "hello")).rejects.toThrow("session id");
  });

  it("saves account pins only under account:local and lists by scope", async () => {
    const saved = await bridgeApi.saveMemoryRecord("I prefer Conventional Commits");
    expect(saved.scopeKey).toBe("account:local");
    expect(saved.provenance).toBe("user_explicit");
    const listed = await bridgeApi.listMemoryRecords("account:local");
    expect(listed.records.some(record => record.id === saved.id)).toBe(true);
    expect((await bridgeApi.listMemoryRecords("workspace:other")).records).toEqual([]);
    await expect(bridgeApi.listMemoryRecords("  ")).rejects.toThrow("scope");
    const forgotten = await bridgeApi.deleteMemoryRecord(saved.id);
    expect(forgotten.status).toBe("deleted");
    expect((await bridgeApi.listMemoryRecords("account:local")).records.some(record => record.id === saved.id)).toBe(false);
  });
});
