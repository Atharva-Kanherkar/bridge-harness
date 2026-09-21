# Third-party code

## Orca terminal integration

The split-tree and terminal snapshot helpers under src/terminal and sidecar/terminal-state/orca are adapted from https://github.com/stablyai/orca at revision f2d5711b2d32e9f11277cd63805c76b0b5f9ddf7. The terminal state service follows Orca's headless-emulator.ts, session-output-plane.ts, and terminal-history-session-writer.ts, adapted to Bridge's Rust PTY host.

MIT License

Copyright (c) 2026 Lovecast Inc.

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.

## CodexBar native Menu Bar reference and provider icons

The provider SVG marks embedded in `src-tauri/bridge-menu-bar/swift/ProviderIcon.swift`
come from CodexBar's `Sources/CodexBar/Resources/ProviderIcon-*.svg` at revision
928166f. Native overview summary and separate status-item presentation follow its
`OverviewSpendSummary`, `ProviderBrandIcon`, and `StatusItemController` patterns,
adapted to Bridge's shared Rust snapshots. See
[the included MIT license](docs/third-party/CodexBar-LICENSE.txt).
