import { describe, expect, it } from "vitest";
import { continuationNotice, importedChatBadge, importSourceLabel } from "./importSources";

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

describe("continuationNotice", () => {
  it("says the next message resumes the session when it can", () => {
    expect(continuationNotice({ mode: "native" }, "claude_code")).toEqual({
      headline: "Continues the Claude Code session",
      detail: "Your message opens a new chat that resumes it. Unfinished tool calls, background processes and hidden reasoning stay behind.",
    });
  });
  it("says it starts a new chat with the history, and why", () => {
    expect(continuationNotice({ mode: "history", reason: "Claude Code no longer has this session." }, "claude_code")).toEqual({
      headline: "Starts a new chat with this history",
      detail: "Claude Code no longer has this session. Unfinished tool calls, background processes and hidden reasoning stay behind.",
    });
    expect(continuationNotice({ mode: "history", reason: null }).detail).toMatch(/^Claude Code cannot resume it\./);
  });
});
