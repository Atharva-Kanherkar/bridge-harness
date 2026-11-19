import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { baseChoice, removed, switched } from "./chatToolChoice";
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

  it("does not branch on a harness name", () => {
    const source = readFileSync(fileURLToPath(new URL("./chatToolChoice.ts", import.meta.url)), "utf8");
    expect(source).not.toMatch(/harness\s*[!=]==\s*["']/);
  });
});
