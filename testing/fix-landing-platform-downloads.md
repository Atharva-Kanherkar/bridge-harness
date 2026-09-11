# Landing platform downloads — Test Contract

## Functional Behavior
- The home and download pages offer separate macOS and Linux download buttons.
- Buttons resolve the current published stable release: Apple Silicon DMG for macOS, x86_64 AppImage for Linux.
- Missing assets, prereleases, drafts, malformed responses, or API failures fall back to the latest release page without inventing a download URL.
- Release links update without a site rebuild. No native app or release workflow changes.

## Unit Tests
- Test actual asset selection, architecture and checksum exclusions, missing platforms, untrusted asset URLs, draft/prerelease rejection, and API failure fallback.

## Integration / Functional Tests
- Landing build and lint pass. Root build and test commands are run as required by AGENTS.md.
- Local download endpoints redirect to the published macOS DMG and to the release page while Linux has no published asset.

## Smoke Tests
- Home and download pages return HTTP 200.

## E2E Tests
- Inspect desktop and mobile layouts for both platform buttons, usable links, and no horizontal overflow.

## Manual / cURL Tests
- curl -I http://127.0.0.1:1432/download/macos
- curl -I http://127.0.0.1:1432/download/linux
- Compare redirect destinations with gh release view --json assets,url.
