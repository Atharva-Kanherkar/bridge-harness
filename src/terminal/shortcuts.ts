export type TerminalCommand = "new-tab" | "split-right" | "split-down" | "close-pane" | "maximize" | "next-pane" | "previous-pane" | "next-tab" | "previous-tab" | "search";
export const TERMINAL_SHORTCUTS: { id: TerminalCommand; key: string; shift?: boolean; alt?: boolean; label: string }[] = [
  { id: "new-tab", key: "t", label: "New terminal tab" },
  { id: "split-right", key: "d", label: "Split right" },
  { id: "split-down", key: "d", shift: true, label: "Split down" },
  { id: "close-pane", key: "w", shift: true, label: "Close focused pane" },
  { id: "maximize", key: "Enter", shift: true, label: "Maximize pane" },
  { id: "next-pane", key: "ArrowRight", alt: true, label: "Next pane" },
  { id: "previous-pane", key: "ArrowLeft", alt: true, label: "Previous pane" },
  { id: "next-tab", key: "]", shift: true, label: "Next tab" },
  { id: "previous-tab", key: "[", shift: true, label: "Previous tab" },
  { id: "search", key: "f", label: "Search scrollback" },
];
const isMac = () => /Mac|iPhone|iPad/.test(navigator.platform);
export function terminalChord(id: TerminalCommand) {
  const s = TERMINAL_SHORTCUTS.find(s => s.id === id)!;
  return `${isMac() ? "⌘" : "Ctrl+Alt+"}${s.alt && isMac() ? "⌥" : ""}${s.shift ? (isMac() ? "⇧" : "Shift+") : ""}${s.key.replace("ArrowRight", "→").replace("ArrowLeft", "←").replace("Enter", "↩").toUpperCase()}`;
}
export function terminalCommand(event: Pick<KeyboardEvent, "metaKey" | "ctrlKey" | "shiftKey" | "altKey" | "key" | "code">): TerminalCommand | undefined {
  // Plain Control belongs to the CLI, including Ctrl+C, Ctrl+D and Ctrl+F.
  if (isMac() ? !event.metaKey || event.ctrlKey : !event.ctrlKey || event.metaKey || !event.altKey) return;
  return TERMINAL_SHORTCUTS.find(s => {
    const key = event.code === "BracketLeft" ? "[" : event.code === "BracketRight" ? "]" : event.key;
    return s.key.toLowerCase() === key.toLowerCase() && !!s.shift === event.shiftKey && (!isMac() || !!s.alt === event.altKey);
  })?.id;
}
