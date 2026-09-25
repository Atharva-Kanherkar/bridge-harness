// Every keyboard command the app answers to, in one table.
//
// A shortcut has to agree with itself in four places: the handler that runs
// it, the tooltip that advertises it, the native menu item that duplicates
// it, and the sheet that lists it. Four hand-maintained copies drift; one
// table cannot. Nothing here knows about React or Tauri — App dispatches from
// it, the shell mirrors its accelerators, and the sheet renders it.

export type CommandId =
  | "new-chat"
  | "new-project"
  | "interrupt-turn"
  | "open-recall"
  | "jump-to-chat"
  | "next-chat"
  | "previous-chat"
  | "open-projects"
  | "open-settings"
  | "toggle-sidebar"
  | "toggle-fullscreen"
  | "toggle-dock"
  | "open-dock-pane"
  | "expand-dock"
  | "zoom-in"
  | "zoom-out"
  | "zoom-reset"
  | "show-shortcuts";

/** Where a command sits in the native menu. Commands with no home there are
 *  reachable by chord and from the sheet only. */
export type MenuSubmenu = "Bridge" | "File" | "View" | "Go" | "Help";

export type ShortcutGroup = "Chat" | "Navigation" | "View";

export type Chord = {
  /** Compared case-insensitively with `event.key`. */
  key?: string;
  /** Every printed key this chord answers to, for a chord macOS spells two
   *  ways. Cmd+ is `=` unshifted and `+` shifted, and `modifiersMatch` holds
   *  shift to the chord, so one `key` cannot cover both. Takes precedence over
   *  `key`, which then only names the spelling the menu accelerator uses. */
  keys?: string[];
  /** Compared with `event.code`. Option rewrites the printed key on macOS, so
   *  anything behind ⌥ has to be addressed physically. Either match counts. */
  code?: string;
  /** ⌘, or Control where there is no Command key. */
  meta?: boolean;
  alt?: boolean;
  shift?: boolean;
  /** Match whether or not shift is held. For a chord macOS spells both ways on
   *  the same physical keys: the plus key is `=` alone and `+` with shift, and
   *  both are the same zoom in. A chord with `keys` and this set covers the
   *  whole gesture without a second command for the shifted spelling. */
  shiftOptional?: boolean;
};

export type Shortcut = {
  id: CommandId;
  label: string;
  group: ShortcutGroup;
  chord: Chord;
  /** 1 through 9 on the same chord, resolving to an index. */
  digits?: boolean;
  /** Fires with a text field focused. Reserved for the commands whose whole
   *  point is to be reachable mid-sentence. */
  whileTyping?: boolean;
  /** Presentation of the window rather than a command about its contents, so
   *  it stays live where the others are suppressed: inside the terminal pane,
   *  where the polyfill answered it too and where reading wide output is a
   *  fair reason to want it larger. */
  windowLevel?: boolean;
  /** How the sheet prints the chord, where a digit family or a symbol reads
   *  better than the literal key. */
  display?: string;
  menu?: MenuSubmenu;
  /** Mirrored in `src-tauri/src/menu.rs`. `keymap.test.ts` holds the two
   *  spellings to the same chord. */
  accelerator?: string;
};

export type ShortcutMatch = { shortcut: Shortcut; index?: number };

/** Only what matching reads, so a test can hand over a literal. */
export type KeyStroke = Pick<KeyboardEvent, "key" | "code" | "metaKey" | "ctrlKey" | "altKey" | "shiftKey">;

export const SHORTCUTS: Shortcut[] = [
  { id: "new-chat", label: "New chat", group: "Chat", chord: { key: "n", meta: true }, menu: "File", accelerator: "CmdOrCtrl+N" },
  { id: "new-project", label: "New project", group: "Chat", chord: { key: "n", meta: true, shift: true }, menu: "File", accelerator: "Shift+CmdOrCtrl+N" },
  // Held while typing on purpose: wanting a turn to stop is something you
  // realise with your hands already in the composer.
  { id: "interrupt-turn", label: "Interrupt the running turn", group: "Chat", chord: { key: ".", meta: true }, whileTyping: true, display: "⌘.", menu: "File", accelerator: "CmdOrCtrl+." },
  { id: "open-recall", label: "Search this chat's history", group: "Navigation", chord: { key: "k", meta: true }, menu: "Go", accelerator: "CmdOrCtrl+K" },
  { id: "jump-to-chat", label: "Jump to chat 1-9", group: "Navigation", chord: { meta: true }, digits: true, display: "⌘1-9" },
  { id: "next-chat", label: "Next chat", group: "Navigation", chord: { key: "ArrowDown", code: "ArrowDown", meta: true, alt: true }, menu: "Go", accelerator: "Alt+CmdOrCtrl+Down" },
  { id: "previous-chat", label: "Previous chat", group: "Navigation", chord: { key: "ArrowUp", code: "ArrowUp", meta: true, alt: true }, menu: "Go", accelerator: "Alt+CmdOrCtrl+Up" },
  { id: "open-projects", label: "Projects", group: "Navigation", chord: { key: "p", meta: true, shift: true }, menu: "Go", accelerator: "Shift+CmdOrCtrl+P" },
  { id: "open-settings", label: "Settings", group: "Navigation", chord: { key: ",", meta: true }, display: "⌘,", menu: "Bridge", accelerator: "CmdOrCtrl+," },
  { id: "toggle-sidebar", label: "Toggle the sidebar", group: "View", chord: { key: "b", meta: true }, menu: "View", accelerator: "CmdOrCtrl+B" },
  // ⌥⌘F rather than ⌃⌘F: the latter is macOS's own native-fullscreen binding,
  // and this is an in-window layout change, not a window state change.
  { id: "toggle-fullscreen", label: "Fullscreen layout", group: "View", chord: { key: "f", code: "KeyF", meta: true, alt: true }, menu: "View", accelerator: "Alt+CmdOrCtrl+F" },
  { id: "toggle-dock", label: "Toggle the dock", group: "View", chord: { key: "0", code: "Digit0", meta: true, alt: true }, menu: "View", accelerator: "Alt+CmdOrCtrl+0" },
  { id: "open-dock-pane", label: "Open dock pane 1-9", group: "View", chord: { meta: true, alt: true }, digits: true, display: "⌥⌘1-9" },
  { id: "expand-dock", label: "Expand the dock", group: "View", chord: { key: "Enter", code: "Enter", meta: true, alt: true }, display: "⌥⌘↩", menu: "View", accelerator: "Alt+CmdOrCtrl+Enter" },
  // Zoom lives in this table rather than in Tauri because the step Tauri takes
  // is a constant in its own crate. `keys` plus `shiftOptional` is what the
  // polyfill did by ignoring shift: the plus key is `=` alone and `+` shifted,
  // and both are one zoom in. Matching on the printed key rather than `code`
  // also keeps the numeric keypad working, which the polyfill had.
  //
  // `whileTyping` because the polyfill listened on the window and zoomed from
  // inside the composer, which is where a keystroke usually lands. Without it
  // these three would be dead for most of a session.
  { id: "zoom-in", label: "Zoom in", group: "View", chord: { key: "=", keys: ["=", "+"], meta: true, shiftOptional: true }, display: "⌘+", whileTyping: true, windowLevel: true, menu: "View", accelerator: "CmdOrCtrl+=" },
  { id: "zoom-out", label: "Zoom out", group: "View", chord: { key: "-", keys: ["-", "_"], meta: true, shiftOptional: true }, display: "⌘-", whileTyping: true, windowLevel: true, menu: "View", accelerator: "CmdOrCtrl+-" },
  { id: "zoom-reset", label: "Actual size", group: "View", chord: { key: "0", meta: true }, display: "⌘0", whileTyping: true, windowLevel: true, menu: "View", accelerator: "CmdOrCtrl+0" },
  { id: "show-shortcuts", label: "Keyboard shortcuts", group: "View", chord: { key: "/", meta: true }, display: "⌘/", menu: "Help", accelerator: "CmdOrCtrl+/" },
];

export const SHORTCUT_GROUPS: ShortcutGroup[] = ["Chat", "Navigation", "View"];

/** The event name the shell emits a menu selection on. Not a protocol method:
 *  the menu lives in the Tauri shell and speaks command ids, not RPC. */
export const MENU_COMMAND_EVENT = "bridge-menu-command";

function metaHeld(stroke: KeyStroke): boolean {
  // One binding for two keyboards. Nothing in the table wants Command and
  // Control to mean different things.
  return stroke.metaKey || stroke.ctrlKey;
}

function modifiersMatch(stroke: KeyStroke, chord: Chord): boolean {
  if (chord.shiftOptional) return metaHeld(stroke) === !!chord.meta && stroke.altKey === !!chord.alt;
  return metaHeld(stroke) === !!chord.meta
    && stroke.altKey === !!chord.alt
    && stroke.shiftKey === !!chord.shift;
}

function keyMatches(stroke: KeyStroke, chord: Chord): boolean {
  if (chord.code && stroke.code === chord.code) return true;
  if (chord.keys) return chord.keys.some(key => stroke.key.toLowerCase() === key.toLowerCase());
  return !!chord.key && stroke.key.toLowerCase() === chord.key.toLowerCase();
}

/** The digit a stroke names, by code first so ⌥ rewriting the printed
 *  character cannot hide it. */
export function strokeDigit(stroke: KeyStroke): number | undefined {
  const physical = /^Digit([0-9])$/.exec(stroke.code)?.[1];
  const printed = /^[0-9]$/.test(stroke.key) ? stroke.key : undefined;
  const digit = physical ?? printed;
  return digit === undefined ? undefined : Number(digit);
}

/** True where a keystroke belongs to whatever the user is writing in. The
 *  terminal pane counts: xterm reads through a hidden textarea. */
export function isTypingTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  if (target.isContentEditable) return true;
  const tag = target.tagName;
  return tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT";
}

export function matchShortcut(stroke: KeyStroke, typing = false): ShortcutMatch | undefined {
  for (const shortcut of SHORTCUTS) {
    if (typing && !shortcut.whileTyping) continue;
    if (!modifiersMatch(stroke, shortcut.chord)) continue;
    if (shortcut.digits) {
      const digit = strokeDigit(stroke);
      // Zero is a command of its own (the dock toggle), never a pane index.
      if (digit === undefined || digit < 1) continue;
      return { shortcut, index: digit - 1 };
    }
    if (keyMatches(stroke, shortcut.chord)) return { shortcut };
  }
  return undefined;
}

const KEY_GLYPHS: Record<string, string> = {
  enter: "↩",
  arrowup: "↑",
  arrowdown: "↓",
  arrowleft: "←",
  arrowright: "→",
  escape: "esc",
  " ": "space",
};

/** macOS prints modifiers ⌃⌥⇧⌘ in that order, always. */
export function formatChord(shortcut: Shortcut): string {
  if (shortcut.display) return shortcut.display;
  const { chord } = shortcut;
  const parts = [chord.alt ? "⌥" : "", chord.shift ? "⇧" : "", chord.meta ? "⌘" : ""];
  const raw = chord.key ?? "";
  const glyph = KEY_GLYPHS[raw.toLowerCase()] ?? (raw.length === 1 ? raw.toUpperCase() : raw);
  return `${parts.join("")}${glyph}`;
}

/** True where the command presents the window rather than its contents, and so
 *  is answered even in the terminal pane where the rest are suppressed. */
export function isWindowLevel(id: CommandId): boolean {
  return shortcutFor(id).windowLevel === true;
}

export function shortcutsInGroup(group: ShortcutGroup): Shortcut[] {
  return SHORTCUTS.filter(shortcut => shortcut.group === group);
}

export function shortcutFor(id: CommandId): Shortcut {
  const found = SHORTCUTS.find(shortcut => shortcut.id === id);
  if (!found) throw new Error(`No shortcut declared for ${id}`);
  return found;
}

/** The chord a command advertises, for a tooltip or an aria-label. */
export function chordLabel(id: CommandId): string {
  return formatChord(shortcutFor(id));
}
