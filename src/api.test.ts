import { describe, expect, it } from "vitest";
import { bridgeApi } from "./api";

describe("SQLite-shaped mock observability", () => {
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
