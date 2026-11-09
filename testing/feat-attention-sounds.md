# Contract: notification sounds, per-kind settings and quiet hours

Branch `feat/attention-sounds`, stacked on `feat/attention-toast-stack`.
Implements the last part of #909, slice 5 of #904. Locked before
implementation.

## What this changes

Every attention kind currently gets a toast and, in the background, a
banner, with no sound and no way to turn any of it off. This adds a short
sound, a switch per kind for each of the three, and quiet hours.

## Functional behavior

Settings (`src/attentionSettings.ts`), kept in this client's localStorage
like the other General preferences (`bridge.attention.settings`), read
through `useSyncExternalStore` so every window sees a change at once:

- Per kind (`finished`, `failed`, `needsYou`, `escalation`): `toast`,
  `banner`, `sound`.
- Defaults: toast and banner on for every kind. Sound on for `needsYou` and
  `escalation`, off for `failed` and `finished` (a sound for every finished
  turn is noise).
- Quiet hours: off by default; `start` and `end` as `HH:MM`, default 22:00 to
  08:00. A window that crosses midnight works. While quiet, no banner and no
  sound; toasts still show, since they are silent and in-app.
- A missing, partial or corrupt stored value falls back to the defaults
  field by field.

Policy: `decide(item, presence, settings, now)` applies the settings after
the existing rules (on-screen suppression stays first). It returns
`{ toast, banner, sound, durationMs }`, or `null` when nothing is left on.

Sound (`src/attentionSound.ts`): two short tones through the Web Audio API,
synthesized rather than bundled files, so no binary asset ships: a rising
pair for requests (`needsYou`, `escalation`), a single soft tone for
`failed` and `finished`. One shared `AudioContext`, created on first use. A
failure to play (no audio device, autoplay policy) is ignored.

Settings UI: Settings → General gets a "Notifications" row that opens a
page with:
- one row per kind with Toast, Banner and Sound switches, and a "Play"
  button that previews that kind's sound;
- a Quiet hours switch with start and end times.
The page and its rows are in the settings search index.

Out of scope: per-chat overrides (the sidebar's existing notice level is for
worker reports), server-side settings shared across clients (#910).

## Unit tests

- `attentionSettings.test.ts`: defaults; a write is read back and announced;
  corrupt and partial values fall back field by field; `isQuiet` inside,
  outside and across midnight, and never when off.
- `attentionPolicy.test.ts`: each switch turns its channel off; all three off
  returns `null`; quiet hours drop banner and sound but keep the toast;
  on-screen suppression still wins.
- `attentionSound.test.ts`: plays through an `AudioContext` stub with the
  pattern for the kind; a throwing context is swallowed.
- `NotificationsPage.test.tsx`: renders a row per kind with three switches;
  toggling writes the setting; the Play button plays that kind; the quiet
  hours switch and times write the setting.
- `sections.test.ts` / settings search: the new section maps to General and
  its rows are searchable.

## Integration / functional tests

- `App.attention.test.tsx`: with `finished` toasts off, a finished row shows
  no toast; with `needsYou` sound on, the held request plays one sound;
  during quiet hours a background row posts no banner.

## Smoke tests

- `bunx vitest run src/attention*.test.ts src/components/settings src/App.attention.test.tsx`
- `bun run check`, `bun run build`, `bun run test`

## E2E / Manual

Mock mode in a browser: the Notifications page, dark and light screenshots.
Turning a kind's toast off, then pushing a row of that kind, shows nothing.
