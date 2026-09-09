#!/bin/sh
# Shared helpers. Source this file after setting project_root; never use set -x.

release_load_env() {
  release_env_file=${BRIDGE_RELEASE_ENV:-"$HOME/.bridge-release/env"}
  if [ -f "$release_env_file" ]; then
    # Plain NAME=value assignments must reach the signing subprocess too.
    set -a
    # shellcheck disable=SC1090
    . "$release_env_file"
    set +a
  fi
}

release_require_credentials() {
  if [ -n "${APPLE_API_KEY:-}${APPLE_API_KEY_PATH:-}${APPLE_API_ISSUER:-}" ]; then
    if [ -z "${APPLE_API_KEY:-}" ] || [ -z "${APPLE_API_KEY_PATH:-}" ]; then
      echo "release: API notarization requires APPLE_API_KEY and APPLE_API_KEY_PATH; APPLE_API_ISSUER is required for Team keys only." >&2
      return 1
    fi
    if [ ! -r "$APPLE_API_KEY_PATH" ]; then
      echo "release: APPLE_API_KEY_PATH must point to a readable .p8 file." >&2
      return 1
    fi
    release_auth=api
  elif [ -n "${APPLE_ID:-}${APPLE_PASSWORD:-}${APPLE_TEAM_ID:-}" ]; then
    if [ -z "${APPLE_ID:-}" ] || [ -z "${APPLE_PASSWORD:-}" ] || [ -z "${APPLE_TEAM_ID:-}" ]; then
      echo "release: Apple ID notarization requires APPLE_ID, APPLE_PASSWORD (app-specific password), and APPLE_TEAM_ID." >&2
      return 1
    fi
    release_auth=apple-id
  else
    echo "release: configure API key or Apple ID notarization credentials in the environment or ~/.bridge-release/env." >&2
    return 1
  fi
}

release_require_identity() {
  release_identities=$(security find-identity -v -p codesigning 2>/dev/null || true)
  if [ -z "${APPLE_SIGNING_IDENTITY:-}" ]; then
    APPLE_SIGNING_IDENTITY=$(printf '%s\n' "$release_identities" | awk -F'"' '/Developer ID Application:/ { print $2; exit }')
    export APPLE_SIGNING_IDENTITY
  fi
  case "${APPLE_SIGNING_IDENTITY:-}" in
    'Developer ID Application: '*) ;;
    *) echo "release: a Developer ID Application identity is required for public distribution." >&2; return 1 ;;
  esac
  if ! printf '%s\n' "$release_identities" | awk -F'"' '{print $2}' | grep -Fxq "$APPLE_SIGNING_IDENTITY"; then
    echo "release: the configured Developer ID Application identity is not available in the keychain with a private key." >&2
    return 1
  fi
}

release_notarize() {
  release_submission=$1
  release_result=$2
  if [ "$release_auth" = api ]; then
    set -- --key "$APPLE_API_KEY_PATH" --key-id "$APPLE_API_KEY"
    if [ -n "${APPLE_API_ISSUER:-}" ]; then
      set -- "$@" --issuer "$APPLE_API_ISSUER"
    fi
  else
    set -- --apple-id "$APPLE_ID" --password "$APPLE_PASSWORD" --team-id "$APPLE_TEAM_ID"
  fi
  if ! xcrun notarytool submit "$release_submission" "$@" --wait --timeout 30m --output-format json > "$release_result"; then
    cat "$release_result" >&2
    echo "release: notarization submission failed or timed out; nothing will be published." >&2
    return 1
  fi
  python3 - "$release_result" <<'PY'
import json, sys
with open(sys.argv[1]) as source:
    result = json.load(source)
if result.get("status") != "Accepted":
    raise SystemExit("release: Apple did not accept the submission (status: %s, id: %s)" % (result.get("status"), result.get("id")))
print("Notarization accepted: " + result["id"])
PY
}
