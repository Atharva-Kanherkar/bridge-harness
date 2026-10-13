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
  `mode` resolves the way Omarchy's own `omarchy-theme-color` does.
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
  - `mode_follows_omarchy_precedence` — mirrors `omarchy-theme-color`: `mode` key, then legacy `theme_type`, then a `light.mode` file beside `colors.toml`, then background luminance (r+g+b > 382 is light), else dark.
  - `detection_rules` — state dir alone; `omarchy` on PATH + Hyprland signature; neither.
  - `font_family_takes_first_fontconfig_alias` — `"JetBrainsMono Nerd Font,JetBrainsMono NF"` → `"JetBrainsMono Nerd Font"`; empty → `None`.
  - `stamp_changes_when_theme_switches` — rewriting `theme.name` or `colors.toml` changes the poll stamp; an unchanged dir does not.
- `src/lib/omarchyTheme.test.ts`
  - every fixture yields the full token set with no `undefined`;
  - tokyo-night snapshot of the table;
  - `orange` fallback for `white`;
  - light themes use the 12/16/16% border mix, dark 9/13/13%;
  - missing `background` → empty map; non-hex values rejected.
  - legacy palettes resolve the way `omarchy-theme-color` does: `bg`/`fg` short names, ANSI `color0`..`color15` (the `omarchy-theme-colors-from-alacritty` shape), `purple`, and derived shades (25%/50% toward black, bright hues 20% toward white); shipped themes are unaffected.
- `src/theme.test.ts`
  - `omarchy` is a valid skin; Graphite stays the default when not detected;
  - with a payload, the skin sets tokens + font on `<html>` and forces the theme's mode;
  - without a payload it sets nothing (Graphite rendering);
  - switching back to graphite removes every property.
  - a payload delivered after render but before the hook subscribes is still applied (startup race).
- `src/components/settings/AppearancePage.test.tsx` — the Omarchy tile is absent until a payload with `available: true` arrives, then present.
- `src/designSystem.test.ts` — unchanged and green.
- `landing/test/omarchy.test.ts`
  - `landing/app/lib/omarchyTheme.ts` is byte-identical to `src/lib/omarchyTheme.ts`;
  - `landing/app/content/omarchyThemes.ts` matches every fixture file (names, mode, colours);
  - the pre-paint script handles `?omarchy=off`, unknown theme names, and blocked storage.
  - a choice made on the page after a `?omarchy=` deep link survives reload (the controller drops the param; boot does not rewrite it).

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

---

# Part 2: the rest of the Omarchy edition (Phases 0, 2, 3, 4)

Locked before implementation, same branch. Linux-only code is compiled and
tested in a Linux container (Docker, kernel 6.12, Landlock ABI 6); anything a
container cannot exercise (Hyprland, Quickshell, a notification daemon) is
listed under Manual.

## Functional Behavior

### Phase 0: Linux baseline
- **Keycaps.** `src/lib/platform.ts` exports `isMacPlatform()` and
  `formatShortcut(keys)`. On macOS `["mod","shift","k"]` renders `⌘⇧K`; elsewhere
  `Ctrl+Shift+K`. No `⌘` literal remains in non-test `src/` files outside the helper.
- **Landlock read-only workers.** On Linux, `worker_sandbox::command` builds a
  Landlock ruleset in the parent and applies it in the child before exec
  (`PR_SET_NO_NEW_PRIVS` + `landlock_restrict_self`, both async-signal-safe).
  Reads and execution are allowed everywhere; writes only under the worker
  output dir and the same device exemptions as the seatbelt profile
  (`/dev/null`, `/dev/zero`, `/dev/ptmx`, `/dev/pts`, `/dev/tty`). When
  `runtime_network_denied`, TCP connect and bind are denied (ABI 4+). Fails
  closed with the existing error style when the kernel lacks Landlock, when
  ABI < 3 (truncate is unrestricted before 3), or when network denial is
  required and ABI < 4. The parent-death watchdog wrapper still runs.
  `docs/linux-release.md` "Known blocker" is removed.
- **Lazy agent launchers.** A version probe that times out on the first try is
  retried once with a longer budget, so a mise-backed launcher that installs
  its tool on first run is not reported unavailable.
- **NVIDIA on Wayland.** When `/proc/driver/nvidia` exists and the session is
  Wayland, the shell sets `WEBKIT_DISABLE_DMABUF_RENDERER=1` before the
  webview starts, unless the user already set it.

### Phase 2: desktop integration
- **Hyprland chrome.** Under `HYPRLAND_INSTANCE_SIGNATURE` the main window has
  no decorations and no transparency; the app id / WM class is `bridge`.
- **argv routing.** `bridge-deck --open <sessionId>`, `--new-chat [--project <path>]`,
  `--quick`, `--approvals`, and `--omarchy <setup|remove|retint>` are parsed by one
  pure function; in a second instance they are forwarded through the
  single-instance plugin to the running app as a `bridge-launch` event.
- **Omarchy setup/remove.** `bridge-deck --omarchy setup [--hyprland]` writes only
  under `~/.config`: the `theme-set.d` hook, a `bridge` subtree merged into
  `omarchy/extensions/omarchy-menu.jsonc`, the Quickshell plugin under
  `omarchy/plugins/app.bridge.status/`, and with `--hyprland` a
  `hypr/bridge.lua` plus one `require` line in the user's Hyprland config.
  It is idempotent. Every user file it modifies is backed up first;
  `--omarchy remove` restores those byte for byte and deletes everything it created.
- **Notifications (Linux).** Approvals raise a freedesktop notification
  (`notify-rust`, urgency critical, app name `Bridge`) with actions
  `allow-once`, `decline`, `open`. An action resolves through the same
  `resolve_approval` path as the in-app button; a second resolution of the same
  approval is a no-op reported as "handled already".
- **Quick prompt.** `--quick` toggles a second webview labelled `quick`
  (composer only). Esc hides it; the session keeps running.

### Phase 3: Quickshell plugin + CLI
- **CLI.** `bridge status --json [--watch]` prints
  `{ running, working, waiting, sessions:[...] }` (one NDJSON line per change
  with `--watch`); `bridge sessions --json` lists sessions;
  `bridge approval <allow-once|decline> <sessionId> <eventId>` resolves one.
  All built from existing protocol methods; no new wire method.
- **Plugin.** `integrations/omarchy/plugin/` holds `manifest.json`
  (`schemaVersion: 1`, `id: "app.bridge.status"`, `kinds: ["bar-widget","panel"]`),
  `BarWidget.qml` (working and waiting counts, hidden when `bridged` is not
  running) and `Panel.qml` (Needs you with Allow once / Decline / Open, then
  Working). The QML only runs the `bridge` CLI; it holds no secrets.

### Phase 4: distribution
- `.desktop` actions `new-chat`, `quick-prompt`, `approvals`, `mission-control`.
- `packaging/arch/PKGBUILD` gains `hyprland` and `quickshell` as `optdepends` only.
- `docs/omarchy.md`: install, setup, what is written where, removal, troubleshooting.

## Unit Tests
- `src/lib/platform.test.ts`: mac and linux rendering; a lint test that fails on a `⌘` literal outside the helper.
- `worker_sandbox.rs` (Linux, run in Docker): write outside the output dir fails with EACCES; write inside succeeds; `/dev/null` writes succeed; TCP connect fails when network is denied and succeeds otherwise; ABI gating returns the fail-closed error.
- `health`/probe: a probe that times out once and then succeeds is reported available.
- `launch.rs`: argv parsing for every flag, unknown flags ignored, `--omarchy` subcommands validated.
- `omarchy_setup.rs`: setup then remove on a temp `$HOME` with pre-existing `omarchy-menu.jsonc` and Hyprland config leaves them byte-identical; setup twice is a no-op; nothing is written outside `~/.config`.
- `bridge` CLI: `status` and `approval` argument parsing and JSON shape against an in-process daemon.
- Notification action mapping: `allow-once` → `accept`, `decline` → `decline`; a double resolution is a no-op.
- Plugin: `manifest.json` validates against the fields above.

## Integration / Smoke
- `bun run test`, `bun run build` green; Linux container: `cargo test -p bridge-core worker_sandbox` green.

## Manual (not possible in this session)
- On Omarchy 4: setup from the UI path, bar widget counts, notification actions, `SUPER+SHIFT+CTRL+B` quick prompt, `hyprctl clients` shows class `bridge`, NVIDIA and Intel/AMD at 1x/2x. Recorded in the PR as not yet run.
