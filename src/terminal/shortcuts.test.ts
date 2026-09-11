import { afterEach, describe, expect, it, vi } from "vitest";
import { terminalChord, terminalCommand } from "./shortcuts";

const event = (overrides: Partial<KeyboardEvent> = {}) => ({ key: "d", code: "KeyD", metaKey: false, ctrlKey: false, shiftKey: false, altKey: false, ...overrides });
afterEach(() => vi.restoreAllMocks());

describe("terminal shortcuts", () => {
  it.each(["MacIntel", "Linux x86_64"])("preserves CLI Control keys on %s", platform => {
    vi.spyOn(navigator, "platform", "get").mockReturnValue(platform);
    for (const key of ["c", "d", "f", "t", "w"]) expect(terminalCommand(event({ key, ctrlKey: true }))).toBeUndefined();
  });
  it("uses Command modifiers for splits, focus and shifted bracket tabs on macOS", () => {
    vi.spyOn(navigator, "platform", "get").mockReturnValue("MacIntel");
    expect(terminalCommand(event({ metaKey: true }))).toBe("split-right");
    expect(terminalCommand(event({ metaKey: true, shiftKey: true }))).toBe("split-down");
    expect(terminalCommand(event({ metaKey: true, altKey: true, key: "ArrowRight" }))).toBe("next-pane");
    expect(terminalCommand(event({ metaKey: true, shiftKey: true, key: "}", code: "BracketRight" }))).toBe("next-tab");
  });
  it("uses Control+Alt elsewhere without duplicating the modifier label", () => {
    vi.spyOn(navigator, "platform", "get").mockReturnValue("Linux x86_64");
    expect(terminalCommand(event({ ctrlKey: true, altKey: true }))).toBe("split-right");
    expect(terminalChord("next-pane")).toBe("Ctrl+Alt+→");
  });
});
