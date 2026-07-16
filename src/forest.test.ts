import { describe, expect, it } from "vitest";
import { forestSnapshotKey } from "./forest";
import type { SessionForestSnapshot } from "./types";

function snapshot(): SessionForestSnapshot {
  return {
    sessionId: "session-1",
    entries: [],
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

describe("forestSnapshotKey", () => {
  it("is stable for equivalent snapshots and changes with durable state", () => {
    const first = snapshot();
    const equivalent = structuredClone(first);
    expect(forestSnapshotKey(equivalent)).toBe(forestSnapshotKey(first));

    equivalent.entries.push({
      id: "entry-1",
      sessionId: "session-1",
      parentEntryId: null,
      sequence: 1,
      semanticSchemaVersion: 1,
      kind: "assistant.message",
      payload: { text: "done" },
      providerEventId: null,
      contextVisibility: "eligible",
      tokenEstimate: null,
      createdAt: "now",
    });
    expect(forestSnapshotKey(equivalent)).not.toBe(forestSnapshotKey(first));
  });
});
