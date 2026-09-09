# fix/menu-bar-meter — Test Contract

The CodexBar port landed as an in-app modal with a decorative tray. This branch
makes it an actual menu-bar meter: a panel that opens at the menu bar, reads
limits without a live chat, and shows the number in the menu bar itself.

Locked before implementation. Four defects, each with a verified root cause.

## Root causes (verified against the tree at `main`, not assumed)

**D1 — `window.show not allowed`.** `src/api.ts:1196` `revealMainWindow()` calls
`getCurrentWindow().show()`, which is the `plugin:window|show` IPC command.
`src-tauri/capabilities/default.json` grants `core:default`,
`core:window:allow-start-dragging`, `core:webview:allow-set-webview-zoom`,
`dialog:allow-open`, `shell:allow-open` — no `core:window:allow-show`. Tauri
rejects at `tauri-2.11.5/src/ipc/authority.rs:432`, producing verbatim
`window.show not allowed. Permissions associated with this command:
core:window:allow-show`. `App.tsx:366` routes that rejection into `setError`,
which is the "Something went wrong" banner. `unminimize` and `set_focus` are
missing the same way and would fail next.

**D2 — the panel opens inside the app.** The same tray click runs `openMeter()`.
`App.tsx` renders `MeterPopover` inside
`fixed inset-0 z-50 grid place-items-center bg-background/60` — a centred modal
over the main window, with a scrim. There is no second window anywhere in the
shell; `tauri.conf.json` declares exactly one. So clicking a menu-bar icon
raises the whole app and dims it behind a dialog.

**D3 — usage waits for a chat.** `sessions.rs:1100 request_codex_usage` selects
`sessions WHERE harness='codex' AND ended_at IS NULL` and calls `read_usage()`
on a live adapter runtime. With no running Codex chat the loop body never
executes and no `account-usage` frame is ever published. Claude is already
cold-safe (`sessions.rs:1129`, a global CLI probe). Codex is not, and Cursor and
OpenCode are never queried at all.

**D4 — the port stops at the math.** `meter.rs` ports `UsagePace.weekly` and the
adaptive-refresh table. The menu-bar surface itself is not ported: one generic
tray icon, no percentage in the menu bar, a three-item menu, and a popover whose
provider tiles are unreadable at panel width.

## Non-goals (stated, not silently dropped)

- **Cursor and OpenCode cold-start limits.** `~/.cursor` holds `chats/store.db`
  and `ai-tracking/ai-code-tracking.db`; neither records quota windows, which
  `usage_import/cursor.rs` already documents. Real Cursor limits need an
  authenticated `cursor.com` call. Out of scope; both stay in the registry as
  planned providers with an honest reason string.
- **`LSUIElement` / dock-less operation.** Bridge is a windowed app. The tray is
  a companion, not a replacement.
- **`NSPanel` adoption.** A borderless always-on-top window gets the behaviour
  without a new native dependency. Revisit only if it proves insufficient.

## Functional Behavior

### F1 — Tray click opens a menu-bar panel, not the app
- Left-click on the tray icon toggles a `meter` window: borderless, not
  resizable, always on top, absent from the taskbar/app switcher.
- The panel is positioned from the tray icon's own rect: horizontally centred
  under the icon, clamped so it never leaves the monitor's visible frame.
- Left-click while open hides it. The panel also hides when it loses focus and
  when the main window gains focus.
- **Dismissal is not universal, and that is a deliberate trade.** Showing the
  panel must not activate Bridge, so it is never focused; an unfocused window
  receives no blur event, and a click on another app or the desktop therefore
  leaves it up. Tray toggle, the close control, and returning to Bridge all
  dismiss it. Making an outside click dismiss it too requires either an
  `NSPanel` or a global event monitor, both deferred (see Non-goals).
- The main window is **not** shown, raised, or focused by a tray click.
- No error banner appears on any tray interaction.

### F2 — Window permissions are granted, and stay granted
- `core:window:allow-show`, `allow-hide`, `allow-unminimize`, `allow-set-focus`
  are granted to `main`.
- The `meter` window has its own capability covering what it calls.
- A test fails if frontend code calls a `@tauri-apps/api/window` method whose
  matching `core:window:allow-*` permission is not in a capability file. This is
  the durable fix: D1 was a silent gap, and must not be able to reappear.

### F3 — Limits are readable with nothing running
- On a cold start with no Codex chat, the meter shows Codex's `primary` (5h) and
  `secondary` (weekly) windows, read from the newest rollout under
  `~/.codex/sessions/**/rollout-*.jsonl`.
- The reader takes the **last** `rate_limits` payload in the newest file that has
  one, and publishes it on the existing `account-usage` channel in the **same
  shape the live adapter publishes**, so no frontend parsing changes.
- **Expired windows are dropped, not shown.** `resets_at` is absolute Unix
  seconds; a rollout from last week carries a `used_percent` that has since
  reset. A window whose `resets_at` is at or before now is omitted. If every
  window is expired, nothing is published — a stale number is worse than none.
- A live Codex session still wins: the disk read only fills the gap.

### F4 — The menu bar shows the number
- The tray title shows the worst live window as a percentage (`50%`), which is
  CodexBar's defining behaviour — the number lives in the menu bar.
- The title clears when no live window is known.
- The tray menu offers: open the meter, refresh, show Bridge, quit.

### F5 — The panel reads correctly at panel width
- One section per provider with its mark, label, and plan.
- Per window: name, percent, bar, reset countdown, and the pace line, laid out so
  they do not collide at ~360px.
- Empty state distinguishes "no provider connected" from "connected, no windows".
- Refresh control shows in-flight state and cannot be double-fired.

## Unit Tests

**Rust — `bridge-core`**
- `meter::codex_rate_limits_from_rollout_reads_newest` — fixture tree of three
  rollouts; returns the newest file's payload.
- `meter::codex_rate_limits_takes_last_payload_in_file` — file with two
  `token_count` events; returns the later one.
- `meter::codex_rate_limits_drops_expired_windows` — `primary.resets_at` in the
  past, `secondary` in the future → only `secondary` survives.
- `meter::codex_rate_limits_none_when_all_windows_expired` — returns `None`.
- `meter::codex_rate_limits_none_without_rate_limits` — rollouts with
  `token_count` but no `rate_limits` → `None`.
- `meter::codex_rate_limits_ignores_malformed_lines` — truncated/invalid JSON
  lines are skipped, not fatal.
- `meter::codex_rate_limits_none_when_sessions_dir_missing` — `None`, no panic.
- `meter::tray_title_uses_worst_window` — 8% weekly + 50% 5h → `50%`.
- `meter::tray_title_empty_without_windows` — empty string.

**Rust — shell**
- `capabilities::meter_window_has_capability` — a capability exists whose
  `windows` list covers `meter`.
- `capabilities::main_window_grants_show_hide_focus` — the four permissions in F2
  are present for `main`.

**Frontend — Vitest**
- `capabilities.test.ts` — scans `src/**/*.{ts,tsx}` for
  `@tauri-apps/api/window` method calls, maps each to its `core:window:allow-*`
  permission, asserts each is granted by a capability file. Fails on D1 exactly.
- `meterPanel.test.tsx` — renders the panel standalone: provider sections, empty
  states, refresh disabled while in flight.
- Existing `src/meter.test.ts` pace/adaptive cases keep passing unchanged.

## Integration / Functional Tests

- `refresh_account_usage` with **no** Codex session and a seeded fake
  `CODEX_HOME` publishes exactly one `AccountUsage{provider:"codex"}` event whose
  `rate_limits` matches the fixture's surviving windows.
- `refresh_account_usage` with a live Codex session does **not** double-publish
  from disk.
- Claude's existing cold probe is unaffected — its event still fires.
- `bun run check` clean: `tsc -b` + `cargo check --workspace`.
- Protocol artifacts regenerate to a no-op diff
  (`cargo test -p bridge-protocol`), or the generated files are committed
  alongside if the registry gained a method.

## Smoke Tests

1. `bun run test` — sidecar + vitest + cargo, all green.
2. `bun run build` — clean.
3. Release bundle launches (per the known dev-build caveat, `tauri dev` restarts
   itself and is not representative).

## E2E / Manual Tests

Run against a release bundle copied out of `~/Documents` (unsigned bundles are
killed by TCC when launched from there).

1. **D1** — quit every Codex chat, click the tray icon. Expect: panel opens at
   the menu bar; **no** "Something went wrong" banner; main window not raised.
2. **D2** — with the main window minimised, click the tray icon. Expect: the
   panel appears at the menu bar and the main window stays minimised.
3. **D3** — with zero Codex chats ever started in this Bridge install, open the
   panel. Expect: Codex 5h and weekly windows populated. Cross-check against the
   on-disk truth:
   ```bash
   find ~/.codex/sessions -name '*.jsonl' | xargs ls -t | head -1 \
     | xargs grep -h rate_limits | tail -1 | python3 -m json.tool
   ```
   The percentages must match, and any window whose `resets_at` is in the past
   must be absent from the panel.
4. **D4** — confirm the menu bar shows a percentage matching the worst window.
5. Click into the Bridge main window. Expect: the panel hides. Then reopen it
   and click another app instead — it stays up, which is the documented limit
   of dismissal without an `NSPanel`, not a regression.
6. Toggle rapidly ten times. Expect: no duplicate windows, no stuck state.

## Regression guard

`git grep -n "revealMainWindow"` must return no tray-path caller: raising the
main window is no longer part of opening the meter.
