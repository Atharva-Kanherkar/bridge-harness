import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { allOn, baseChoice, leavesOut, removed, switched } from "./chatToolChoice";
import type { ChatToolItem } from "./types";

const skill = (id: string, state: ChatToolItem["state"] = "on"): ChatToolItem =>
  ({ kind: "skill", id, label: id, group: "Skills", state, fixedReason: null, seen: true, tokens: null });
const skills = [skill("review"), skill("simplify"), skill("pdf", "off")];

describe("the arithmetic of a tool choice", () => {
  it("keeps the Claude skills in an allowlist and the Codex skills in an off list", () => {
    const claude = switched(baseChoice(null, "claude", null), skills[1]!, false, skills);
    expect(claude).toMatchObject({ claudeSkills: { only: ["review"] }, codexSkillsOff: [] });

    const codex = switched(baseChoice(null, "codex", null), skills[1]!, false, skills);
    expect(codex).toMatchObject({ claudeSkills: null, codexSkillsOff: ["simplify"] });
  });

  it("starts over for a choice of another harness or account", () => {
    const choice = { harness: "claude", account: "home", serversOff: ["github"] };
    expect(baseChoice(choice, "claude", "home").serversOff).toEqual(["github"]);
    expect(baseChoice(choice, "claude", "work")).toMatchObject({ account: "work", serversOff: [] });
    expect(baseChoice(choice, "codex", "home")).toMatchObject({ harness: "codex", serversOff: [] });
    expect(baseChoice({ harness: "claude", account: "claude" }, "claude", null).account).toBeNull();
  });

  it("leaves a choice without a skill field as it is", () => {
    const base = baseChoice(null, "opencode", null);
    expect(switched(base, skills[0]!, false, skills)).toEqual(base);
  });

  it("removes a saved name from the field that holds it", () => {
    const claude = baseChoice({ harness: "claude", claudeSkills: { only: ["review", "gone"] } }, "claude", null);
    expect(removed(claude, skill("gone"))).toMatchObject({ claudeSkills: { only: ["review"] }, codexSkillsOff: [] });

    const codex = baseChoice({ harness: "codex", codexSkillsOff: ["gone", "pdf"] }, "codex", null);
    expect(removed(codex, skill("gone"))).toMatchObject({ claudeSkills: null, codexSkillsOff: ["pdf"] });
  });

  it("turns each item on with the set of no choice", () => {
    const claude = baseChoice({ harness: "claude", account: "home", serversOff: ["github"], claudeSkills: { only: ["review"] } }, "claude", "home");
    expect(leavesOut(claude)).toBe(true);
    expect(allOn(claude)).toEqual({ harness: "claude", account: "home", serversOff: [], codexAppsOff: false, claudeSkills: null, codexSkillsOff: [] });
    expect(leavesOut(allOn(claude))).toBe(false);
    // A list that names each skill still leaves out a skill that comes later.
    expect(leavesOut(baseChoice({ harness: "claude", claudeSkills: { only: [] } }, "claude", null))).toBe(true);
    expect(leavesOut(baseChoice({ harness: "codex", codexAppsOff: true }, "codex", null))).toBe(true);
    expect(leavesOut(baseChoice({ harness: "codex", codexSkillsOff: ["pdf"] }, "codex", null))).toBe(true);
    expect(leavesOut(baseChoice(null, "opencode", null))).toBe(false);
  });

  it("does not compare a harness with a name on either side", () => {
    const branches = (source: string) => /harness\s*[!=]==\s*["']|["']\s*[!=]==\s*[\w.?]*harness\b/.test(source);
    // The check finds a comparison in both operand orders.
    expect(branches('if (base.harness === "claude") return base;')).toBe(true);
    expect(branches('if ("claude" === base.harness) return base;')).toBe(true);
    expect(branches("if (choice.harness !== 'codex') return base;")).toBe(true);
    expect(branches("if ('codex' !== choice?.harness) return base;")).toBe(true);
    expect(branches("return choice.harness === harness;")).toBe(false);
    const source = readFileSync(fileURLToPath(new URL("./chatToolChoice.ts", import.meta.url)), "utf8");
    expect(branches(source)).toBe(false);
  });
});
