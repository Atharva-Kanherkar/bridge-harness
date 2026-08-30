import { describe, expect, it } from "vitest";
import type { AgentDefinition } from "./types";
import { agentMentionQuery, agentShortcutCandidates, normalizeAgentToken, parseAgentMention } from "./agentMention";

const agent = (overrides: Partial<AgentDefinition>): AgentDefinition => ({
  id: "bridge-verification",
  name: "Verification agent",
  description: "Tests outcomes independently.",
  role: "verification",
  harness: "bridge",
  model: null,
  effort: "high",
  systemPrompt: "",
  enabled: true,
  isDefault: false,
  isBuiltIn: true,
  createdAt: "",
  updatedAt: "",
  ...overrides,
});

describe("parseAgentMention", () => {
  it("parses a leading directive and trims only its objective", () => {
    expect(parseAgentMention("#Verifier   verify the current implementation  ")).toEqual({
      token: "verifier",
      objective: "verify the current implementation",
    });
  });

  it("retains an empty objective for host-side rejection", () => {
    expect(parseAgentMention("#implementer")).toEqual({ token: "implementer", objective: "" });
    expect(parseAgentMention("#implementer   ")).toEqual({ token: "implementer", objective: "" });
  });

  it("does not reserve inline hashes, markdown headings, or unrelated shortcut syntaxes", () => {
    expect(parseAgentMention("please ask #verifier to check")).toBeNull();
    expect(parseAgentMention("# verifier heading")).toBeNull();
    expect(parseAgentMention("$codex hello")).toBeNull();
    expect(parseAgentMention("/clear")).toBeNull();
    expect(parseAgentMention("@src/App.tsx review this")).toBeNull();
  });
});

describe("agentMentionQuery", () => {
  it("recognizes only a leading token before objective whitespace", () => {
    expect(agentMentionQuery("#")).toBe("");
    expect(agentMentionQuery("#ver")).toBe("ver");
    expect(agentMentionQuery("#verifier verify")).toBeUndefined();
    expect(agentMentionQuery("ask #ver")).toBeUndefined();
  });
});

describe("agentShortcutCandidates", () => {
  it("matches names, ids, roles, and aliases and inserts the canonical role alias", () => {
    const candidates = agentShortcutCandidates([
      agent({}),
      agent({ id: "bridge-implementation", name: "Implementation agent", role: "implementation", effort: "medium" }),
    ], "ver");
    expect(candidates.map(candidate => [candidate.agent.id, candidate.token])).toEqual([
      ["bridge-verification", "verifier"],
    ]);
    expect(agentShortcutCandidates([agent({})], "bridge-verification")).toHaveLength(1);
    expect(agentShortcutCandidates([agent({})], "reviewer")).toHaveLength(1);
  });

  it("excludes disabled agents and orchestrators", () => {
    expect(agentShortcutCandidates([
      agent({ enabled: false }),
      agent({ id: "bridge-orchestrator", name: "Bridge orchestrator", role: "orchestrator" }),
    ], "")).toEqual([]);
  });

  it("uses configured names when a role alias would be ambiguous and sorts deterministically", () => {
    const candidates = agentShortcutCandidates([
      agent({ id: "z", name: "Security Reviewer" }),
      agent({ id: "a", name: "Accessibility Reviewer" }),
    ], "review");
    expect(candidates.map(candidate => candidate.token)).toEqual(["accessibility-reviewer", "security-reviewer"]);
  });

  it("normalizes custom labels case-insensitively", () => {
    expect(normalizeAgentToken("  #Release.QA Agent  ")).toBe("release-qa-agent");
    expect(agentShortcutCandidates([agent({ id: "custom-qa", name: "Release QA" })], "RELEASE.QA")).toHaveLength(1);
  });
});
