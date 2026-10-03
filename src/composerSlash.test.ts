import { describe, expect, it } from "vitest";
import { composerSlashToken, composerSlashMatches, insertComposerSlash } from "./composerSlash";
import type { SlashCommand } from "./types";

describe("explicit slash discovery", () => {
  it("finds leading and embedded tokens at the caret", () => {
    expect(composerSlashToken("/rev")).toEqual({ start: 0, end: 4, query: "rev", leading: true });
    expect(composerSlashToken("Please use /rev")).toEqual({ start: 11, end: 15, query: "rev", leading: false });
    expect(composerSlashToken("Please use /rev on this", 15)?.query).toBe("rev");
    expect(composerSlashToken("Please use /review on this", 15)?.end).toBe(18);
  });
  it("ignores prose, selections, URLs, paths and code", () => {
    for (const text of ["The skill suggestion does not work", "https://site/rev", "open /src/index.ts", "`use /rev", "use `/rev`", "use /rev."]) {
      expect(composerSlashToken(text), text).toBeUndefined();
    }
    expect(composerSlashToken("/rev", 1, 4)).toBeUndefined();
  });
  it("preserves the sentence and suffix when selecting a skill", () => {
    const text = "Please use /rev on this";
    expect(insertComposerSlash(text, composerSlashToken(text, 15)!, "review")).toEqual({ text: "Please use /review on this", caret: 19 });
    const middle = "Please use /review on this";
    expect(insertComposerSlash(middle, composerSlashToken(middle, 15)!, "review-checkpoint").text).toBe("Please use /review-checkpoint on this");
    expect(insertComposerSlash("use /", composerSlashToken("use /")!, "review").text).toBe("use /review ");
  });
  it("offers inline skills and prompts but keeps builtins leading-only", () => {
    const commands: SlashCommand[] = [
      { name: "clear", description: "Clear chat", kind: "builtin", harness: "bridge" },
      { name: "review", description: "Review code", kind: "skill", harness: "codex" },
      { name: "deploy", description: "Deploy app", kind: "prompt", harness: "codex" },
    ];
    expect(composerSlashMatches(commands, composerSlashToken("use /")).map(c => c.name)).toEqual(["deploy", "review"]);
    expect(composerSlashMatches(commands, composerSlashToken("/")).map(c => c.name)).toEqual(["clear", "deploy", "review"]);
  });
});
