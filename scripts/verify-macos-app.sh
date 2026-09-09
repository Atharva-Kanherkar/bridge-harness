#!/bin/sh
# Strict public-release gate. This intentionally rejects local ad-hoc builds.
set -eu
project_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
app=${1:-}
if [ -z "$app" ] || [ ! -d "$app/Contents" ]; then
  echo "verify-macos-app: pass the exact Bridge.app path." >&2
  exit 1
fi
verify_tmp=$(mktemp -d "${TMPDIR:-/tmp}/bridge-verify.XXXXXX")
trap 'rm -rf "$verify_tmp"' EXIT HUP INT TERM
codesign --verify --deep --strict --verbose=2 "$app"
codesign -dv --verbose=4 "$app" 2> "$verify_tmp/signature.txt"
codesign -d --entitlements - --xml "$app" > "$verify_tmp/entitlements.plist" 2>/dev/null
python3 - "$app" "$project_root" "$verify_tmp" <<'PY'
import json, os, pathlib, platform, plistlib, re, subprocess, sys
app, root, scratch = map(pathlib.Path, sys.argv[1:])
def verify_signature(signature, label):
    if not re.search(r"^Authority=Developer ID Application:", signature, re.M):
        raise SystemExit("verify-macos-app: missing Developer ID Application signature: " + label)
    team = re.search(r"^TeamIdentifier=(.+)$", signature, re.M)
    if not team or team[1].strip() in ("", "not set"):
        raise SystemExit("verify-macos-app: missing signing TeamIdentifier: " + label)
    if not re.search(r"flags=.*\bruntime\b", signature):
        raise SystemExit("verify-macos-app: Hardened Runtime is not enabled: " + label)

verify_signature((scratch / "signature.txt").read_text(), "Bridge.app")
with (scratch / "entitlements.plist").open("rb") as source:
    entitlements = plistlib.load(source)
with (root / "src-tauri/entitlements.plist").open("rb") as source:
    required = plistlib.load(source)
for key, value in required.items():
    if type(entitlements.get(key)) is not type(value) or entitlements.get(key) != value:
        raise SystemExit("verify-macos-app: missing or incorrect entitlement: " + key)
with (app / "Contents/Info.plist").open("rb") as source:
    info = plistlib.load(source)
config = json.loads((root / "src-tauri/tauri.conf.json").read_text())
if info.get("CFBundleShortVersionString") != config["version"]:
    raise SystemExit("verify-macos-app: bundle version does not match this checkout")
if info.get("CFBundleIdentifier") != config["identifier"]:
    raise SystemExit("verify-macos-app: bundle identifier does not match this checkout")
icon = info.get("CFBundleIconFile", "")
if not icon:
    raise SystemExit("verify-macos-app: no CFBundleIconFile")
if not icon.endswith(".icns"):
    icon += ".icns"
subprocess.run(["node", str(root / "scripts/verify-icons.mjs"), str(app / "Contents/Resources" / icon)], check=True)
sidecar = app / "Contents/Resources/sidecar/claude-agent"
for name in ("index.mjs", "briefing.mjs", "input.mjs", "options.mjs", "read-only.mjs", "package.json", "package-lock.json"):
    if (sidecar / name).read_bytes() != (root / "sidecar/claude-agent" / name).read_bytes():
        raise SystemExit("verify-macos-app: stale or missing sidecar source: " + name)
sdk = sidecar / "node_modules/@anthropic-ai/claude-agent-sdk"
if sdk.is_symlink() or not (sdk / "package.json").is_file():
    raise SystemExit("verify-macos-app: bundled Claude SDK is missing or is a symlink")
lock = json.loads((sidecar / "package-lock.json").read_text())
actual = json.loads((sdk / "package.json").read_text())
if actual["version"] != lock["packages"]["node_modules/@anthropic-ai/claude-agent-sdk"]["version"]:
    raise SystemExit("verify-macos-app: bundled Claude SDK does not match its lockfile")

# The SDK's JS loader needs its optional native CLI package. npm considers an
# omitted optional dependency valid, so npm ls and sdk/package.json are not
# sufficient. Public DMGs currently target the release host architecture.
arch = {"arm64": "arm64", "aarch64": "arm64", "x86_64": "x64", "AMD64": "x64"}.get(platform.machine())
if not arch:
    raise SystemExit("verify-macos-app: unsupported release host architecture")
native_name = "node_modules/@anthropic-ai/claude-agent-sdk-darwin-" + arch
native_dir = sidecar / native_name
native = native_dir / "claude"
if native_dir.is_symlink() or native.is_symlink() or not native.is_file() or not os.access(native, os.X_OK):
    raise SystemExit("verify-macos-app: missing real executable Claude SDK binary for darwin-" + arch)
native_metadata = json.loads((native_dir / "package.json").read_text())
if native_metadata["version"] != actual["version"] or native_metadata["version"] != lock["packages"][native_name]["version"]:
    raise SystemExit("verify-macos-app: native Claude SDK version does not match its lockfile")

# Tauri signs externalBin entries, but keeps arbitrary Resources binaries as
# supplied. Preserve Anthropic's upstream signature and entitlements. Its Team
# ID legitimately differs from the enclosing Bridge app's signing team.
subprocess.run(["codesign", "--verify", "--strict", str(native)], check=True)
native_signature = subprocess.run(["codesign", "-dv", "--verbose=4", str(native)], check=True, capture_output=True, text=True)
verify_signature(native_signature.stderr, "Claude SDK native binary")
native_entitlements = subprocess.run(["codesign", "-d", "--entitlements", "-", "--xml", str(native)], check=True, capture_output=True)
native_grants = plistlib.loads(native_entitlements.stdout)
for key in ("com.apple.security.cs.allow-jit", "com.apple.security.cs.allow-unsigned-executable-memory", "com.apple.security.cs.disable-library-validation"):
    if native_grants.get(key) is not True:
        raise SystemExit("verify-macos-app: native Claude SDK is missing entitlement: " + key)
for name in ("bridged", "bridge-browser-host"):
    binary = app / "Contents/MacOS" / name
    if not binary.is_file() or not os.access(binary, os.X_OK):
        raise SystemExit("verify-macos-app: missing executable sidecar: " + name)
print("Verified app version, signature, Hardened Runtime, entitlements, icon, and sidecars.")
PY
