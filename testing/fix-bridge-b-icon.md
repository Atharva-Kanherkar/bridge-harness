# Bridge B icon refresh: test contract

## Functional Behavior
- Keep the original Bridge wordmark, Doto font, and all brand lettering unchanged.
- Use the connected, stepped B in the browser favicon, website icon, desktop app PNG/ICNS/ICO files, Windows tiles, and generated iOS/Android icons.
- Use the same B silhouette without its tile in the native menu bar, Open Bridge menu item, home brand symbol, and Bridge runtime glyph.
- Derive every representation from assets/bridge-icon.svg. Preserve native template behavior and theme-aware foreground color.

## Unit Tests
- scripts/test/icons.test.mjs: committed native exports contain visible monochrome artwork, ICNS pixels are checked, ICO frames are valid, and website/frontend/native representations match the canonical source.
- src/components/harnessMarks.test.tsx: Bridge uses the canonical B while retaining its existing neutral tint and routing semantics.
- Native menu-bar tests: the B has a continuous stem and two transparent counters; status and menu icons retain their template flags and point sizes.

## Integration / Functional Tests
- Run bun run generate:icons twice and confirm identical generated file hashes.
- Run bun run build and NODE_OPTIONS=--no-experimental-webstorage bun run test. The environment flag disables Node 26's global web storage so jsdom supplies test localStorage.
- Run the landing build and tests.

## Smoke Tests
- All production favicon and website icon URLs return successfully.
- The landing header/footer still render the original Doto Bridge lettering.

## E2E Tests
- N/A: no navigation, persistence, or provider behavior changes.

## Manual / cURL Tests
- Capture the real landing page and a labeled icon comparison showing the committed app exports and native template render.
- Review 16/32px favicon previews and the native menu-bar render for readable counters.
- Confirm the PR diff contains no unrelated feature commits or lettering changes.
