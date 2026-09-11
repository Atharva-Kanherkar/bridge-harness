# Linux release candidates

The `Linux release candidates` workflow builds x86_64 Debian and AppImage artifacts on Ubuntu 22.04. A second job verifies their SHA-256 checksums and repackages the Debian payload with an Arch PKGBUILD. Artifacts are retained on the workflow run; this workflow neither publishes a release nor uploads to AUR.

The Linux Tauri overlay selects Linux bundles without changing the macOS signing configuration. Both native helpers and the Claude and terminal-state sidecars are staged by the existing build hook. Node.js and Git are runtime requirements; Debian and Arch declare them as package dependencies. AppImage users must install Node.js 18+ and Git separately. Systems without FUSE can extract an AppImage with `--appimage-extract` and run `squashfs-root/AppRun`.

## Build

On Ubuntu 22.04 or later, install Rust, Bun, Node.js, Git, and the dependencies listed in `.github/workflows/release-linux.yml`, then run:

```sh
bun install --frozen-lockfile
bun run build
bun run test
bun run tauri build --config src-tauri/tauri.linux.conf.json --ci
```

The Arch recipe is a CI packaging template. CI adds the exact local artifact name and checksums; it is not a published AUR package.

## Known blocker: read-only workers

Linux and Arch are **experimental candidates**. OS-enforced read-only research and verification workers currently require macOS Seatbelt. Linux rejects these launches rather than running them without isolation. A Linux sandbox implementation and native isolation tests are required before promoting these candidates to stable. Ordinary chat and write-capable workers do not use this boundary.

## Before calling a Linux release stable

Install the candidate on Ubuntu and current Arch, under both X11 and Wayland. Verify first-run setup, provider login/browser handoff, workspace creation, text and image turns, approval handling, terminal rendering, restart/history, and uninstall. Build success alone does not establish these behaviors. Record the tested distribution and session type in release notes.

After the PR is reviewed and merged, publish only the exact tested artifacts with their checksums. The signed/notarized macOS DMG is produced separately by `Release macOS DMG`; a manual branch run retains an artifact without publishing it.
