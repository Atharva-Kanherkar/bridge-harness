// Appearance: groups of tiles, each tile a small preview and a radio.
//
// Tiles rather than rows because these are the settings whose value is a look:
// a select naming "Graphite" tells you less than the two swatches do. The selected tile is marked by
// a foreground border, the same mark the rest of the screen uses for "this
// one", and by a real radio so the choice is announced.

import type { ReactNode } from "react";
import { useThemePreference, type ThemePreference, type ThemeSkin } from "../../theme";
import { useShowWorkerChatsInMissionControl } from "../../missionControlSettings";
import { useAutoExpandEditActivity, useShowThinking } from "../../transcriptSettings";
import { SettingsFold, SettingsGroup, SettingsPage, SettingsRow, Switch, TextButton } from "./kit";
import { cn } from "@/lib/utils";
import { canZoom, stepZoom, useZoomLevel } from "../../zoom";
import { useOmarchyTheme } from "../../omarchy";
import { chordLabel } from "../../keymap";
import { PAGES } from "./sections";
import { RowButton } from "@/components/ui/row";

/** A tile previews either two swatches or a drawn figure, never both. */
type Tile<T> = { id: T; label: string; hint: string } & ({ swatches: [string, string]; preview?: undefined } | { preview: ReactNode; swatches?: undefined });

const MODES: Tile<ThemePreference>[] = [
  { id: "system", label: "Match macOS", hint: "Follows your system appearance", swatches: ["bg-[var(--preview-paper-canvas)]", "bg-[var(--preview-graphite-canvas)]"] },
  { id: "light", label: "Paper", hint: "Light mode", swatches: ["bg-[var(--preview-paper-sidebar)]", "bg-[var(--preview-paper-canvas)]"] },
  { id: "dark", label: "Graphite", hint: "Dark mode", swatches: ["bg-[var(--preview-graphite-sidebar)]", "bg-[var(--preview-graphite-canvas)]"] },
];

const SKINS: Tile<ThemeSkin>[] = [
  { id: "graphite", label: "Solid", hint: "Solid window background", swatches: ["bg-foreground/[0.08]", "bg-foreground/[0.16]"] },
  { id: "vibrancy", label: "Vibrancy", hint: "Sidebar tinted by your desktop", swatches: ["bg-foreground/[0.04]", "bg-info/25"] },
];

/* Only offered when the shell has detected Omarchy. The swatches are the live
   theme's own ground and accent, so the tile previews whatever theme is on. */
const OMARCHY_SKIN: Tile<ThemeSkin> = { id: "omarchy", label: "Omarchy", hint: "Follows your Omarchy theme", swatches: ["bg-sidebar", "bg-primary"] };

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
      return <RowButton
        key={tile.id}
        size="bare"
        tone="choice"
        shape="card"
        role="radio"
        aria-checked={selected}
        tabIndex={selected ? 0 : -1}
        onKeyDown={event => {
          const step = event.key === "ArrowRight" || event.key === "ArrowDown" ? 1 : event.key === "ArrowLeft" || event.key === "ArrowUp" ? -1 : 0;
          if (!step && event.key !== "Home" && event.key !== "End") return;
          event.preventDefault();
          const index = tiles.findIndex(candidate => candidate.id === tile.id);
          const next = event.key === "Home" ? 0 : event.key === "End" ? tiles.length - 1 : (index + step + tiles.length) % tiles.length;
          onChange(tiles[next].id);
          event.currentTarget.parentElement?.querySelectorAll<HTMLButtonElement>('[role="radio"]')[next]?.focus();
        }}
        onClick={() => onChange(tile.id)}
        className="items-stretch"
      >
        {/* A column from the top: a tile whose hint wraps must not push its
            neighbours' swatches down. */}
        <span className="flex w-full flex-col justify-start p-2.5">
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
            selected ? "border-foreground" : "border-muted-foreground/50",
          )}>{selected && <span className="size-1.5 rounded-full bg-foreground" />}</span>
          <span className="min-w-0 flex-1 truncate text-ui text-foreground">{tile.label}</span>
        </span>
        <span className="mt-0.5 block text-meta text-muted-foreground">{tile.hint}</span>
        </span>
      </RowButton>;
    })}
  </div>;
}

/* Zoom is a row rather than a tile: the value is a number, and a drawn preview
   of "125%" would say less than the number does. The stepper is here because
   Tauri's own step is not adjustable and the level it leaves behind is
   otherwise only reachable through a chord. */
function ZoomControl({ level, onChange }: { level: number; onChange: (next: number) => void }) {
  return <div className="flex items-center gap-1">
    <TextButton onClick={() => onChange(stepZoom(level, -1))} disabled={!canZoom(level, -1)} ariaLabel="Zoom out">&minus;</TextButton>
    <span aria-live="polite" className="min-w-11 text-center text-ui tabular-nums text-foreground">{Math.round(level * 100)}%</span>
    <TextButton onClick={() => onChange(stepZoom(level, 1))} disabled={!canZoom(level, 1)} ariaLabel="Zoom in">+</TextButton>
  </div>;
}

export function AppearancePage() {
  const { preference, resolved, setPreference, skin, setSkin } = useThemePreference();
  const [showWorkerChats, setShowWorkerChats] = useShowWorkerChatsInMissionControl();
  const [showThinking, setShowThinking] = useShowThinking();
  const [autoExpandEditActivity, setAutoExpandEditActivity] = useAutoExpandEditActivity();
  const [zoom, setZoom] = useZoomLevel();
  const omarchy = useOmarchyTheme();
  const skins = omarchy ? [OMARCHY_SKIN, ...SKINS] : SKINS;
  // A stored `omarchy` without a payload renders as Graphite, so say so.
  const shownSkin: ThemeSkin = skin === "omarchy" && !omarchy ? "graphite" : skin;
  const onOmarchy = shownSkin === "omarchy" && omarchy;
  return <SettingsPage
    title={PAGES.general.label}
    description={PAGES.general.description}
  >
    <SettingsGroup anchor="mode" label="Mode" note={onOmarchy
      ? `Following your Omarchy theme${omarchy.themeName ? ` (${omarchy.themeName})` : ""}, which decides light or dark`
      : `Follows macOS by default. Showing ${resolved === "dark" ? "graphite" : "paper"} now`}>
      <TileGrid name="Mode" tiles={MODES} value={preference} onChange={setPreference} columns="@min-[580px]/settings:grid-cols-3" />
    </SettingsGroup>
    <SettingsGroup anchor="window-background" label="Window background" note="Solid or tinted by your desktop">
      <TileGrid name="Shell" tiles={skins} value={shownSkin} onChange={setSkin} columns={omarchy ? "@min-[580px]/settings:grid-cols-3" : "@min-[480px]/settings:grid-cols-2"} />
    </SettingsGroup>
    <SettingsGroup label="Zoom" note={`Also on ${chordLabel("zoom-in")} and ${chordLabel("zoom-out")}`}>
      <SettingsRow
        anchor="zoom"
        label="Window zoom"
        description={`Scales the whole window, the way ${chordLabel("zoom-in")} does in a browser. Kept between launches.`}
        control={<ZoomControl level={zoom} onChange={setZoom} />}
      />
    </SettingsGroup>
    <SettingsGroup label="Conversation" note="What appears while your agent works">
      <SettingsRow
        anchor="show-thinking"
        label="Show thinking"
        description="Show the thinking text an agent shares while it works. When off, Bridge still shows a working indicator."
        control={<Switch label="Show thinking" checked={showThinking} onChange={setShowThinking} />}
      />
      <SettingsRow
        anchor="edit-activity"
        label="Open edit activity automatically"
        description="Off by default. Short Activity sections with a file diff stay collapsed until you click them. Turn this on to open them automatically."
        control={<Switch label="Open edit activity automatically" checked={autoExpandEditActivity} onChange={setAutoExpandEditActivity} />}
      />
    </SettingsGroup>
    <SettingsFold>
    <SettingsGroup label="Mission Control">
      <SettingsRow
        anchor="mission-control"
        label="Show worker chats in Mission Control"
        description="Off by default: only your main chat appears automatically. Turn on to also show chats for background tasks."
        control={<Switch label="Show worker chats in Mission Control" checked={showWorkerChats} onChange={setShowWorkerChats} />}
      />
    </SettingsGroup>
    </SettingsFold>
  </SettingsPage>;
}
