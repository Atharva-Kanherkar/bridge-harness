import { describe, expect, it } from "vitest";
import { agentName, agentRole, agentTopic } from "./agentIdentity";

// Contract: testing/fix-agent-starting-state.md, unit tests.

describe("agentIdentity", () => {
  it("gives the same id the same name every time", () => {
    expect(agentName("4c1f0b8e-0d1e-4a3b-9b55-3f1f2c9a7d10")).toBe(agentName("4c1f0b8e-0d1e-4a3b-9b55-3f1f2c9a7d10"));
  });

  it("spreads different ids across the pool", () => {
    const names = new Set(Array.from({ length: 40 }, (_, index) => agentName(`worker-${index}`)));
    expect(names.size).toBeGreaterThan(15);
  });

  it("reads the role from the label", () => {
    expect(agentRole("Research · standard")).toBe("Research");
    expect(agentRole("Worker")).toBe("Worker");
  });

  it("only offers a topic that says more than the label", () => {
    expect(agentTopic({ title: "Rank the gateway repos", label: "Research · standard" })).toBe("Rank the gateway repos");
    expect(agentTopic({ title: "Research · standard", label: "Research · standard" })).toBeUndefined();
    expect(agentTopic({ title: null, label: "Research · standard" })).toBeUndefined();
    expect(agentTopic({ title: "  ", label: "Research · standard" })).toBeUndefined();
  });
});
