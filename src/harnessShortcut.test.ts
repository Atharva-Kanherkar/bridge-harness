import { describe, expect, it } from "vitest";
import { harnessShortcutQuery, parseHarnessShortcut } from "./harnessShortcut";

describe("parseHarnessShortcut", () => {
  it("splits a completed shortcut into harness id and message", () => {
    expect(parseHarnessShortcut("$codex do you think we are right?"))
      .toEqual({ harnessId: "codex", rest: "do you think we are right?" });
  });

  it("trims the message and tolerates extra whitespace", () => {
    expect(parseHarnessShortcut("$claude   hello there  \n"))
      .toEqual({ harnessId: "claude", rest: "hello there" });
  });

  it("accepts harness ids with dots, dashes, and underscores", () => {
    expect(parseHarnessShortcut("$acp-gemini go")).toEqual({ harnessId: "acp-gemini", rest: "go" });
  });

  it("is null with no message yet — still just a token being typed", () => {
    expect(parseHarnessShortcut("$codex")).toBeNull();
    expect(parseHarnessShortcut("$codex ")).toBeNull();
    expect(parseHarnessShortcut("$codex   ")).toBeNull();
  });

  it("is null without a leading $", () => {
    expect(parseHarnessShortcut("codex do the thing")).toBeNull();
    expect(parseHarnessShortcut("hello $codex")).toBeNull();
  });

  it("does not fire on a mid-sentence $ (e.g. talking about money)", () => {
    expect(parseHarnessShortcut("it costs $5 more than expected")).toBeNull();
  });
});

describe("harnessShortcutQuery", () => {
  it("reads the in-progress token with nothing typed after it", () => {
    expect(harnessShortcutQuery("$")).toBe("");
    expect(harnessShortcutQuery("$cod")).toBe("cod");
  });

  it("is undefined once a message follows, or with no $ at all", () => {
    expect(harnessShortcutQuery("$codex hi")).toBeUndefined();
    expect(harnessShortcutQuery("hello")).toBeUndefined();
  });
});
