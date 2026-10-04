# Bridge B icon evidence

- `icon-exports.png`: Chrome screenshot of an asset comparison. The macOS tile is the actual ICNS `ic09` PNG; Windows, iOS, Android, and ICO/SVG favicon previews use the committed exports. The menu-bar and Open Bridge previews are PNGs rendered from the production AppKit template icons by the native tests. This is an asset comparison, not an installed-app or Dock screenshot.
- `wordmark-preserved.png`: Chrome screenshot of the production landing build. The original Doto Bridge lettering is unchanged.

Native renders: `BRIDGE_ICON_RENDER_DIR=/tmp/bridge-b-icon-evidence bun run test:menu-bar`.
