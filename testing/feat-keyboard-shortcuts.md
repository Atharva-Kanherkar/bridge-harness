# feat/keyboard-shortcuts — test contract

Locked before implementation. One workstream: the app answers to the
keyboard. Today every global binding lives in a single hand-rolled effect —
⌥⌘F, ⌥⌘0, ⌥⌘1-9, ⌥⌘Return, Escape — and nothing else in the app has a
chord at all. New chat, the most frequent action there is, is mouse-only;
recall search is a state toggle with no binding; the shell builds no native
menu, so macOS's own conventions are absent and nothing is discoverable.

The failure mode this removes is not "a chord is missing". It is that a
shortcut has to agree with itself in four places — the handler that runs
it, the tooltip that advertises it, the native menu item that duplicates
it, and the sheet that lists it — and four hand-maintained copies drift.
One table cannot.

## Shape of the thing

`src/keymap.ts` is the table: every command's id, label, group, chord,
whether it fires while typing, and where it belongs in the native menu. It
owns matching (`matchShortcut`), the typing guard (`isTypingTarget`), and
display formatting (`formatChord`). It knows nothing about React and
nothing about Tauri, so it is unit-testable without either.

App keeps one keydown effect that asks the table what a key means and
dispatches to handlers it already has. The dock chords move onto the table
rather than staying hand-rolled beside it. Escape stays outside the table:
it unwinds whichever layer is nearest and is not a command.

`src/components/ShortcutsSheet.tsx` renders the table, grouped, from the
same source — never a second list. The shell builds a macOS menu whose
items carry the same ids and the same accelerators, and emits the command
id to the webview; the frontend runs the same dispatch either way. The
menu carries the predefined Edit items, because a custom macOS menu without
them silently breaks copy and paste in the webview.

## 1. The table — `src/keymap.test.ts`

| # | Behaviour | Assertion |
|---|---|---|
| 1.1 | A plain chord matches its own event and nothing else | ⌘N matches meta+n; ⇧⌘N does not |
| 1.2 | Shift is part of the chord, not noise | ⇧⌘N matches only with shift held, and ⌘N only without |
| 1.3 | Option-rewritten keys are matched physically | ⌥⌘F matches by code when macOS rewrites the printed key |
| 1.4 | Digit families resolve to an index | ⌘1-9 and ⌥⌘1-9 return the index they name, and 0 is not one of them |
| 1.5 | ⌥⌘0 stays its own command | the dock toggle matches at digit zero and never as a pane index |
| 1.6 | Ctrl stands in for Cmd | a Control-held event matches a Command chord |
| 1.7 | Typing suppresses everything not marked otherwise | while typing only the whileTyping commands match |
| 1.8 | The typing guard knows a text surface | inputs, textareas and contenteditable are typing targets; a button is not |
| 1.9 | No two commands claim the same chord | every chord in the table is unique |
| 1.10 | Display strings follow macOS order | modifiers render ⌃⌥⇧⌘ then the key |
| 1.11 | Menu accelerators agree with their chords | every command carrying an accelerator spells the same chord it matches |

## 2. The sheet — `src/components/ShortcutsSheet.test.tsx`

| # | Behaviour | Assertion |
|---|---|---|
| 2.1 | Every command is listed | the sheet renders one row per table entry, none missing |
| 2.2 | Rows are grouped by the table's own groups | each group heading appears with its commands under it |
| 2.3 | Chords render as chords | a row shows the formatted chord, not a raw key name |
| 2.4 | It closes | the close control invokes the host's handler |

## 3. In the app — `src/App.shortcuts.test.tsx`

| # | Behaviour | Assertion |
|---|---|---|
| 3.1 | ⌘N starts a chat without the mouse | pressing it from the welcome surface lands in a chat |
| 3.2 | ⌘/ opens the sheet, and closes it again | the sheet appears on the first press and is gone after Escape |
| 3.3 | ⌘B toggles the rail | the sidebar's collapsed state flips and persists |
| 3.4 | Typing in the composer is never a command | ⌘-less keys and plain letters typed into the composer reach the field and start nothing |
| 3.5 | The dock chords still work after moving onto the table | ⌥⌘0 toggles the dock and ⌥⌘F still toggles fullscreen |

## Out of scope

User-remappable bindings. The table is what makes them possible later; the
persistence, the editor, and the conflict resolution are not this change.
