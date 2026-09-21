import type { MenuBarLayoutToken } from "../../protocol/generated/protocol";

export const layoutTokens: { value: MenuBarLayoutToken; label: string; example: string }[] = [
  { value: "icon", label: "Icon", example: "▥" },
  { value: "provider", label: "Provider", example: "Codex" },
  { value: "used", label: "Used", example: "42%" },
  { value: "remaining", label: "Remaining", example: "58%" },
  { value: "fiveHourUsed", label: "5-hour used", example: "5h 42%" },
  { value: "fiveHourRemaining", label: "5-hour left", example: "5h 58%" },
  { value: "weeklyUsed", label: "Weekly used", example: "7d 74%" },
  { value: "weeklyRemaining", label: "Weekly left", example: "7d 26%" },
  { value: "reset", label: "Reset", example: "2h 10m" },
  { value: "todayCost", label: "Today's spend", example: "≈$1.20" },
  { value: "dot", label: "Separator", example: " · " },
  { value: "space", label: "Space", example: " " },
];

export function parseLayout(text: string): MenuBarLayoutToken[][] {
  const value: unknown = JSON.parse(text);
  if (!Array.isArray(value) || value.length > 2 || value.some(line => !Array.isArray(line) || line.length > 12 || line.some(token => !layoutTokens.some(item => item.value === token))) || value.flat().filter(token => token === "icon").length > 1) {
    throw new Error("Use up to two lines, twelve items per line, and one icon. Choose items from the buttons above.");
  }
  return value as MenuBarLayoutToken[][];
}

export function layoutExample(layout: MenuBarLayoutToken[][]): string {
  return layout.map(line => line.map(token => layoutTokens.find(item => item.value === token)!.example).join("")).join("\n");
}
