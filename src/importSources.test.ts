import { describe, expect, it } from "vitest";
import { importedChatBadge, importSourceLabel } from "./importSources";

describe("import source labels", () => {
  it("names every bundle tool id", () => {
    expect(importSourceLabel("claude_code")).toBe("Claude Code");
    expect(importSourceLabel("t3code")).toBe("T3 Code");
    expect(importSourceLabel("conductor")).toBe("Conductor");
  });
  it("treats a missing id as the pre-bundle Claude import and shows an unknown id as itself", () => {
    expect(importSourceLabel(undefined)).toBe("Claude Code");
    expect(importSourceLabel("newtool")).toBe("newtool");
  });
  it("builds the toolbar badge with a shortened fingerprint", () => {
    expect(importedChatBadge("codex", "0123456789abcdef")).toBe("Imported · Codex · 0123456789ab…");
    expect(importedChatBadge("codex", undefined)).toBe("Imported · Codex");
  });
});
