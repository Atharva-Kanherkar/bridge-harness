# feat/personal-app-drafts — Test Contract

## Functional Behavior

- Eligible top-level Claude chats advertise `apps_draft`. Other harnesses, workers and restricted launches cannot draft apps. Inputs are name, intent, server, reads, writes, view_html, optional app_id and sample_args.
- Validate the real connected server and every named tool on the chat's existing Claude query, without a model turn. Return only selected server names, connection status, tool names and safety annotations; never credentials or configuration.
- Store a validated, hashed draft bound to its creator chat. Optional sample arguments are an object, at most 32 KiB, included in the hash when present; absent samples preserve existing manifest hashes. Existing view/name/tool bounds still apply; drafts expire after 24 hours.
- Fetch and preview require matching session, draft id and hash. Recompute the stored manifest hash on every request. Consumed drafts reload the exact locked version; expiry, tampering, deletion and another chat refuse. Locked previews remain dry-run.
- Preview reads must be on the draft allowlist, exist in current connected inventory, pass Bridge's read vocabulary and lack unsafe annotations. Writes return a “would send” result without making an MCP call. Other tools refuse. Arguments are objects bounded to 64 KiB, results to 256 KiB; concurrent control requests are capped at four per chat and 64 globally and timeout after 30 seconds.
- Correlated control replies bind to the originating session, never enter transcript/model context, and never broaden the separate approved Outgoing call path.
- Render a standalone transcript card from a successful tool result, fetching canonical stored HTML. Show target, read/write access and preview-only status outside the frame. Lock uses exactly the displayed hash; widening requires an explicit native second confirmation listing changes. Revise fills the composer, never sends. History is read-only.
- Personal HTML runs under injected CSP in nested opaque-origin sandbox frames with a trusted relay. No external links, network, self-navigation, popups, forms, child frames or parent DOM access. Only the mounted frame's messages are accepted. Clamp reported height to 48–1200 px. Preserve existing visual behavior.
- A preview reads a server only where the chat's own access mode lets the model read it: Full yes; Shared and Isolated no; read-only only with network authority and a projected server. Inventory stays available.
- CSP cannot block WebRTC. Live reads run only once the host confirms WebRTC is off in the calling webview (the Mac app turns it off and reads it back). The view mounts only after that check. Web mode, other platforms and a failed check refuse reads; writes still preview.
- Saved-app routing, real writes and non-Claude providers remain later slices.

## Unit Tests

- Sidecar app-control: access-mode read policy per write mode, and refusal when no policy is given; disabled/restricted launches; selected-server sanitization; ambiguous aliases; nonexistent/disconnected tools; mutation vocabulary and unsafe annotations; payload/result limits; read request flags; errors contain no configuration; writes never accepted.
- Core control: reply correlation/session isolation, stale/unknown replies, concurrency and timeout cleanup.
- Personal drafts: legacy hash compatibility, sample bounds/hash, binding persistence, wrong owner/hash, expiry, tampering, consumed locked version reload, deletion, write stub and allowlist refusal.
- Wire round trips, method registrations, daemon and Tauri dispatch signatures stay consistent with generated protocol 1.39.
- Transcript recognition/grouping excludes unrelated tools and incomplete results. Card loading/error/locked/widening/revise/history states; the view waits for the isolation check, and reads refuse unless it succeeded. Frame CSP injection precedes arbitrary markup, tool routing and source checks.

## Integration / Functional Tests

- Authenticated tool route exposes drafting only to top-level Claude sessions. Invalid inventory cannot create a draft. Draft → preview read/write → exact-hash lock → reload remains dry-run.
- Run focused Rust/sidecar/Vitest suites and `bun run build`. Open the PR after focused validation, then use remote CI for the full build/test gate and push fixes, as requested by the user.

## Smoke Tests

- Browser smoke with a fresh Playwright profile in light and dark themes: card renders, preview initializes, samples arrive, read result returns, write displays would-send, Lock and Revise work. Capture screenshots.

## E2E Tests

- Real browser security probe attempts fetch, image/CSS loads, forms, nested frames and self-navigation. No attempted external request reaches the network. Trusted wrapper relay carries normal MCP Apps messages.
- WebRTC exfiltration (STUN to a local UDP listener, including via a child frame) in a WKWebView: packets arrive with `_peerConnectionEnabled` on and none once it is off, including frames created after it is switched off on a loaded page.
- A real authenticated Claude account is not required for automated checks; sidecar tests inject the SDK control sender. Record any account-dependent live testing not performed.

## Manual / cURL Tests

- In a Claude chat with an MCP connection ask for an app using one actual read and write. Confirm the card's access list, load live read data, try the write and see would-send, then Lock. Reload the chat and see the original locked preview. Revise fills the composer. Request another tool from the HTML and confirm refusal. This account-dependent check is documented for reviewers if unavailable locally.
