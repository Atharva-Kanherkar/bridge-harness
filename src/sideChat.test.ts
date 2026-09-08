import { describe, expect, it } from "vitest";
import { parseSideChatCommand, quoteSelection } from "./sideChat";

describe("parseSideChatCommand", () => {
  it("parses /btw and /side with their question", () => {
    expect(parseSideChatCommand("/btw is the plan sound?")).toEqual({ command: "btw", query: "is the plan sound?" });
    expect(parseSideChatCommand("/side what did we pick?")).toEqual({ command: "side", query: "what did we pick?" });
  });

  it("accepts any casing and trims the question", () => {
    expect(parseSideChatCommand("/BTW   hi ")).toEqual({ command: "btw", query: "hi" });
  });

  it("keeps a bare command as an empty question", () => {
    expect(parseSideChatCommand("/btw")).toEqual({ command: "btw", query: "" });
    expect(parseSideChatCommand("/btw ")).toEqual({ command: "btw", query: "" });
  });

  it("leaves every other composer text alone", () => {
    expect(parseSideChatCommand("/btwext hi")).toBeNull();
    expect(parseSideChatCommand("/usage")).toBeNull();
    expect(parseSideChatCommand("$codex hi")).toBeNull();
    expect(parseSideChatCommand("what about /btw later?")).toBeNull();
    expect(parseSideChatCommand("")).toBeNull();
  });
});

describe("quoteSelection", () => {
  it("blockquotes each selected line", () => {
    expect(quoteSelection("first line\nsecond line")).toBe("> first line\n> second line");
  });

  it("keeps a single-line selection one blockquote line", () => {
    expect(quoteSelection("only")).toBe("> only");
  });

  it("drops trailing whitespace instead of quoting blank tails", () => {
    expect(quoteSelection("keep me  \n")).toBe("> keep me");
  });
});
