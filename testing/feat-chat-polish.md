# feat/chat-polish: Test Contract

Chat polish: Thinking row, reply copy, logo activity timeline, Gitplace tab, and UX cleanup.
Presentation-only pass. No protocol or wire change. Graphite & Paper stays locked:
achromatic chrome, colour only for status, diffs and approvals.

## Scope

In this PR: themes 1 to 4 (Gitplace phase 1), and from theme 5 the card soup, toast
placement, placeholder chat names, nav cleanup, stale-base duplicate copy, verifying
card tone, composer fade, and the mock compaction string.

Deferred, named so they are not forgotten:
- Gitplace phase 2 (`github/inbox`, wire change, separate PR).
- A repo-wide Checks tab in Gitplace. Checks are read per pull request today
  (`githubChecks(workspaceId, number)`); there is no repo-level read to back a tab
  without a wire change. PR detail keeps its checks.
- Composer footer collapse into one `project · branch` chip.
- Usage view switcher reduction.
- Copy-whole-turn from the `Worked for` header.

## Functional Behavior

### 1. Thinking row
- `computeNarration` with no phase returns label `Thinking`.
- Phase labels carry no harness or model name: `spawning` → `Starting…`,
  `handshake` → `Connecting…`, `session_open` → `Opening session…`.
- `modelName` is removed from `NarrationInput`; no string `is reading your message`
  exists anywhere in `src/`.
- Elapsed counter appears only after 10s (`ELAPSED_VISIBLE_AFTER_MS = 10000`).
- `StartupStatusRow` renders the harness mark plus a `Thinking` word that pulses
  (`thinking-pulse` keyframe); under reduced motion the word is static.
- A reply with no first token renders the same row (mark + pulsing `Thinking`), not
  the shimmer bar.
- Streaming reasoning renders the same row with faint text beneath, no card border.
- Completed reasoning is a one-line `Thought for Ns` disclosure with no border.

### 2. Reply action bar
- Completed assistant replies get an icon-only bar: Copy · Remember · Fork · Rewind,
  28px buttons, each with `aria-label` and `title`.
- Copy writes `item.text` (Markdown) to the clipboard and shows a tick for 1.5s.
- The bar is always visible on the latest assistant reply, hover-revealed on older ones.
- Streaming replies show no bar.
- User bubbles get Copy too. Hidden hover actions take zero layout height (absolutely
  positioned, not an opacity-0 line in flow).

### 3. Activity timeline with tool logos
- `ActivityGroup` has no outer border box. Header reads `Worked for {duration}` when
  finished and timestamps exist, `Working for {duration}` or `Working` when live, and
  `Worked` fallback with no timestamps. No separate `Activity` title, no check mark.
  Verb summary stays as faint trailing text. Failures keep a destructive glyph and
  `needs attention` wording.
- Rows: no borders, no `divide-y`, icon then label.
- Success ticks per row are gone; only running (spinner) and failed (X) show a glyph.
- `ToolGlyph` gains `git`, `github`, `mcp`; `ToolCallDisplay` gains optional `server`.
  - Shell whose first token is `git` → `git`.
  - Shell whose first token is `gh` → `github`.
  - `mcp__github__*` → `github`.
  - Other `mcp__<server>__*` → `mcp` with `server`.
  - Web search/fetch stays `globe`; other shell stays `terminal`.
- `mcp` rows render the connector logo from `CONNECTOR_LOGOS` for known servers
  (slack, gmail, github, linear, notion; matched case-insensitively and on a
  `claude_ai_Notion`-style prefix), else the wrench. Logos inherit `currentColor`.
- `sameItems` memo and `SELF_OPENING_STEPS` are unchanged.

### 4. Gitplace (phase 1)
- `AppView` gains `"gitplace"`; `chromeTitle` shows `Gitplace`.
- Sidebar nav row `Gitplace` (GitPullRequest icon) after Usage; bottom-rail source
  control button opens Gitplace and is labelled `Gitplace`.
- `GitplaceScreen` lists workspaces with a path as repositories, remembers the last
  choice in localStorage, and renders `GitHubPane` with `layout="page"` and no session.
- `GitHubPane layout="page"`: repo slug not truncated at page widths; at ≥1100px a
  selected PR or issue shows as a two-column list + detail.
- Jumping to a review comment from Gitplace opens a chat in that workspace (most
  recent, or a new one) and then the file.
- No workspace with a path → empty state `Add a project with a GitHub remote`.

### 5. Chrome
- `checkpoint`, `compaction` (non-failed), `context-compacted`, `branch-summary` render
  as one faint centred line that expands to show text; failed compaction keeps the
  card with Retry.
- A branch summary whose text is `Session started` renders nothing.
- Connector, GitHub and attention toasts share one top-right stack below the title bar;
  none sits at the bottom-right over the composer.
- A chat whose title is empty and label is a placeholder (`Orchestrator`,
  `Bridge orchestrator`, `New chat`) is named `New chat`.
- Nav: duplicate Usage rail button removed; `Agent Fleet` renamed `Terminals`;
  sidebar section label `Projects` (matching the nav).
- Stale-base card: title + action only; no `N behind · M ahead · base` summary repeat.
- Verifying card uses the neutral `border-x-border` tick, not info.
- A short canvas fade sits above the composer.
- Mock codex session compaction is stamped `harness: "codex"` so it reads `Codex`.

## Unit Tests
- `startupNarration.test.ts`: default label `Thinking`; no label contains a harness or
  model name; elapsed hidden at 9s, shown at 10s.
- `toolCall.test.ts`: `git status` → `git`; `gh pr view 1` → `github`;
  `mcp__notion__search` → `mcp` + `server: "notion"`; `mcp__github__get_pr` → `github`;
  `ls` stays exploratory; `bun test` → `terminal`.
- `sidebarChats` test: placeholder label with no title → `New chat`; a real title wins.
- `designSystem.test.ts` stays green.

## Integration / Functional Tests
- `AgentConversation` jsdom: clicking Copy on an assistant reply calls
  `navigator.clipboard.writeText(item.text)`; hover actions on a user bubble are
  absolutely positioned (zero in-flow height).
- `AgentConversation.transcript` jsdom: forest events render as `data-forest-line`
  rows, not cards; `Session started` branch summary renders nothing; the activity
  group header reads `Worked for`.
- `GitplaceScreen.test.tsx` jsdom: renders with no session; lists repos; switching
  repos remounts `GitHubPane` with the new `workspaceId`; empty state with none.
- Existing AgentConversation / motion / golden tests updated only where copy changed.

## Smoke Tests
- `bun run build` green.
- `bun run test` frontend (`bunx vitest run`) green. Rust untouched; `cargo test`
  run if time allows, not affected by this change.

## E2E Tests
N/A. No E2E harness for the webview; covered by mock-mode screenshots below.

## Manual Tests
- `bunx vite --port 1421`, headless Chrome over CDP at 1440×900 dark:
  - Orchestrator chat: activity reads `Worked for`, rows borderless, git/gh rows
    show marks, latest reply has Copy bar.
  - Sending a message shows logo + `Thinking`.
  - New chat shows no `Session started` card.
  - Gitplace opens from the sidebar with no chat selected.
  - No toast covers the composer.
- `grep -rn "is reading your message" src/` returns nothing.
