// @vitest-environment jsdom
// isTypingTarget reads real elements; everything else here is pure.
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { chordLabel, formatChord, isTypingTarget, matchShortcut, SHORTCUTS, shortcutFor, shortcutsInGroup, strokeDigit, type KeyStroke, type Shortcut } from "./keymap";

function stroke(overrides: Partial<KeyStroke> = {}): KeyStroke {
  return { key: "", code: "", metaKey: false, ctrlKey: false, altKey: false, shiftKey: false, ...overrides };
}

/** The accelerator a chord spells, derived rather than copied, so the strings
 *  the shell mirrors cannot quietly stop describing the chord they sit on. */
function acceleratorFor(shortcut: Shortcut): string {
  const { chord } = shortcut;
  const named: Record<string, string> = { arrowup: "Up", arrowdown: "Down", enter: "Enter" };
  const raw = chord.key ?? "";
  const key = named[raw.toLowerCase()] ?? (raw.length === 1 ? raw.toUpperCase() : raw);
  return [chord.alt ? "Alt" : "", chord.shift ? "Shift" : "", chord.meta ? "CmdOrCtrl" : "", key]
    .filter(Boolean)
    .join("+");
}

describe("the keymap", () => {
  it("matches a chord against its own stroke and nothing adjacent", () => {
    expect(matchShortcut(stroke({ key: "n", metaKey: true }))?.shortcut.id).toBe("new-chat");
    expect(matchShortcut(stroke({ key: "n" }))).toBeUndefined();
    expect(matchShortcut(stroke({ key: "n", altKey: true, metaKey: true }))).toBeUndefined();
  });

  it("treats shift as part of the chord rather than noise", () => {
    // macOS hands over the shifted character, so the table has to match the
    // modifier and not the letter it printed.
    expect(matchShortcut(stroke({ key: "N", metaKey: true, shiftKey: true }))?.shortcut.id).toBe("new-project");
    expect(matchShortcut(stroke({ key: "n", metaKey: true }))?.shortcut.id).toBe("new-chat");
  });

  it("matches option-rewritten keys physically", () => {
    // ⌥⌘F arrives as "ƒ" on a US layout; only the code still says F.
    expect(matchShortcut(stroke({ key: "ƒ", code: "KeyF", metaKey: true, altKey: true }))?.shortcut.id)
      .toBe("toggle-fullscreen");
  });

  it("resolves digit families to the index they name", () => {
    expect(matchShortcut(stroke({ key: "3", code: "Digit3", metaKey: true })))
      .toMatchObject({ shortcut: { id: "jump-to-chat" }, index: 2 });
    expect(matchShortcut(stroke({ key: "¡", code: "Digit1", metaKey: true, altKey: true })))
      .toMatchObject({ shortcut: { id: "open-dock-pane" }, index: 0 });
  });

  it("keeps the dock toggle out of the pane indexes", () => {
    const match = matchShortcut(stroke({ key: "º", code: "Digit0", metaKey: true, altKey: true }));
    expect(match?.shortcut.id).toBe("toggle-dock");
    expect(match?.index).toBeUndefined();
    // A bare ⌘0 is a command of its own (actual size), but it must never
    // resolve to a chat.
    const actual = matchShortcut(stroke({ key: "0", code: "Digit0", metaKey: true }));
    expect(actual?.shortcut.id).toBe("zoom-reset");
    expect(actual?.index).toBeUndefined();
  });

  it("answers both spellings macOS sends for plus and minus", () => {
    // On a US layout the plus key is `=` alone and `+` shifted, and both are one
    // zoom in. The polyfill ignored shift to get this; a chord that pinned shift
    // would lose the shifted half of the gesture.
    expect(matchShortcut(stroke({ key: "=", metaKey: true }))?.shortcut.id).toBe("zoom-in");
    expect(matchShortcut(stroke({ key: "+", metaKey: true, shiftKey: true }))?.shortcut.id).toBe("zoom-in");
    expect(matchShortcut(stroke({ key: "-", metaKey: true }))?.shortcut.id).toBe("zoom-out");
    expect(matchShortcut(stroke({ key: "_", metaKey: true, shiftKey: true }))?.shortcut.id).toBe("zoom-out");
    // The numeric keypad spells the same characters and is still one command.
    expect(matchShortcut(stroke({ key: "+", code: "NumpadAdd", metaKey: true }))?.shortcut.id).toBe("zoom-in");
    expect(matchShortcut(stroke({ key: "0", code: "Numpad0", metaKey: true }))?.shortcut.id).toBe("zoom-reset");
    // Shift being optional must not make these chords match anything unheld.
    expect(matchShortcut(stroke({ key: "=" }))?.shortcut.id).not.toBe("zoom-in");
    expect(matchShortcut(stroke({ key: "=", altKey: true, metaKey: true }))?.shortcut.id).not.toBe("zoom-in");
  });

  it("lets Control stand in for Command", () => {
    expect(matchShortcut(stroke({ key: "k", ctrlKey: true }))?.shortcut.id).toBe("open-recall");
  });

  it("suppresses every command that is not meant to reach a text field", () => {
    expect(matchShortcut(stroke({ key: "n", metaKey: true }), true)).toBeUndefined();
    expect(matchShortcut(stroke({ key: ".", metaKey: true }), true)?.shortcut.id).toBe("interrupt-turn");
  });

  it("knows a text surface from a control", () => {
    const editable = document.createElement("div");
    editable.contentEditable = "true";
    // jsdom does not implement isContentEditable off the attribute.
    Object.defineProperty(editable, "isContentEditable", { value: true });
    expect(isTypingTarget(document.createElement("input"))).toBe(true);
    expect(isTypingTarget(document.createElement("textarea"))).toBe(true);
    expect(isTypingTarget(editable)).toBe(true);
    expect(isTypingTarget(document.createElement("button"))).toBe(false);
    expect(isTypingTarget(null)).toBe(false);
  });

  it("gives every command a chord of its own", () => {
    const spellings = SHORTCUTS.map(shortcut => JSON.stringify({ ...shortcut.chord, digits: !!shortcut.digits }));
    expect(new Set(spellings).size).toBe(SHORTCUTS.length);
  });

  it("prints modifiers in the order macOS does", () => {
    expect(chordLabel("new-chat")).toBe("⌘N");
    expect(chordLabel("new-project")).toBe("⇧⌘N");
    expect(chordLabel("toggle-fullscreen")).toBe("⌥⌘F");
    expect(formatChord(shortcutFor("next-chat"))).toBe("⌥⌘↓");
    expect(chordLabel("expand-dock")).toBe("⌥⌘↩");
  });

  it("spells every menu accelerator the way its chord matches", () => {
    const withMenus = SHORTCUTS.filter(shortcut => shortcut.accelerator);
    expect(withMenus.length).toBeGreaterThan(0);
    for (const shortcut of withMenus) {
      expect(shortcut.accelerator, `${shortcut.id} accelerator`).toBe(acceleratorFor(shortcut));
    }
    // A digit family cannot be one menu item, so it must not claim one.
    expect(SHORTCUTS.filter(shortcut => shortcut.digits).every(shortcut => !shortcut.accelerator)).toBe(true);
  });

  it("sorts every command into a group the sheet renders", () => {
    const grouped = ["Chat", "Navigation", "View"] as const;
    expect(grouped.flatMap(group => shortcutsInGroup(group))).toHaveLength(SHORTCUTS.length);
  });

  it("reads a digit off either the code or the printed key", () => {
    expect(strokeDigit(stroke({ code: "Digit7", key: "&" }))).toBe(7);
    expect(strokeDigit(stroke({ code: "", key: "7" }))).toBe(7);
    expect(strokeDigit(stroke({ code: "KeyA", key: "a" }))).toBeUndefined();
  });
});

// The shell's menu spells the same chords in Rust, where it cannot import the
// table. Reading the file is what stops the two spellings from drifting apart
// silently — the menu is the only place a binding is written twice.
describe("the shell's menu", () => {
  const source = readFileSync(join(process.cwd(), "src-tauri/src/menu.rs"), "utf8");
  const declared = [...source.matchAll(/Command\("([^"]+)",\s*"([^"]+)",\s*"([^"]+)"\)/g)]
    .map(([, id, label, accelerator]) => ({ id, label, accelerator }));

  it("names commands the keymap declares", () => {
    expect(declared.length).toBeGreaterThan(0);
    for (const item of declared) {
      const shortcut = SHORTCUTS.find(candidate => candidate.id === item.id);
      expect(shortcut, `${item.id} is a keymap command`).toBeDefined();
      expect(shortcut!.accelerator, `${item.id} accelerator`).toBe(item.accelerator);
    }
  });

  it("carries every command the keymap sends to a submenu", () => {
    const menued = SHORTCUTS.filter(shortcut => shortcut.menu).map(shortcut => shortcut.id).sort();
    expect(declared.map(item => item.id).sort()).toEqual(menued);
  });
});
