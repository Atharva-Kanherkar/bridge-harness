// Appearance: two groups of tiles, each tile a two-swatch preview and a radio.
//
// Tiles rather than rows because this is the one setting whose value is a look:
// a select naming "Graphite" tells you less than the two swatches do. The
// selected tile is marked by a foreground border, the same mark the rest of the
// screen uses for "this one", and by a real radio so the choice is announced.

import { useThemePreference, type ThemePreference, type ThemeSkin } from "../../theme";
import { SettingsGroup, SettingsPage } from "./kit";
import { cn } from "@/lib/utils";

type Tile<T> = { id: T; label: string; hint: string; swatches: [string, string] };

const MODES: Tile<ThemePreference>[] = [
  { id: "system", label: "Match macOS", hint: "Follows your system appearance", swatches: ["bg-[var(--preview-paper-canvas)]", "bg-[var(--preview-graphite-canvas)]"] },
  { id: "light", label: "Paper", hint: "Light mode", swatches: ["bg-[var(--preview-paper-sidebar)]", "bg-[var(--preview-paper-canvas)]"] },
  { id: "dark", label: "Graphite", hint: "Dark mode", swatches: ["bg-[var(--preview-graphite-sidebar)]", "bg-[var(--preview-graphite-canvas)]"] },
];

const SKINS: Tile<ThemeSkin>[] = [
  { id: "graphite", label: "Solid", hint: "Opaque shell", swatches: ["bg-foreground/[0.08]", "bg-foreground/[0.16]"] },
  { id: "vibrancy", label: "Cursor", hint: "Translucent, tints with your wallpaper", swatches: ["bg-foreground/[0.04]", "bg-info/25"] },
];

function TileGrid<T extends string>({ name, tiles, value, onChange, columns }: {
  name: string;
  tiles: Tile<T>[];
  value: T;
  onChange: (next: T) => void;
  columns: string;
}) {
  return <div role="radiogroup" aria-label={name} className={cn("grid gap-2 p-2.5", columns)}>
    {tiles.map(tile => {
      const selected = tile.id === value;
      return <button
        key={tile.id}
        type="button"
        role="radio"
        aria-checked={selected}
        onClick={() => onChange(tile.id)}
        className={cn(
          "rounded-lg border p-2.5 text-left transition-colors",
          selected ? "border-foreground" : "border-border-card hover:bg-accent",
        )}
      >
        <span aria-hidden="true" className="mb-2 flex h-8 overflow-hidden rounded-md border border-border">
          <span className={cn("h-full flex-1", tile.swatches[0])} />
          <span className={cn("h-full flex-1", tile.swatches[1])} />
        </span>
        <span className="flex items-center gap-1.5">
          <span aria-hidden="true" className={cn(
            "grid size-3 shrink-0 place-items-center rounded-full border",
            selected ? "border-foreground" : "border-border",
          )}>{selected && <span className="size-1.5 rounded-full bg-foreground" />}</span>
          <span className="min-w-0 flex-1 truncate text-[13px] text-foreground">{tile.label}</span>
        </span>
        <span className="mt-0.5 block text-[11.5px] text-muted-foreground">{tile.hint}</span>
      </button>;
    })}
  </div>;
}

export function AppearancePage() {
  const { preference, resolved, setPreference, skin, setSkin } = useThemePreference();
  return <SettingsPage
    title="Appearance"
    description={`Bridge follows macOS by default. Currently showing ${resolved === "dark" ? "graphite" : "paper"}.`}
  >
    <SettingsGroup label="Mode">
      <TileGrid name="Mode" tiles={MODES} value={preference} onChange={setPreference} columns="sm:grid-cols-3" />
    </SettingsGroup>
    <SettingsGroup label="Shell" note="The surface behind the app">
      <TileGrid name="Shell" tiles={SKINS} value={skin} onChange={setSkin} columns="sm:grid-cols-2" />
    </SettingsGroup>
  </SettingsPage>;
}
