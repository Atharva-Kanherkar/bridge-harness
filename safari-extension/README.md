# Bridge Safari Web Extension

This is the Phase 3 Safari counterpart to `browser-extension/`. It uses the same semantic page protocol and tab-scoped lease model, while handing native messages to `SafariWebExtensionHandler.swift`, which relays them to Bridge's local supervisor socket.

Build the Xcode wrapper on macOS with a full Xcode installation (Command Line Tools alone do not include Apple's converter):

```bash
bun run prepare:safari-extension
```

The script uses Apple's `safari-web-extension-converter` and writes generated Xcode output outside the source tree by default. The user must enable the extension in Safari and explicitly attach a tab. Safari does not expose Chrome's `debugger` or `tabCapture` APIs, so network/console inspection and continuous capture remain Chrome-only; semantic control, screenshots, approvals, lease expiry, domain boundaries, and takeover use the shared Bridge protocol.
