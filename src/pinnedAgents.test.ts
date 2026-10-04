import { describe, expect, it } from "vitest";
import { PINNED_AGENTS_KEY, readPinnedAgents, writePinnedAgents } from "./pinnedAgents";

const storage = (initial: Record<string, string> = {}) => {
  const values = new Map(Object.entries(initial));
  return { getItem: (key: string) => values.get(key) ?? null, setItem: (key: string, value: string) => { values.set(key, value); } };
};

describe("pinned agents", () => {
  it("round-trips through storage", () => {
    const store = storage();
    writePinnedAgents(new Set(["w1", "w2"]), store);
    expect([...readPinnedAgents(store)]).toEqual(["w1", "w2"]);
  });

  it("reads nothing from a missing, malformed or foreign value", () => {
    expect(readPinnedAgents(storage()).size).toBe(0);
    expect(readPinnedAgents(storage({ [PINNED_AGENTS_KEY]: "{not json" })).size).toBe(0);
    expect([...readPinnedAgents(storage({ [PINNED_AGENTS_KEY]: JSON.stringify(["w1", 7, "", null]) }))]).toEqual(["w1"]);
  });
});
