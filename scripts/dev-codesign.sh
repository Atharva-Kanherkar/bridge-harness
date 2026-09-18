#!/bin/sh
# Give locally built dev binaries (e.g. the `bridged` daemon) one stable
# macOS code identity across rebuilds.
#
# Without this, `cargo build` produces an unsigned binary. macOS Keychain
# grants access to another app's item (e.g. the `Claude Code-credentials`
# entry the `claude` CLI owns) by trusting the *requesting binary's*
# designated requirement. An ad-hoc/unsigned binary has no certificate to
# anchor that requirement to, so it falls back to the binary's own content
# hash — which changes on every rebuild, so every rebuild looks like a new,
# untrusted app and the OS re-prompts (or silently denies a headless
# process), surfacing as the provider going "unavailable" after a rebuild
# that changed nothing about auth.
#
# Signing with a self-signed certificate fixes this: once macOS trusts that
# certificate for code signing, binaries signed with it keep the same
# designated requirement across rebuilds, so a Keychain grant survives.
set -eu

# Only macOS has this Keychain-ACL problem.
[ "$(uname -s)" = "Darwin" ] || exit 0
# Never touch a CI runner's keychain; release builds sign with the real
# Developer ID identity and overwrite whatever this leaves behind anyway.
[ -z "${CI:-}" ] || exit 0

IDENTITY_NAME="Bridge Local Dev"
KEYCHAIN="${HOME}/Library/Keychains/login.keychain-db"

ensure_identity() {
  security find-certificate -c "$IDENTITY_NAME" "$KEYCHAIN" >/dev/null 2>&1 && return 0

  echo "bridge (dev): creating a local code-signing identity ('$IDENTITY_NAME') so rebuilt dev binaries keep one stable macOS identity"
  work_dir=$(mktemp -d)
  trap 'rm -rf "$work_dir"' EXIT INT TERM

  openssl req -x509 -newkey rsa:2048 -keyout "$work_dir/key.pem" -out "$work_dir/cert.pem" \
    -days 3650 -nodes -subj "/CN=$IDENTITY_NAME" \
    -addext "keyUsage=critical,digitalSignature" \
    -addext "extendedKeyUsage=critical,codeSigning" \
    -addext "basicConstraints=critical,CA:FALSE" >/dev/null 2>&1

  openssl pkcs12 -export -out "$work_dir/cert.p12" \
    -inkey "$work_dir/key.pem" -in "$work_dir/cert.pem" -passout pass:bridge-local-dev

  # -T grants codesign access to the private key without a per-use prompt.
  security import "$work_dir/cert.p12" -k "$KEYCHAIN" -P bridge-local-dev -T /usr/bin/codesign

  # The one-time step: trust this certificate for code signing so the
  # Keychain ACL machinery recognizes a designated requirement anchored to
  # it, rather than falling back to a raw (and rebuild-unstable) hash.
  security add-trusted-cert -p codeSign -k "$KEYCHAIN" "$work_dir/cert.pem"
}

sign_binary() {
  binary="$1"
  ensure_identity
  codesign --force --sign "$IDENTITY_NAME" --identifier "dev.bridge.deck.dev" "$binary"
}

case "${1:-}" in
  sign)
    [ "${2:-}" ] || { echo "usage: dev-codesign.sh sign <binary>" >&2; exit 1; }
    sign_binary "$2"
    ;;
  *)
    ensure_identity
    ;;
esac
