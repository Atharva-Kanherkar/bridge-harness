# Browser workspace

The Browser dock contains page tabs. Open it from a task with the dock's Browser button (⌥⌘4). Each task owns its tabs and address history independently. Switching tabs or collapsing the dock keeps the page alive; closing a tab destroys that page. Reopen restores its address metadata, not form values.

The desktop uses a native child webview for each page. On macOS the navigation controls use WKWebView history and reload the actual current page, including pages reached through links. Stored address history supplies navigation after an app restart. Page tabs can be created, closed, selected with arrow keys, and reopened. Within the browser, ⌘L focuses the address, ⌘T opens a tab, ⌘W closes one, and ⇧⌘T reopens the most recently closed tab.

Bare localhost, IPv4 loopback, and bracketed IPv6 loopback addresses default to HTTP; public hostnames default to HTTPS. Explicit HTTP(S) addresses are supported. Executable and local-file URL schemes are rejected. New-window links open a page tab rather than replacing an unrelated page. Browser downloads are currently declined; use the system browser for downloads.

A slow page stays alive. After a delay the chrome offers continued waiting, stopping, or external-open without claiming that a site forbids embedding. An unfinished native navigation that becomes idle is shown as a generic load failure with Retry and external-open. Native pages are not placed in iframes, so a site's framing policy does not itself prevent opening the page.

## Select an element and request an edit

1. Open the project preview in a browser tab and choose **Select page element**.
2. Hover to see an outline, then click the component. The click selects it without activating the underlying control. Escape cancels.
3. Review its page identity, selector, and bounded snippet. Add an annotation if useful. The selected rectangle and note form the markup attached to the prompt.
4. Choose **Attach to prompt**, then write the desired change in the composer and send it to the current task. The selection chip can be removed before sending.

The selection is context for locating source code; Bridge does not claim a DOM selector is a source-file location. The coding agent must verify the corresponding source before editing. Navigation, task changes, and closing the source tab invalidate context. Bridge checks the live page generation again at submission. Sending a command or a task-routing shortcut with a selection is refused with the draft preserved, so it cannot silently reach another task.

## Trust and persistence

Browser pages have no Tauri capabilities and cannot invoke ordinary Bridge commands, even when they use the same localhost origin as the dev app. Only the main webview can control the browser plugin. Page selection data is explicitly marked as untrusted context. Form contents and private descendants are excluded; context is bounded and sanitized again before attaching. The existing authenticated Browser Surface lease workflow is unchanged.

Only versioned task-scoped page metadata is persisted: tab identities, safe HTTP(S) URLs, titles, and bounded address history. Credentials, authorization URLs, page DOM, form values, screenshots, and selection context are not stored in browser metadata. Normal explicitly submitted prompt context follows the existing conversation persistence path.

Native pages sit above the React DOM, so their visibility follows the dock, active page, task, document visibility, and app dialogs. The selection review reserves its own surface and hides the page beneath it.

## Platform and preview limits

The macOS desktop has native history and loading-state integration. Other desktop platforms use the native page surface with stored address-history fallback where native history APIs are unavailable; verify platform-specific navigation before claiming parity. The standalone Vite web preview uses sandboxed iframes. Browser security limits cross-origin navigation visibility and element inspection there; use the desktop app for the complete workflow. Embedded subframes and closed shadow roots are not source-component mappings.
