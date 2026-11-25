# UI rules

How Bridge's interface stays one interface. These rules are enforced by
`bun run lint`; this page says what to do instead when a rule fires.

```mermaid
flowchart LR
  T[Tokens<br/>src/index.css] --> P[Primitives<br/>src/components/ui]
  P --> F[Feature code<br/>layout classes only]
  F --> L{bun run lint}
  L -- new violation --> F
  L -- clean --> M[Merge]
```

## The three layers

| Layer | Where | Owns |
|---|---|---|
| Tokens | `src/index.css` (`@theme`, `:root`, `.dark`) | Every colour, type size, radius, shadow and easing curve |
| Primitives | `src/components/ui/` | How a control looks in every state: size, padding, colour, radius, focus, hover, disabled |
| Feature code | everything else in `src/` | What is on screen and where: layout, spacing between things, content |

Feature code composes primitives and positions them. It does not decide what a
button, field, row or tooltip looks like.

## Rules

1. **Pick a `variant` and a `size`; do not restyle a primitive with
   `className`.** Layout classes are fine on a primitive (width, flex, grid,
   margin, position, `truncate`). Padding, colour, type, radius and shadow are
   not. (`shadcn/no-restyle`)
2. **No raw `<button>`, `<input>`, `<select>` or `<textarea>` in feature
   code.** Use the primitive. (`bridge/no-raw-controls`)
3. **No native `title` tooltips.** Use `<Tip label="…">` or `tooltip=` on
   `Button`. An icon-only control still needs its own `aria-label`.
   (`bridge/no-native-title-tooltip`)
4. **No arbitrary appearance values.** `text-[13px]`, `rounded-[10px]`,
   `p-[7px]`, `bg-[#333]` are errors. Arbitrary *layout* values
   (`w-[320px]`, `grid-cols-[1fr_auto]`, `max-h-[60vh]`) are allowed.
   (`shadcn/no-arbitrary-values`)
5. **Only theme colours.** `text-red-500` and colours that are not declared in
   `src/index.css` are errors. (`shadcn/no-raw-colors`)
6. **Only classes Tailwind can generate.** A typo such as
   `transition-shadows` renders nothing and is an error.
   (`shadcn/no-unknown-classes`)
7. **Static class strings on primitives**, so the linter can read them. Use
   `cn("a", cond && "b")`, not a template string. (`shadcn/require-static-classes`)
8. **No inline `style` for a static value.** Inline style is for values
   computed at runtime (a measured width, a per-item delay).

## What to reach for

| You need | Use |
|---|---|
| An action beside other content | `Button` (`variant`: `default` `outline` `secondary` `ghost` `quiet` `text` `link` `chip` `destructive` `destructive-outline` `destructive-ghost` `destructive-quiet`; `size`: `2xs` `xs` `sm` `default` `lg` `xl`, `icon-3xs` … `icon-xl`, `inline`; `shape="round"` for the composer's send and stop) |
| A filter pill that toggles | `Button variant="chip" size="2xs" aria-pressed={on}` |
| A row's delete or remove | `Button variant="destructive-quiet" size="icon-xs"`: muted until hovered |
| A link styled as a button | `Button render={<a href=… />}` |
| The small text action on a rendered block (Copy, Expand, Zoom) | `BlockAction` (`floating` pins it to the block's corner) |
| An icon-only action | `Button size="icon-xs" variant="quiet" tooltip="Close tab"` |
| A link-like action inside a sentence | `Button variant="text" size="inline"` |
| A line that is clicked as a whole (list item, tree node, menu-like choice) | `RowButton` (`size`: `sm` `default` `lg` `card` `bare`; `tone`: `default` `muted` `selection` `nav` `tab` `choice` `surface` `plain`; `shape`: `rounded` `card` `panel` `flush`; `active`). With `size="bare"`, put the row's own layout and padding on a child element |
| Navigation (settings rail, a page list) | `RowButton tone="nav" active={current}` |
| A selectable option card | `RowButton tone="choice" role="radio" aria-checked={chosen}` |
| Two to four visible choices | `Segmented`, or `SegmentedGroup` + `SegmentedItem`; `tabs`/`tab` for a section switcher |
| One choice from a short list with descriptions | `RadioGroup` + `Radio` |
| A hint on hover or focus | `Tip`, or `tooltip=` on `Button` |
| A text field | `Input` (`size="sm"` for dense rows, `mono` for paths and ids, `unstyled` inside a header or composer), `Textarea`, `InputGroup` for a field with an adornment |
| On or off | `Switch` |
| On or off, with a label | `Checkbox` |
| A choice from a list | `SelectField` (value, options, onChange); compose `Select` parts only when an option needs more than a label |
| A menu | `MenuPanel`, `MenuItem` |
| A floating panel | `Popover` |
| A dialog | `Dialog` and its parts |
| A status label | `Badge` (`muted` for secondary facts, `dashed` for estimates) |
| A notice | `Alert` |
| A framed block | `Card` |
| A key hint | `Kbd` |
| Empty, loading or failed pane | `PaneState`, `Empty`, `Skeleton*` (`SkeletonRows density`: `default` `menu` `flush` `card`) |
| A page heading | `ScreenHeading`, `SCREEN_CONTENT` |
| A settings page | `src/components/settings/kit.tsx`, which composes the primitives above |

## Tokens worth knowing

Letter spacing: `tracking-snug` for UI text, `tracking-tight`, `tracking-display` for large figures. Motion: `animate-meter-fill`, `animate-chart-rise`, `animate-insight-rise`, `animate-thinking-pulse`. Shadows: `shadow-control`, `shadow-float`. All live in `src/index.css`.

## Testing hints and selects

`Tip` mirrors a string label to `data-tip` on its child, so a test finds a hint with `[data-tip="…"]` instead of `title`. `SelectItem` mirrors its value to `data-value`; drive a select from jsdom with `chooseOption`, `openSelect` and `selectValue` from `src/testing/select.ts`.

## When nothing fits

Add it to `src/components/ui/` first, in its own commit:

- a new **variant or size** when an existing primitive is nearly right;
- a new **primitive** when the thing has its own states and will appear on
  more than one screen. Pull it from the coss registry when one exists there:
  `bunx shadcn@latest add @coss/<name>`.

Then use it from feature code. Do not work around a missing primitive with a
hand-styled element: that is how the same control ended up drawn five ways.

## The ratchet

Every rule is an error. The violations that existed when a rule was adopted
are recorded per rule and file in `lint-baseline.json`, and that count can
only go down.

```bash
bun run lint           # fails if any file has more violations than its baseline
bun run lint:update    # after you remove violations, lock the lower counts in
node scripts/lint-ratchet.mjs --list bridge/no-raw-controls   # see what is left
```

A new file starts at zero. Do not raise a baseline count; fix the code, or add
the primitive you were missing.

## Exceptions

A raw element is right when no control look is involved: a hidden or file
`<input>`, the composer's editing surface, an editor or terminal host. Say why
on the line:

```tsx
{/* oxlint-disable-next-line bridge/no-raw-controls -- CodeMirror owns this textarea */}
```

A disable comment without a reason does not pass review.
