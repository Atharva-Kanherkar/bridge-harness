# feat/omarchy-look — Test Contract

Phase 1 ("Look") of the Omarchy edition epic, plus an Omarchy mode for the
landing page. Phases 0, 2, 3 and 4 (Landlock sandbox, keycaps, notifications,
argv routing, Hyprland quick prompt, Quickshell plugin, AUR) are out of scope
here and land as their own PRs.

Facts verified against the real Omarchy source (`basecamp/omarchy@7dd477b`,
branch `quattro`) before locking this:

- The active theme is `~/.local/state/omarchy/current/theme/colors.toml`; its
  name is `~/.local/state/omarchy/current/theme.name`.
- `colors.toml` is flat `key = "value"` with `mode = "dark" | "light"`. 23 themes
  ship; `last-horizon`, `solitude` and `white` have no `orange`.
- `shell.toml` `[font]` has only sizes (`base-size`), no family. Omarchy's own
  `omarchy-font-current` reads the family from fontconfig:
  `fc-match monospace -f '%{family}'`, first comma entry. Bridge does the same.
- Omarchy already themes T3 Code from a template; Bridge reads `colors.toml`
  directly instead, so it needs nothing from upstream.

## Functional Behavior

### Desktop app (Linux shell → webview)

- Omarchy is detected when `~/.local/state/omarchy/current/theme` exists, or
  `omarchy` is on `PATH` and `HYPRLAND_INSTANCE_SIGNATURE` is set. On macOS and
  Windows detection is always off and no thread is started.
- The webview asks for the theme on boot (`bridge-omarchy-theme-request`); the
  shell answers on `bridge-omarchy-theme` with
  `{ available, themeName, mode, colors, font }`. `colors` holds only keys whose
  value is a hex colour (`#rgb`, `#rrggbb`, `#rrggbbaa`); anything else is dropped.
- While detected, the shell polls the theme dir every 400ms (theme name and
  `colors.toml` mtime) and re-emits when either changes, so a theme switch
  retints within 1s. A missing or unreadable `colors.toml` emits nothing.
- One table (`src/lib/omarchyTheme.ts`) maps an Omarchy palette onto Bridge's
  existing token names, exactly as the epic specifies (surfaces, ink, lines,
  signal, syntax ramp, `--window-radius: 0px`, `--radius: 0.3rem`). No new token
  names. Harness and chart tints are untouched. A missing role falls back along
  a fixed chain (`orange` → `bright_yellow` → `yellow`, etc.); a palette with no
  `background`, `foreground` or `accent` yields no tokens.
- `ThemeSkin` gains `"omarchy"`. Under it, with a payload, the tokens are set as
  custom properties on `<html>`, the font goes to `--font-sans` and
  `--font-mono`, and light/dark follows the theme's `mode` rather than the
  macOS preference. Without a payload the skin renders as Graphite.
- Leaving the skin removes every property it set.
- Settings > Appearance shows an "Omarchy" tile ("Follows your Omarchy theme")
  only when the shell reports `available: true`. When detected and the user has
  never picked a skin, Omarchy is the default; picking Solid sticks.
- The `theme-color` meta follows the Omarchy `background` under the skin.
- No hex literal enters a component or `index.css`.

### Landing page

- A header toggle labelled "Omarchy" (`aria-pressed`) switches the site into
  Omarchy mode, and a theme picker next to it lists all 23 shipped themes with
  swatches. The choice persists in `localStorage` (`bridge-site-skin`,
  `bridge-site-omarchy-theme`) and is applied by an inline script before first
  paint, so there is no flash. `?omarchy=<theme>` deep-links into the mode;
  `?omarchy=off` leaves it.
- In Omarchy mode the whole page retints through the same token table the app
  uses (byte-identical copy, test-enforced), including the animated app demo,
  so the hero shows what Bridge looks like on that theme. Light themes render
  light.
- Omarchy mode borrows omarchy.org's language: JetBrains Mono everywhere,
  square corners, a square pixel field instead of the round dot lattice, solid
  square primary buttons, and a bordered announcement line above the headline.
- Copy claims only what ships: "Made for Omarchy" and "takes its colours and
  font from your Omarchy theme and retints when you switch". No claim about
  notifications, the bar widget or the AUR.
- Off by default; the Graphite site is unchanged when the toggle is off.

## Unit Tests

- `src-tauri/src/omarchy.rs`
  - `parses_every_shipped_theme` — every fixture in `testing/fixtures/omarchy/themes` parses, has `background`/`foreground`/`accent`, and reports the mode its file states.
  - `drops_non_hex_values_and_comments`
  - `light_mode_marker_file_selects_light` — a `light.mode` file next to `colors.toml` wins when `mode` is absent.
  - `detection_rules` — state dir alone; `omarchy` on PATH + Hyprland signature; neither.
  - `font_family_takes_first_fontconfig_alias` — `"JetBrainsMono Nerd Font,JetBrainsMono NF"` → `"JetBrainsMono Nerd Font"`; empty → `None`.
  - `stamp_changes_when_theme_switches` — rewriting `theme.name` or `colors.toml` changes the poll stamp; an unchanged dir does not.
- `src/lib/omarchyTheme.test.ts`
  - every fixture yields the full token set with no `undefined`;
  - tokyo-night snapshot of the table;
  - `orange` fallback for `white`;
  - light themes use the 12/16/16% border mix, dark 9/13/13%;
  - missing `background` → empty map; non-hex values rejected.
- `src/theme.test.ts`
  - `omarchy` is a valid skin; Graphite stays the default when not detected;
  - with a payload, the skin sets tokens + font on `<html>` and forces the theme's mode;
  - without a payload it sets nothing (Graphite rendering);
  - switching back to graphite removes every property.
- `src/components/settings/AppearancePage.test.tsx` — the Omarchy tile is absent until a payload with `available: true` arrives, then present.
- `src/designSystem.test.ts` — unchanged and green.
- `landing/test/omarchy.test.ts`
  - `landing/app/lib/omarchyTheme.ts` is byte-identical to `src/lib/omarchyTheme.ts`;
  - `landing/app/content/omarchyThemes.ts` matches every fixture file (names, mode, colours);
  - the pre-paint script handles `?omarchy=off`, unknown theme names, and blocked storage.

## Integration / Functional Tests

- `cargo test -p bridge` (shell crate) runs the `omarchy` module tests on macOS (parsing and detection are platform-neutral; only the poll thread is Linux-gated).
- `cargo check --workspace` green.
- `bun run test` and `bun run build` green at the root; `bun run build` and `bun test test` green in `landing/`.

## Smoke Tests

- `bun run dev`: app in mock mode boots, Appearance shows no Omarchy tile (not detected on macOS).
- Mock mode, payload injected through `receiveOmarchyTheme` from the console: tokyo-night and rose-pine retint the app; switching to Solid restores Graphite.
- Landing `next dev`: toggle on, pick gruvbox, reload: still gruvbox, no flash; toggle off: Graphite.

## E2E Tests

N/A in CI: there is no Omarchy runner. Manual on an Omarchy 4 install is
recorded in the PR as not yet run (no Linux machine in this session).

## Manual / cURL Tests

- Screenshots for the PR (dark/light/interaction): landing Graphite vs Omarchy
  tokyo-night vs Omarchy flexoki-light, the theme picker open, and the app in
  mock mode under tokyo-night and rose-pine.
- `curl -s localhost:3000/?omarchy=tokyo-night | grep data-omarchy` shows the
  pre-paint script is in the HTML.
