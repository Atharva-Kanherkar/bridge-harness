# fix/menu-bar-meter (continued) — Usage meter, chart colour, Insights, access control

Test contract for the second half of the branch. Locked before implementation.

## Meter (menu bar)

- **M1** The card shows every quota window a provider reports. A window whose
  reset has passed is reported at 0% and marked *fresh window*, never dropped
  and never counted down to (`meter_sources::settle_reset_windows`;
  `sessions::publish_codex_usage_from_disk`).
- **M2** No signed pace delta (`+4%`) anywhere on the card. Pace is a phrase:
  *on pace*, *ahead of pace · runs out in 2d 3h*, *under pace · lasts to reset*
  (`meter.pacePhrase`).
- **M3** No planned-provider matrix and no attribution footer. The card is the
  limits and only the limits. *Open Bridge* moves to the header.
- **M4** One reading per provider: a ring gauge for the headline (worst)
  window. Bars wear the harness's chart colour; pace stays a phrase under the
  bar (the pace chart was tried and removed as noise at panel width).
- **M5** The tray shows the icon only. No percentage in the menu bar.

## Colour

- **C1** `--warning` is a warm grey in both appearances. Nothing in the app
  renders a yellow tint.
- **C2** Chart series colours are per-harness tokens (`--chart-codex`,
  `--chart-cursor`, `--chart-claude`, `--chart-opencode`), validated as one
  categorical palette in light and dark (lightness band, chroma floor, CVD
  and normal-vision separation ≥ 15, contrast ≥ 3:1). Colour follows the
  harness, never its rank; text never wears the series colour.

## Access mode

- **A1** The *Approvals bypassed* badge is gone from the chrome. The composer
  carries an access control with two modes: **Full access** (auto-approve) and
  **User approval** (ask first). Both composers (session and hero) show it.
- **A2** Changing the mode saves the permission policy and re-reads it; the
  control is disabled until the policy has loaded.

## Insights

- **I1** `usage/insights` params `{ windowDays, refresh }`; result carries
  `status` (`ready | unavailable | failed | empty`), the harness/model that
  wrote it, and a report. Params are strict (`deny_unknown_fields`).
- **I2** Every chart figure (harness totals, prompts by hour, tokens by day,
  GitHub PR counts) is computed by Bridge from its own ledgers. The model
  contributes prose only, returned as one fenced JSON object, bounded and
  normalised before storage. Theme shares are normalised to sum to 1.
- **I3** The run is a hidden `briefing`-kind session under a briefing policy
  with no servers in scope (no tools). Prompts are sampled (60) and truncated
  (280 chars) before they reach the model; the report stores none of them.
- **I4** Without `refresh`, the stored report is returned and no model runs.
  The tab loads the stored report; *Analyse* is the only action that runs one.
- **I5** Unavailable (no harness), failed, and empty states each say why and
  keep the retry where it makes sense.

## Usage page (follow-up)

- **U1** Local history is always included; the toggle and its scope line are
  gone, and a stored `includeImported: false` is ignored.
- **U2** The model-prices card shows the snapshot date and model count only;
  the rate-table URL lives in the refresh button's tooltip.
