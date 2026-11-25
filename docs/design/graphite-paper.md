# Graphite &amp; Paper

Bridge's design system. Bridge hosts Codex, Claude Code, and OpenCode side by side, so its chrome
stays achromatic and **colour only ever carries meaning** — agent status, diffs, and approvals.

The direction was drawn from how those three products dress themselves. They converge on the same
grammar: elevation as a lightness ladder rather than borders and shadows, near-zero chroma in the
chrome, a pill composer, monospace reserved for code and paths, tool calls collapsed into labelled
rows, and no glass anywhere.

How feature code may use these tokens, and the primitives built on them, is in
[`ui-rules.md`](ui-rules.md). `bun run lint` enforces it.

## Tokens

Both modes are renderings of the same token names, defined in [`src/index.css`](../../src/index.css).

|                | Light · paper | Dark · graphite |
| -------------- | ------------- | --------------- |
| `--sidebar`    | `#F4F4F3`     | `#09090A`       |
| `--background` | `#FBFBFA`     | `#0C0C0D`       |
| `--card`       | `#FFFFFF`     | `#141416`       |
| `--popover`    | `#FFFFFF`     | `#19191C`       |
| `--border`     | `#E7E7E4`     | `#1D1D20`       |
| `--foreground` | `#18181A`     | `#EDEDEF`       |
| `--muted-foreground` | `#66666B` | `#8E8E94`      |
| `--primary`    | `#18181A`     | `#EDEDEF`       |
| `--ring`       | `#5B63D3`     | `#8B91F0`       |
| `--success`    | `#2F7D4F`     | `#5DB872`       |
| `--info`       | `#3D6FA5`     | `#6A9BCC`       |
| `--destructive`| `#BE3D3D`     | `#E5695E`       |

Four rules hold the system together:

1. **Elevation is a ladder** — `sidebar` < `background` < `card` < `popover`. Resting surfaces are
   opaque; only genuinely floating layers (menus, dialogs, toasts) get a shadow. Nothing blurs at rest.
2. **The primary action is an inversion**, not a hue: `bg-primary text-primary-foreground` is ink on
   paper and paper on ink.
3. **One slate signal** carries focus rings, links, text selection, and the active-session tick. It is
   interaction plumbing, deliberately too quiet to brand anything.
4. **Status ink is the hue itself** (`text-success`). The `*-foreground` variants are the ink that sits
   *on* a full-strength status fill and are near-black in dark mode — never use them on a wash.

Type is a single family: Geist 400/500/600, with hierarchy from weight and size rather than a second
display face. Geist Mono is reserved for paths, commands, telemetry, and diffs.

Every UI size comes from one scale:

| Token | Size | For |
| --- | --- | --- |
| `text-micro` | 10.5px | badges, key caps |
| `text-caption` | 11.5px | times, counts, section labels |
| `text-meta` | 12.5px | secondary rows, chips |
| `text-ui` | 13px | controls and lists |
| `text-body` | 14px | dialogs and panes |
| `text-message` | 15px | reading |
| `text-heading` | 18px | section titles |
| `text-title` | 24px | page titles |

Labels are sentence case: no all-caps, no positive letter-spacing. Large type tightens instead
(about -0.02em to -0.03em). Corners come from the radius scale (`rounded-sm` to `rounded-2xl`).

## Session view

The transcript follows the grammar the reference products share: the user's message in a bubble, the
agent's reply naked on the canvas, tool calls as collapsed mono rows with the result right-aligned,
and the diff carrying the only saturated colour.

### Dark · graphite

![Session view in dark mode](graphite-paper/session-dark.png)

### Light · paper

![Session view in light mode](graphite-paper/session-light.png)

## Mission Control

Tiles sit one ladder step above the canvas. Status is carried by the semantic ramp, the live bar is a
solid success edge, and an approval is a neutral row with a warning tick rather than a tinted alarm
panel — so a grid of busy agents never becomes a wall of colour.

### Dark · graphite

![Mission Control in dark mode](graphite-paper/mission-dark.png)

### Light · paper

![Mission Control in light mode](graphite-paper/mission-light.png)

## Responsive

The window goes down to 420px. Below the `sm` breakpoint the sidebar becomes an off-canvas drawer over
a scrim, with a title bar that clears the macOS traffic lights.

| Transcript at 390px | Drawer open | Paper at 390px |
| --- | --- | --- |
| ![Transcript at 390px](graphite-paper/mobile-dark.png) | ![Navigation drawer open](graphite-paper/mobile-drawer-dark.png) | ![Transcript at 390px in light mode](graphite-paper/mobile-light.png) |

## Theming

The app follows macOS appearance unless a mode is pinned in **Settings › Appearance**. The preference
is stored under `bridge.theme` and resolved by an inline script in `index.html` before first paint, so
the window never flashes the wrong ground. [`src/theme.ts`](../../src/theme.ts) owns the logic;
`useThemePreference()` keeps every consumer in sync.

Anything that cannot read a CSS custom property — the xterm terminal, mermaid diagrams — reads the
resolved tokens from the document and re-themes when the mode flips.

## Guard rails

[`src/designSystem.test.ts`](../../src/designSystem.test.ts) fails the build on:

- a Tailwind palette class (`text-neutral-500`, `bg-emerald-400`, …) anywhere in `src/`
- a white/black alpha wash (`bg-white/[0.05]`, `border-white/10`)
- a raw hex literal outside the one allowlisted file
- `backdrop-blur` on a class string that also names a resting surface
- `text-<status>-foreground` used anywhere but on a full-strength `bg-<status>`
- an arbitrary text size under 16px (`text-[12px]`): use the type scale
- `uppercase` or positive tracking on labels
- a one-off pixel radius (`rounded-[5px]`)

It also asserts the ladder rungs stay distinct within each mode, that both grounds are exactly
`#FBFBFA` and `#0C0C0D`, that `--primary` equals `--foreground`, that no `color-scheme` is pinned on
the document, and that reduced motion is honoured.

Screenshots on this page are the real components rendered against the real compiled CSS with
representative session data.
