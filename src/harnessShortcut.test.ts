import { describe, expect, it } from "vitest";
import { closestHarnessShortcut, harnessShortcutQuery, parseHarnessShortcut } from "./harnessShortcut";

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

  it("does not treat a leading currency amount as routing intent", () => {
    expect(parseHarnessShortcut("$5 is cheaper than expected")).toBeNull();
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

describe("closestHarnessShortcut", () => {
  it("suggests one clearly close harness id", () => {
    expect(closestHarnessShortcut("claud", ["codex", "claude", "opencode"])).toBe("claude");
  });

  it("does not guess when no harness id is close", () => {
    expect(closestHarnessShortcut("hanress", ["codex", "claude", "opencode"])).toBeUndefined();
  });
});
