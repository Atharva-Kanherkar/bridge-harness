import { describe, expect, it } from "vitest";
import { mergeForestSnapshot } from "./forest";
import type { SessionEntry, SessionForestSnapshot } from "./types";

function entry(id: string, sequence: number): SessionEntry {
  return {
    id,
    sessionId: "session-1",
    parentEntryId: null,
    sequence,
    semanticSchemaVersion: 1,
    kind: "assistant.message",
    payload: { text: `entry ${id}` },
    providerEventId: null,
    contextVisibility: "eligible",
    tokenEstimate: null,
    createdAt: "now",
  };
}

function snapshot(entries: SessionEntry[] = []): SessionForestSnapshot {
  return {
    sessionId: "session-1",
    entries,
    head: null,
    leaves: [],
    workerLeases: [],
    workerRuntimes: [],
    workerQueue: [],
    usage: [],
    reasons: [],
    policyLimits: {
      maxWorkersPerTurn: 4,
      maxStrongWorkersPerTurn: 1,
      maxCapabilityUnitsPerTurn: 8,
    },
    repositoryDivergence: {
      status: "unknown",
      selectedState: null,
      currentState: { status: "unavailable" },
    },
    completion: null,
  };
}

describe("mergeForestSnapshot", () => {
  it("preserves entry identity when only a worker heartbeat changes", () => {
    const current = snapshot([entry("entry-1", 1)]);
    const next: SessionForestSnapshot = {
      ...snapshot(current.entries.map(value => ({ ...value }))),
      workerRuntimes: [{
        sessionId: "worker",
        parentSessionId: current.sessionId,
        lifecycleState: "working",
        taskFamily: "planning",
        compatibilityKey: "planning",
        resultStatus: "pending",
        retryCount: 0,
        warmUntil: null,
        worktreePath: null,
        worktreeBranch: null,
        lastResult: null,
        lastActivityAt: "2026-07-29T12:00:02Z",
        updatedAt: "2026-07-29T12:00:00Z",
      }],
    };
    const merged = mergeForestSnapshot(current, next);
    expect(merged.entries).toBe(current.entries);
    expect(merged.workerRuntimes).toBe(next.workerRuntimes);
  });

  it("adopts the new entry array when history grows", () => {
    const current = snapshot([entry("entry-1", 1)]);
    const next = snapshot([entry("entry-1", 1), entry("entry-2", 2)]);
    expect(mergeForestSnapshot(current, next).entries).toBe(next.entries);
  });

  it("adopts the new entry array when the tail differs at equal length", () => {
    const current = snapshot([entry("entry-1", 1)]);
    const next = snapshot([entry("entry-other", 1)]);
    expect(mergeForestSnapshot(current, next).entries).toBe(next.entries);
  });

  it("treats matching empty histories as unchanged", () => {
    const current = snapshot();
    const next = snapshot();
    expect(mergeForestSnapshot(current, next).entries).toBe(current.entries);
  });

  it("returns the fresh snapshot when there is no current one", () => {
    const next = snapshot([entry("entry-1", 1)]);
    expect(mergeForestSnapshot(undefined, next)).toBe(next);
  });
});
