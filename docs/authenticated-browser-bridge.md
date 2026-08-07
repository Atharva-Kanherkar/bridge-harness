# Authenticated browser bridge

Bridge can supervise one user-approved tab from the user's normal logged-in browser. The browser remains the credential boundary: Bridge does not copy profiles, export cookies, or persist a continuous recording.

## Chrome setup

1. Build or run Bridge. The Tauri build prepares and bundles `bridge-browser-host` automatically.
2. Open a task and choose **Browser** → **Register native host**.
3. Open `chrome://extensions`, enable Developer mode, and choose **Load unpacked**. Use the extension path shown by Bridge.
4. Choose **Find tabs**, attach one HTTP(S) tab, and grant **Interact** only when clicking or typing is needed.

The extension ID is pinned by `browser-extension/manifest.json`; native messaging accepts only that origin. The tab badge remains visible while a lease is active. A lease expires after 30 minutes, at task completion/manual detach, when the tab closes, or when navigation crosses the granted domain.

## Safety and recovery

- Read-only is the default. Click, type, scroll, and navigation are a separate capability.
- Submit, send, delete, purchase/payment, publish, and credential effects create a plain-language approval in the Browser Surface.
- Password, payment, and one-time-code fields cannot be filled by the extension. Use **Take over** or **Open tab** for protected browser/OS UI, passkeys, CAPTCHA, permission prompts, and native file pickers.
- DOM content is labeled `untrusted_web_content`; suspicious instruction-like page text pauses the surface.
- Sensitive rectangles are painted out before a screenshot is delivered to Bridge.
- Command IDs are retained in `chrome.storage.session`, so reconnecting returns a prior result instead of repeating a side effect.
- Network and console inspection is opt-in and attaches only to the leased Chrome tab.

## Browser routing

The policy order is:

1. structured MCP/API;
2. attached authenticated tab;
3. local headless browser;
4. optional remote browser;
5. screenshot-first computer use.

Automated tests, untrusted sites, isolated QA, and local parallel QA route to headless first. Geo/proxy requirements, unattended work, or remote concurrency route to the optional remote provider. The remote endpoint must be public HTTPS; credentials are read at request time from the configured environment variable and are never stored in Bridge's config.

## Safari

`safari-extension/` contains the Safari Web Extension source and native handler. Run `bun run prepare:safari-extension` to invoke Apple's converter and create the Xcode wrapper under `.generated/`. The Safari implementation shares semantic actions, leases, approvals, redaction, and takeover with Chrome. Chrome-only DevTools and tab-capture capabilities remain unavailable in Safari.

## Metrics and benchmarks

Bridge stores per-site aggregate success, failure, latency, DOM-token, screenshot, intervention, approval, and duplicate-side-effect counts in the app data directory. It does not store page text in these metrics.

`testing/fixtures/browser-bridge-benchmark-v1.json` defines the comparable evidence schema for attached tab, Playwright MCP, Browser Use, and computer use. `src/browserBenchmark.ts` rejects incomplete scenario/provider matrices and successful runs without verified evidence.
