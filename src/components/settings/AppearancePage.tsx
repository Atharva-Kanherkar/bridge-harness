// Appearance: three groups of tiles, each tile a small preview and a radio.
//
// Tiles rather than rows because these are the settings whose value is a look:
// a select naming "Graphite" tells you less than the two swatches do, and a
// select naming "Slider" less than a drawn rail. The selected tile is marked by
// a foreground border, the same mark the rest of the screen uses for "this
// one", and by a real radio so the choice is announced.

import type { ReactNode } from "react";
import { useThemePreference, type EffortSelectorStyle, type ThemePreference, type ThemeSkin } from "../../theme";
import { SettingsGroup, SettingsPage } from "./kit";
import { cn } from "@/lib/utils";

/** A tile previews either two swatches or a drawn figure, never both. */
type Tile<T> = { id: T; label: string; hint: string } & ({ swatches: [string, string]; preview?: undefined } | { preview: ReactNode; swatches?: undefined });

const MODES: Tile<ThemePreference>[] = [
  { id: "system", label: "Match macOS", hint: "Follows your system appearance", swatches: ["bg-[var(--preview-paper-canvas)]", "bg-[var(--preview-graphite-canvas)]"] },
  { id: "light", label: "Paper", hint: "Light mode", swatches: ["bg-[var(--preview-paper-sidebar)]", "bg-[var(--preview-paper-canvas)]"] },
  { id: "dark", label: "Graphite", hint: "Dark mode", swatches: ["bg-[var(--preview-graphite-sidebar)]", "bg-[var(--preview-graphite-canvas)]"] },
];

const SKINS: Tile<ThemeSkin>[] = [
  { id: "graphite", label: "Solid", hint: "Opaque shell", swatches: ["bg-foreground/[0.08]", "bg-foreground/[0.16]"] },
  { id: "vibrancy", label: "Cursor", hint: "Translucent, tints with your wallpaper", swatches: ["bg-foreground/[0.04]", "bg-info/25"] },
];

/* Static previews of the three thinking controls, drawn in the page's ink so
   the tile reads as a form, not as the live control. */
const SLIDER_PREVIEW = <span className="relative block h-full w-full">
  <span className="absolute inset-x-3 top-1/2 h-1 -translate-y-1/2 rounded-full bg-foreground/[0.12]" />
  <span className="absolute left-3 top-1/2 h-1 w-[55%] -translate-y-1/2 rounded-full bg-foreground/55" />
  <span className="absolute left-[calc(3*0.25rem+55%)] top-1/2 size-2.5 -translate-x-1/2 -translate-y-1/2 rounded-full border border-background bg-foreground" />
</span>;
const SENTENCE_PREVIEW = <span className="flex h-full items-center justify-center gap-1 text-[10px] text-muted-foreground">
  Think <span className="rounded border border-border bg-muted px-1 font-semibold text-foreground">deeply</span>
</span>;
const LIST_PREVIEW = <span className="flex h-full flex-col justify-center gap-1 px-3">
  {[0.3, 0.55, 0.42].map((width, i) => <span key={i} className="flex items-center gap-1.5">
    <span className={cn("size-1.5 rounded-full border", i === 1 ? "border-foreground bg-foreground" : "border-foreground/40")} />
    <span className={cn("h-1 rounded-full", i === 1 ? "bg-foreground/70" : "bg-foreground/[0.18]")} style={{ width: `${width * 100}%` }} />
  </span>)}
</span>;

const EFFORT_STYLES: Tile<EffortSelectorStyle>[] = [
  { id: "slider", label: "Slider", hint: "A rail with one tick per level", preview: SLIDER_PREVIEW },
  { id: "sentence", label: "Sentence", hint: "Scrub a word in a line of prose", preview: SENTENCE_PREVIEW },
  { id: "list", label: "List", hint: "Two panes; effort as a vertical list", preview: LIST_PREVIEW },
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
          {tile.swatches
            ? <>
              <span className={cn("h-full flex-1", tile.swatches[0])} />
              <span className={cn("h-full flex-1", tile.swatches[1])} />
            </>
            : <span className="block h-full w-full">{tile.preview}</span>}
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
  const { preference, resolved, setPreference, skin, setSkin, effortSelector, setEffortSelector } = useThemePreference();
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
    <SettingsGroup label="Thinking control" note="How the model picker sets reasoning effort">
      <TileGrid name="Thinking control" tiles={EFFORT_STYLES} value={effortSelector} onChange={setEffortSelector} columns="sm:grid-cols-3" />
    </SettingsGroup>
  </SettingsPage>;
}
