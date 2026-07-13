import { describe, expect, it } from "vitest";
import { queueExplanation, restorationPresentation, turnBudget } from "./observability";
import type { QueuedWorkerRequest, SessionForestSnapshot, WorkerLease } from "./types";

describe("restoration honesty", () => {
  it("keeps every restoration mode distinct", () => {
    expect(["hot", "native", "checkpoint_restored", "fresh"].map(mode => restorationPresentation(mode as never).label))
      .toEqual(["HOT", "NATIVE RESUME", "CHECKPOINT RESTORED", "FRESH"]);
  });
});

describe("policy explanations", () => {
  const queue = { id:"q", parentSessionId:"p", workspaceId:"w", turnId:"t", request:{ objective:"write auth", ownedPaths:["src/auth/**"], reason:"owned_path_conflict" }, actualModel:"runtime", queueStatus:"queued", sequence:1, dispatchedSessionId:null, createdAt:"now", updatedAt:"now" } satisfies QueuedWorkerRequest;
  const lease = { sessionId:"worker", workspaceId:"w", role:"implementation", capabilityTier:"standard", taskFamily:"implementation", ownedPaths:["src/auth/**"], writeMode:"isolated", leaseStatus:"active", expiresAt:null, createdAt:"now", updatedAt:"now" } satisfies WorkerLease;
  it("names the conflicting owner and path", () => expect(queueExplanation(queue, [lease])).toBe("Waiting: implementation owns src/auth/**."));
  it("names human approval and paused TTL as a first-class state", () => expect(queueExplanation({...queue, queueStatus:"blocked_on_human"}, [])).toBe("Blocked on human approval; queue TTL is paused."));
  it("shows current per-turn budget from persisted ledger rows", () => {
    const snapshot = { usage:[{ id:1,workspaceId:"w",sessionId:"a",turnId:"t",inputTokens:null,outputTokens:null,cacheReadTokens:null,cacheWriteTokens:null,contextPercent:null,capabilityUnits:8,runtimeMs:null,source:"policy.spawn.strong",createdAt:"now" }], policyLimits:{maxWorkersPerTurn:3,maxStrongWorkersPerTurn:1,maxCapabilityUnitsPerTurn:24} } as SessionForestSnapshot;
    expect(turnBudget(snapshot, "t")).toMatchObject({ units:8, workers:1, strongWorkers:1 });
  });
});
