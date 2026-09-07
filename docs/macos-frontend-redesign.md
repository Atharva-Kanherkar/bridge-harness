# macOS frontend redesign

Implemented September 6, 2026, using [the macOS HIG reference](apple-macos-hig-reference.md).

This records the initial foundation pass. The subsequent [screen-by-screen component audit](macos-screen-audit.md) covers the full frontend, additional interaction fixes, and final verification scope.

## Design decisions

- **Separate navigation from content.** Paper and Graphite have distinct sidebar, canvas, card, and popover surfaces. The optional native vibrancy treatment applies to the sidebar; the conversation canvas stays opaque.
- **Keep the task visible.** The title bar names the current screen. Conversation toolbars show the conversation title and project, including when the sidebar is hidden.
- **Use a restrained desktop hierarchy.** Geist remains the body font, Bricolage Grotesque supplies headings, and Geist Mono supplies code. Shared controls use compact rounded rectangles; selection, focus, and status use semantic tokens. Settings now uses the same Lucide icons as the rest of the app.
- **Make starting work direct.** The welcome screen groups its heading, composer, keyboard hints, and project choices. Choosing a project preserves the draft. Composer context controls wrap when space is limited.
- **Keep related screens consistent.** Projects, Marketplace, Memory, and Settings share page spacing and heading sizes. Memory remains one screen, with its existing About me, Review queue, and Activity views.
- **Make navigation accessible.** Sidebar status includes words as well as color. Project actions remain visible. The sidebar divider supports arrow keys in 16px steps and Home/End for its limits. A closed narrow-window sidebar is excluded from keyboard navigation and the accessibility tree.
- **Adapt to narrower windows.** Settings switches from its section rail to a section picker. Dialogs retain a centered desktop presentation. Reduced-motion behavior remains in place; custom materials also respond to reduced-transparency and increased-contrast media preferences.

All styling remains in Tailwind v4 utilities and `src/index.css`. No additional stylesheet or styling framework was introduced. Conversation grouping, streaming, approvals, send/stop/queue behavior, and persistence retain their existing implementation.

## Verification

Browser checks used the existing mock backend and covered the welcome screen, Projects, Marketplace, Memory and Activity, Settings, and a conversation. Both appearances were inspected. Narrow-layout checks covered the Settings section picker, hidden navigation, retained composer drafts during project selection, and wrapped project/branch controls without document overflow.

`bun run build` passed. `bun run test` passed: 1,797 frontend tests, 2,110 Rust tests, and 38 sidecar tests; the suites reported 14 skipped/ignored tests in total. Targeted sidebar and composer checks also passed after the final responsive adjustments. The production build retains Vite's large-chunk advisory.

The browser's existing zoom produced approximately 382 CSS pixels of content at the narrow viewport setting. This is narrower than the native window's configured 420px minimum. Native window activation, traffic-light placement, wallpaper vibrancy, OS accessibility preferences, and VoiceOver still require a check in the packaged Tauri app. CSS materials do not implement AppKit Liquid Glass.
