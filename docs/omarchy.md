# Bridge on Omarchy

[Omarchy](https://omarchy.org) is an Arch + Hyprland desktop built for running
coding agents. On Omarchy, Bridge looks and behaves like the rest of the desktop:

- **Theme.** Bridge takes its colors, light or dark mode, and monospace font
  from the active Omarchy theme, and retints within a second when you switch
  themes. Settings > Appearance offers this as the **Omarchy** skin (the
  default there); pick **Solid** to go back to Bridge's own look.
- **Window.** Hyprland draws the border, gaps and rounding. Bridge drops its
  own decorations, and its windows carry the class `bridge`.
- **Bar.** A Quickshell widget shows how many agents are working, lit when any
  is waiting for you. Left click opens what is waiting (or Mission Control);
  right click opens the quick prompt.
- **Notifications.** An approval arrives as a notification with
  **Allow once**, **Decline** and **Open**. It is critical urgency, so it gets
  through do-not-disturb. Answering it resolves the approval exactly as the
  in-app card does; one already answered elsewhere reports "Handled already".
- **Menu and launcher.** `Super + Space`, then "Bridge": new chat, quick
  prompt, review approvals, Mission Control. The same actions are on Bridge's
  desktop entry, so any Linux launcher shows them.
- **Quick prompt.** `Super + Shift + Ctrl + B` floats a Bridge composer over
  whatever you are doing. Enter starts the chat in your last project and the
  prompt hides; Esc hides it. The session keeps running either way.
- **Shortcuts** read `Ctrl+…` instead of `⌘…`.
- **Read-only workers** (research, verification) run under Landlock, Linux's
  kernel sandbox: they can read your project but write only to their own
  output directory.

## Install

```sh
yay -S bridge-bin
bridge-deck --omarchy setup --hyprland
```

`--hyprland` also adds the quick-prompt window rule and binding. Leave it off
to manage Hyprland yourself. Setup is idempotent: run it again after an update
and nothing changes unless something needs to.

## What setup writes

Everything goes under `~/.config`, the place Omarchy's own rules reserve for
user changes. Nothing is written to `/usr/share/omarchy` or `/etc`.

| Path | What | On remove |
|---|---|---|
| `~/.config/omarchy/hooks/theme-set.d/bridge` | Retints a running Bridge when you switch themes | deleted |
| `~/.config/omarchy/extensions/omarchy-menu.jsonc` | A `bridge` submenu, between `>>> bridge` and `<<< bridge` comments; your own entries are kept | restored from the backup |
| `~/.config/omarchy/plugins/app.bridge.status/` | The bar widget (enabled in the right section) | deleted |
| `~/.config/hypr/bridge.lua` (`--hyprland`) | Quick-prompt window rule and `Super + Shift + Ctrl + B` | deleted |
| `~/.config/hypr/hyprland.lua` (`--hyprland`) | One `require("hypr.bridge")` line at the end | restored from the backup |
| `~/.config/bridge/omarchy-setup.json`, `~/.config/bridge/omarchy-backup/` | What setup created, and pristine copies of every file it changed | deleted |

## Remove

```sh
bridge-deck --omarchy remove
```

Every file setup changed is restored byte for byte from its backup, and every
file and directory it created is deleted (a directory you have since put your
own files in is left alone).

## Command line

The bar uses these; they are handy in scripts too. The CLI installs as
`bridge-cli` because `/usr/bin/bridge` is iproute2's network bridge tool.

```sh
bridge-cli status --json            # {"running":true,"working":2,"waiting":1,"sessions":[...]}
bridge-cli status --json --watch    # one line per change
bridge-cli sessions --json
bridge-cli approval allow-once <sessionId> <eventId>   # also: allow-session, decline

bridge-deck --open <sessionId>
bridge-deck --new-chat [--project <path>]
bridge-deck --quick
bridge-deck --approvals
bridge-deck --mission-control
```

`bridge-cli status` never starts Bridge: with the app closed it prints
`"running": false`. `bridge-cli approval` goes through the same resolution path as
the in-app approval card, so the policy engine stays the only authority.

## Troubleshooting

- **Blank or glitchy window on NVIDIA.** Bridge turns off WebKit's DMABUF
  renderer when it detects the NVIDIA driver under Wayland. If you still see
  it, run `WEBKIT_DISABLE_DMABUF_RENDERER=1 bridge-deck` and report your
  driver version.
- **An agent shows as unavailable on a fresh install.** Omarchy's `codex`,
  `claude`, `opencode` and `pi` launchers install the tool on first use, which
  can take a minute. Bridge waits for that first run instead of giving up;
  refresh Settings > Coding agents once it finishes.
- **Read-only workers refuse to start.** They need Landlock ABI 3 (Linux 6.2+),
  and ABI 4 (Linux 6.7+) with `BRIDGE_READ_ONLY_NETWORK=deny`. Current Arch
  kernels qualify. See [`linux-release.md`](linux-release.md).
- **No bar widget.** Run `omarchy plugin enable app.bridge.status`, or check
  that `bridge-cli status --json` prints a line.
- **The theme did not change.** The theme hook and Bridge's own watcher both
  read `~/.local/state/omarchy/current/theme`; `bridge-deck --omarchy retint`
  forces a refresh.

## Status

Built and tested against Omarchy's source (`basecamp/omarchy`, branch
`quattro`), with the Linux-only parts compiled and run on a Linux 6.12
kernel. It has not yet been run end to end on an Omarchy 4 install; until it
has, treat the Linux build as experimental.
