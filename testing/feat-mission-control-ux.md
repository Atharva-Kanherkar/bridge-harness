# feat/mission-control-ux — Test Contract

## Problem

Mission Control shows every live chat as a tile, but a tile does not say what it
is. The project sits in 10px grey text after the status. The title is the stored
three-word heading, which for a pasted link is the raw URL
(`https://github.com/…/pull/43 reviewe…`) and for a typo'd first message is the
typo (`Pleqase ignore repo`). Every tile is equally loud, so the one waiting for
an approval does not stand out. The header carries six always-on icon buttons,
the composer repeats the title in its placeholder and takes two rows, the
transcript scrolls sideways inside narrow tiles, and the view title appears twice.

## Design principles (and the rule each one sets)

1. **Recognition over recall.** Every tile answers "which project, which task" in
   one glance: a project chip, a readable title, and the last thing you asked.
2. **Memory for goals / resumption cues** (Altmann & Trafton 2002; Leroy 2009).
   The header's second line is your latest message to that chat, so returning to
   a tile does not start with scrolling.
3. **One pre-attentive signal** (Treisman & Gelade 1980). Only a tile that needs
   you gets hue in its chrome; working tiles are calm, finished ones recede.
4. **Proximity / common region.** A new chat opens beside its own project's
   tiles; **Arrange** re-tiles into a balanced grid grouped by project.
5. **Spatial stability.** Tiles never reorder on their own when a status
   changes; only a new chat, a closed chat, or an explicit Arrange moves them.
6. **Hick's law / calm chrome.** Secondary actions appear on hover or keyboard
   focus; closing and the pinned state stay visible at rest.

## Functional Behavior

### Titles (`src/components/missionControl/identity.ts`)
- `displayTitle(raw, project)` drops the repository from a derived link heading
  (`kairo PR #43` → `PR #43`) inside that repository's own tile. Every other
  title, including a user-chosen one, passes through. The backend is the only
  link parser; its open-time backfill repairs stored derived titles.
- `projectLabel(session, workspace, projects)` prefers the owning project's name,
  then the workspace title, then the last segment of `session.cwd`, then `null`.
- `latestAsk(entries, events)` returns the newest user message, one line, links
  compacted: live events first (decoded by the transcript codec), then forest
  `user.message` entries. A just-submitted message shows immediately and gives
  way once the stream records any newer ask.

### Backend titles (`src-tauri/bridge-core/src/session_titles.rs`)
- `heading_from_message` names a message that opens with a GitHub PR or issue URL
  `<repo> PR #<n>` / `<repo> issue #<n>`, and a bare repository URL `<repo>`.
  Non-GitHub URLs contribute their host and last readable path segment. Brackets
  and trailing punctuation around the link, and its query or fragment, are
  ignored. Existing
  derived titles are repaired by the existing backfill on the next open.

### Tile header (`src/components/MissionControl.tsx`)
- Row 1: project chip, display title, status pill (dot + sentence-case label:
  Working, Needs you, Done, Failed, …) and elapsed time.
- Row 2: the harness mark and `latestAsk` (falls back to the harness label).
  The raw stored title is the title attribute of the display title.
- Actions keep their accessible names (`Focus chat`, `Maximize tile` /
  `Restore grid`, `Stop worker`, `Pin chat in Mission Control` / `Unpin chat`,
  `Close chat`). All but Close are revealed on header hover or focus-within, in
  place of the elapsed time; the status pill never hides. Collapsed buttons stay
  focusable. A pinned tile shows its pin state at rest.
- Only a tile whose status tone is `waiting` (an approval or question for you)
  carries `data-attention="true"`: an inverted "Needs you" pill with an icon and
  a stronger border. A worker's blocked, needs-delegation, or unreadable result
  is not counted. `warning` is a warm grey by design, so salience comes from
  luminance, the icon, and the words.

### Board toolbar
- The duplicate visible "Mission Control" heading is removed (the window title
  already says it); the landmark keeps `aria-label="Mission Control"`.
- A summary reads `N needs you` (only when N > 0, a button that scrolls to and
  focuses the composer of the next waiting tile after the one you were last in,
  in board order, wrapping), `N working`, and `N pinned` when any. Only the
  needs-you count sits in a polite live region.
- A project legend lists each project on the board with its tile count when there
  are at least two projects. Hovering or focusing a project marks its tiles with
  `data-project-highlight="true"` and dims the rest; clicking toggles it sticky.
- **Arrange** rebuilds the tree with `arrangeLeaves`, clears maximization, and
  persists the result.

### Layout (`src/components/missionControl/layout.ts`)
- `insertLeaf(root, id, near?)` splits the largest leaf among `near` when any of
  them are on the board, else the largest leaf overall (existing behavior).
- `arrangeLeaves(ids)` returns `null` for no ids, a leaf for one, and otherwise
  rows of `ceil(sqrt(n))` columns with equal ratios, in the order given.
- `reconcileLeaves(root, ids, groupOf?)` inserts new leaves beside same-group ones.
- Minimum tile size stays 420 × 360; saved layouts from before load unchanged.

### Composer and transcript
- `ComposerPill` gains `layout="inline"`: the textarea and its Stop / Send
  buttons share one row. Mission Control uses it with placeholder
  `Steer this chat…` while working and `Reply…` otherwise. The title is no
  longer echoed.
- `AgentConversation` gains `density="compact"`: tighter padding at any
  viewport and no horizontal scroll. Mission Control's wrapper no longer scrolls
  on its own, so a tile shows one vertical scrollbar.

## Unit Tests
- `src/components/missionControl/identity.test.ts`: PR / issue / repo / other URL
  titles, project-prefix rules, truncated-fragment drop, plain titles untouched;
  `projectLabel` precedence; `latestAsk` picks the newest user message and skips
  blanks.
- `src/components/missionControl/layout.test.ts`: `arrangeLeaves` for 0, 1, 2, 3,
  4, 5 ids; `insertLeaf` with `near`; `reconcileLeaves` groups a new leaf beside
  its project.
- `session_titles.rs`: GitHub PR, issue, repository and other-host URLs.
- `ComposerPill.test.tsx`: inline layout keeps Send and Stop on the textarea row.

## Integration / Functional Tests
- `src/components/MissionControl.test.tsx`: every existing behavior stays; new
  cases: project chip and display title per tile; latest ask line; attention tile
  flagged and the "needs you" button focuses its composer; legend highlight;
  Arrange persists a grouped layout; a new chat is inserted beside its project;
  placeholder no longer contains the title.

## Smoke Tests
- `bunx tsc -b`, `bunx vitest run`, `bun run build`,
  `cargo test -p bridge-core session_titles`.

## E2E Tests
- N/A. Preview-mode screenshots (before / after) with four live chats across
  three projects, one waiting on an approval, go in the PR description.
