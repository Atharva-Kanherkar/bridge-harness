#!/bin/sh
# patchelf for linuxdeploy, minus the Claude Agent SDK's native binary.
#
# linuxdeploy sets the RPATH of every ELF file in the AppDir with the patchelf
# it bundles (0.8). That rewrite corrupts the SDK's `claude` executable (SDK
# 0.3.322 and later): `ldd` then exits 1 on it, linuxdeploy's GTK plugin pass
# aborts, and no AppImage is built. Had it built, Claude would not have run.
# The binary links only glibc, so it needs no RPATH: leave it byte-for-byte as
# the SDK shipped it, and hand every other call to the patchelf linuxdeploy
# would have used. `scripts/tauri.sh` points PATCHELF here for Linux builds.
set -eu

modifies=false
for arg in "$@"; do
  case "$arg" in
    --set-rpath|--add-rpath|--remove-rpath|--shrink-rpath|--force-rpath|--set-interpreter|--set-soname|--add-needed|--remove-needed|--replace-needed|--no-default-lib|--clear-symbol-version)
      modifies=true ;;
  esac
done

if [ "$modifies" = true ]; then
  for arg in "$@"; do
    case "$arg" in
      */claude-agent-sdk-*/claude) exit 0 ;;
    esac
  done
fi

exec "${BRIDGE_REAL_PATCHELF:-patchelf}" "$@"
