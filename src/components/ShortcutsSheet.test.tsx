// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ShortcutsSheet } from "./ShortcutsSheet";
import { formatChord, SHORTCUT_GROUPS, SHORTCUTS } from "../keymap";

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

// The dialog portals to the body, so assertions read the document rather than
// the mount point.
const rendered = () => document.body.textContent ?? "";

describe("the shortcuts sheet", () => {
  it("lists every command in the keymap", async () => {
    await act(async () => root.render(<ShortcutsSheet open onClose={() => {}} />));
    for (const shortcut of SHORTCUTS) {
      expect(rendered(), `${shortcut.id} is listed`).toContain(shortcut.label);
    }
  });

  it("groups them the way the keymap groups them", async () => {
    await act(async () => root.render(<ShortcutsSheet open onClose={() => {}} />));
    for (const group of SHORTCUT_GROUPS) {
      expect(document.querySelector(`section[aria-label="${group}"]`), `${group} section`).not.toBeNull();
    }
    const chatSection = document.querySelector('section[aria-label="Chat"]')!;
    expect(chatSection.textContent).toContain("New chat");
    expect(chatSection.textContent).not.toContain("Toggle the sidebar");
  });

  it("prints chords, not raw key names", async () => {
    await act(async () => root.render(<ShortcutsSheet open onClose={() => {}} />));
    const chords = [...document.querySelectorAll("kbd")].map(key => key.textContent);
    expect(chords).toContain(formatChord(SHORTCUTS.find(shortcut => shortcut.id === "new-chat")!));
    expect(chords).toContain("⌥⌘↩");
    expect(chords).not.toContain("Enter");
  });

  it("hands closing back to its host", async () => {
    const onClose = vi.fn();
    await act(async () => root.render(<ShortcutsSheet open onClose={onClose} />));
    const close = document.querySelector<HTMLButtonElement>('button[aria-label="Close"]')!;
    await act(async () => close.click());
    expect(onClose).toHaveBeenCalled();
  });

  it("renders nothing while closed", async () => {
    await act(async () => root.render(<ShortcutsSheet open={false} onClose={() => {}} />));
    expect(rendered()).not.toContain("Keyboard shortcuts");
  });
});
