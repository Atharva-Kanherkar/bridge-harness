# Authenticated browser bridge

Bridge can supervise one user-approved tab from the user's normal logged-in browser. The browser remains the credential boundary: Bridge does not copy profiles, export cookies, or persist a continuous recording.

## Chrome setup

1. Build or run Bridge. The Tauri build prepares and bundles `bridge-browser-host` automatically.
2. Open a task and choose **Browser** → **Register native host**.
3. Open `chrome://extensions`, enable Developer mode, and choose **Load unpacked**. Use the extension path shown by Bridge.
4. Choose **Find tabs**, attach one HTTP(S) tab, and grant **Interact** only when clicking or typing is needed.

The extension ID is pinned by `browser-extension/manifest.json`; native messaging accepts only that origin. The tab badge remains visible while a lease is active. A lease expires after 30 minutes, at task completion/manual detach, when the tab closes, or when navigation crosses the granted domain.

## Agent access

When a tab lease is active and its first safe snapshot is ready, Bridge injects a hidden application-owned capability into Codex, Claude, and OpenCode turns. The capability points to a local Unix-socket tool bound to both the Bridge session and the current lease. Agents can inspect the attached title, domain, viewport, and a bounded redacted semantic snapshot, then request click, type, scroll, same-domain navigation, focus, fresh screenshot, or user takeover operations. Tool calls still pass through `BrowserBridgeSupervisor`; providers never talk to the extension directly. Agent clicks always wait for user approval.

The capability is not advertised when no usable attached tab exists. Its token is also bound to the owning adapter process tree, so another agent session cannot reuse a discovered wrapper. Task stop/clear, detach, lease expiry, cross-domain navigation, extension disconnect, or user takeover invalidates it immediately. DOM changes mark native state unreadable until the matching lease and redaction generation produce a fresh snapshot. Provider command calls and structured tool results flow through the adapters' existing normalized event stream, so browser activity remains visible in the conversation timeline.

## Safety and recovery

- Read-only is the default. Click, type, scroll, and navigation are a separate capability.
- Submit, send, delete, purchase/payment, publish, and credential effects create a plain-language approval in the Browser Surface.
- Password, payment, and one-time-code fields cannot be filled by the extension. Use **Take over** or **Open tab** for protected browser/OS UI, passkeys, CAPTCHA, permission prompts, and native file pickers.
- DOM content is labeled `untrusted_web_content`; suspicious instruction-like page text pauses the surface.
- Sensitive rectangles are painted out before a screenshot is delivered to Bridge.
- Command IDs are retained in `chrome.storage.session`, so reconnecting returns a prior result instead of repeating a side effect.
- Network and console inspection is opt-in and attaches only to the leased Chrome tab.

## Live mirror performance

The mirror encodes asynchronous WebP frames at up to 10 FPS and paints known sensitive rectangles before encoding. Redaction maps and frames carry matching epochs, so a frame encoded during a stale DOM generation is discarded. Only one native frame may be in flight; if rendering or transport falls behind, the extension drops stale frames and forwards the newest one after acknowledgement. Bridge exposes frames through a revisioned frame-only command, while semantic state, audit history, and lease status use a separate lightweight poll. Screenshot bytes remain ephemeral and are stripped from persisted audit events. This avoids background decode/re-encode work and repeated cloning of large base64 frames with every state refresh.

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
