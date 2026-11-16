# feat/github-pane-merge-flow — Test Contract

Source of truth: issue Atharva-Kanherkar/bridge-harness#1028, its reference mockups
(`github-pane-merge-flow.html`, `github-pane-action-sheet.html`, `github-pane-motion.html`
in gist `b43e04cd761785d98888973193d11674`) and its screenshots/GIFs. Timings, easings
and copy in those files are the spec. This contract locks what "done" means before any
implementation starts.

## Functional Behavior

### Merge box (the hero)

- An open PR's detail renders **one merge box**: readiness rows (checks with
  `RollupMeter`, review decision, mergeability), a split primary button
  `<strategy label> | caret`, secondary `Approve` + `Review` + an overflow `⋯` menu
  holding Request changes / Ready for review (draft only) / Re-run failed (when a
  failed check exists) / Check out (when the head is not checked out here) / Close.
  At most three visible secondary actions plus the overflow.
- `githubMergeConfig` is loaded **when the box mounts**, not on click. While loading,
  the split button shows the generic label disabled with no spinner; when loaded it
  shows the repo default strategy label. Only `mergeConfig.strategies[name] === true`
  strategies appear in the menu; the selected one carries a check mark.
- Clicking the split button (or picking a strategy from the menu) expands the
  **confirm step in place** — no `Dialog` in the DOM for merge. Readiness rows and the
  actions row collapse (grid-rows 1fr→0fr); the confirm step expands with: a lands row
  ("Squashes 2 commits into one commit on main"), a commit-message field prefilled
  `{title} (#{number})` (squash) / `Merge pull request #{number} from {head}` (merge),
  a "Delete {branch} after merge" checkbox, and a footer `Cancel · ⌘↵ to merge ·
  Confirm squash and merge`. `Cmd+Enter` confirms; `Esc` collapses back to idle.
- While the act call is in flight (`merging`): the button label slides up to
  "Merging …", a hairline sweeps the card's bottom edge, inputs are dimmed/disabled,
  and a second click cannot submit again.
- On `executed: true` the **merged sequence plays once** in GitHub purple (`--merged`,
  `#8250df` light / `#a371f7` dark), in about 1.6s: commits gather (420ms), the curve
  draws (560ms from 500ms), a ring lands at 1.06s, the receipt rises at 1.2s. The pill
  morphs Open→Merged (width tween + glyph slide, 320ms), the sub-line reads
  "atharva merged 2 commits into main just now", and the box border tints merged.
- On `executed: false` or a thrown error: the button shakes once (300ms, ±4px), the
  box border tints red, the reason appears inline **verbatim**, the form stays filled,
  and the button reads "Try again".
- A detail read that already reports `state: "merged"` renders the receipt in its
  **final state without replaying** (also after a later refetch).

### Action sheet (every other write)

- Approve, Request changes, Ready for review, Re-run failed, Check out, Close, Reopen,
  label add/remove, comment and review replies use a pane-scoped `ActionSheet` —
  never an app-wide `Dialog`. It rises from the bottom of the pane (spring, 380ms),
  dims only the pane (scrim inside the pane's root, not `fixed`), and exits in 200ms
  ease-in.
- It names the action ("Approve pull request #341"), says the consequence in one
  sentence, keeps the exact `gh` command behind an "Exact command" disclosure, and
  labels the confirm button with the verb ("Approve", "Close pull request", …).
  Close uses the destructive fill (`bg-destructive text-destructive-foreground`).
- `Esc` cancels, `Cmd+Enter` confirms when ready, the confirm button is disabled when
  `requiresBody` and the body is blank, focus is trapped inside, and focus returns to
  the trigger on close. Children stagger in at 40ms.
- ConnectRepositoryDialog may stay a dialog (not a PR action).
- Approval semantics unchanged: one explicit click calls
  `bridgeApi.githubAct(workspaceId, action, true)`. No optimistic state.

### Everyday motion

- Tab switches: the underline glides (existing `layoutId`) and the panel slides in
  from the direction of travel (`x: ±18, opacity: 0`, 320ms ease-out), keyed on tab
  with direction from tab index delta.
- List to detail (dock layout only): the list exits `x: -30%, opacity 0` and the
  detail enters `x: 24px → 0` (320ms ease-out); back reverses. `page` layout (side by
  side at 1100px+) keeps no push.
- A check that transitions from non-completed to `success` between two reads gets a
  one-shot `github-check-settle` (glyph springs scale .4→1, row background flashes
  success 14%→transparent over 700ms). `github-check-live` keeps breathing for running
  checks. The rollup label rolls vertically when the verdict changes (Running→Passing).

### Mermaid readability

- A wide `flowchart LR` Mermaid block in a PR description is readable in the 440px
  dock: the computed scale (container width / svg viewBox width) is floored at 0.75;
  below that the diagram renders at 0.75 inside the existing `overflow-x-auto` with a
  "Scroll or expand" hint; the existing fullscreen expand stays.

### Wire (slice 5)

- The merge `GithubAction` gains optional `subject`, `body`, `deleteBranch`, mapped to
  `gh pr merge --subject/--body/--delete-branch`; a `revert` action maps to
  `gh pr revert`; a `restoreBranch` action maps to the GitHub branch-restore API
  through `gh api` (the receipt's Restore branch). Protocol artifacts regenerated;
  `cargo test -p bridge-protocol` enforces drift. The receipt's Revert / View commit /
  Restore branch buttons work: Revert and Restore branch through the confirmed act
  path, View commit as a `data-system-browser` link.

### Colour discipline

- `--merged` is used ONLY for: the Merged StatePill, the merged glyph in PR list rows,
  the merge curve / travelling commit / landing ring / landed node in `MergeGraph`, the
  receipt's check badge, and the merge box border once merged. Not for buttons, not for
  open PRs. Green stays for passing checks and approvals, red for refusals and Close.
  No confetti.

### Reduced motion

- With `prefers-reduced-motion: reduce`: transitions collapse (≤120ms cross-fade),
  the merged graph renders in its final state, no shake (red border + inline reason
  remain), and the app is fully usable.

## Unit Tests (vitest, jsdom)

`src/components/GitHubPane.test.tsx`:

- Merge box renders readiness from a mock detail; the split button shows
  `mergeConfig.defaultStrategy`; only allowed strategies appear in the menu.
- Confirm calls `githubAct` once with `{ kind: "merge", number, strategy }` and `true`
  (plus `subject` / `deleteBranch` per the confirm form).
- Double click during `merging` does not call twice.
- `executed: false` shows the message verbatim and the button reads "Try again".
- No `[role="dialog"]` from `Dialog` exists for merge.
- ActionSheet: each action builds the same `GithubAction` the current `setPending`
  builders do (port the existing assertions); `Esc` cancels; confirm disabled when
  `requiresBody` and body is blank; focus returns to the trigger on close.
- Reduced motion: with `matchMedia('(prefers-reduced-motion: reduce)')` mocked true,
  the merged receipt renders final-state markup immediately.

`src/components/Markdown.test.tsx`:

- The Mermaid wrapper applies the minimum-scale treatment when the computed scale is
  below 0.75, and not when the diagram fits.

`src/designSystem.test.ts`:

- Stays green: no hex literals in components, no banned palette classes, no blur on
  resting surfaces; the `--merged` token is asserted in both modes (GitHub purples)
  while raw purple palette classes stay rejected.

## Integration / Functional Tests

- `bun run test` (vitest suite + `cargo test`) green.
- `cargo test -p bridge-protocol` — wire round-trips for the new merge fields and the
  revert / restoreBranch actions, unknown-field rejection intact.
- Regenerated artifacts match the Rust schema (`generate-protocol-artifacts`).

## Smoke Tests

- `bun run build` (tsc + vite) green.
- `cargo check` green.
- The vitest GitHubPane suite passes including the ported confirmation assertions.

## E2E Tests

N/A — no browser harness for the desktop pane in this repo. Replaced by: a real
`bun run tauri dev` walkthrough against a scratch repository is out of scope here;
dark/light/interaction screenshots and a merge-sequence GIF recorded from the real
build are the visual evidence for acceptance criterion 10, and jsdom + cargo tests
cover behavior.

## Manual / cURL Tests

- `cargo run --manifest-path src-tauri/Cargo.toml -p bridge-protocol --bin generate-protocol-artifacts`
  produces no diff after committing regeneration.
- `bun run test -- src/designSystem.test.ts` — token and palette assertions.
- `bun run test -- src/components/GitHubPane.test.tsx` — the full pane suite.
- `bun run test -- src/components/Markdown.test.tsx` — Mermaid floored-scale test.
- Visual: `bun run build` + screenshots of idle / menu / confirm / merging / merged /
  failed in dark and light, and the action sheets, against the mockups.

## Out of scope (unchanged behavior)

- The PR list redesign beyond the push transition; Issues detail design (it reuses
  ActionSheet once slice 1 lands, no new design); the Work pane and composer shelf;
  sound, haptics, confetti; `ConnectRepositoryDialog` (stays a dialog).
