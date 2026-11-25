# Test contract: one simple thinking-effort control

Branch `feat/simple-effort-picker`. Supersedes `feat-effort-selector-styles.md`.

## Behaviour

1. The model picker draws one effort control: a segmented radiogroup
   (`role="radiogroup"`, `aria-label="Reasoning effort"`), one `role="radio"`
   button per level the selected model lists, labelled with the level name and
   carrying `data-effort`.
2. One click on a segment sends that level. Arrow keys, Home and End move and
   select. Enter in the Thinking row keeps the level and closes the picker.
3. The current level is the only `aria-checked="true"` segment. With no level
   set (the provider's default) none is checked and the caption says so.
4. The caption on the right of the "Thinking" label names what the current
   level means.
5. Everything the old styles did around the control still holds: queue a level
   while the host switches models, keep a sent level until confirmed, revert
   when the host settles without it, stay inert without a setter, hide for a
   model with no ladder, Codex's six levels fit the 380px popover.
6. No Settings choice of style: Appearance has no "Thinking control" group, the
   settings search has no entry for it, and `settings_change` has no
   `appearance.effortSelector` key.
7. A stored preferences record or an older client that still sends
   `effortSelector` parses; the key is dropped and never written back, so
   theme and skin are not reset.

## Checks

- `bunx vitest run src/components/ChatModelControl.test.tsx src/components/ChatModelControl.columns.test.tsx src/components/SettingsScreen.test.tsx src/theme.test.ts src/appPreferences.test.ts`
- `cargo test -p bridge-protocol` (schema drift + retired key test)
- `cargo test -p bridge-core app_preferences settings_tools`
- `bun run build`, `bun run test`
- Screenshots: picker in dark and light, a level picked, Codex six levels.
