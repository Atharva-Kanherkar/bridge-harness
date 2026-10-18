# feat/command-readability: test contract

Issue #833. Tool rows lead with what the agent is doing, compound shell
commands open into steps, known structured output renders as UI, and a live
turn shows a Now card. Frontend only; no wire change.

## Functional behavior

### Code font

- Every mono surface (`code`, `pre`, `kbd`, `samp`, `.font-mono`) turns
  ligatures off. `gh pr view 70 --json title` keeps its space before `--`,
  and `-- --check` no longer draws as a long dash.

### Reading a shell command (`src/transcript/command.ts`, React-free)

- **Unwrap the harness shell.** `/bin/zsh -lc "…"`, `bash -lc '…'`, `sh -c "…"`
  and `/bin/bash -c …` become the script inside. Double-quoted escapes
  (`\"`, `\\`, `\$`, `` \` ``) and single-quote splices (`'"'"'`, `'\''`) are
  undone. `bash script.sh` and `zsh -lc` with trailing arguments stay as typed.
- **Split** on `;`, `&&`, `||`, `|` and newlines only outside quotes, `$(…)`,
  backticks, `( … )` and heredoc bodies. `grep "a;b" f` is one step.
- **One step per pipeline.** The first command names the step. Later pipe
  stages become notes: `head`/`head -n N`/`head -N` → `first N lines` (default
  10), `head -c N` → `first N bytes`, `tail …` → `last N lines`, `grep X` →
  `filtered: X`, `wc -l` → `counted lines`, and `sort`/`uniq` add no note. An
  unknown stage becomes `piped to <bin>` and makes the step `exec`.
- **Redirects become notes.** `2>/dev/null` → `errors hidden`; `>/dev/null`,
  `&>/dev/null` and `>/dev/null 2>&1` → `output hidden`; `2>&1` adds nothing;
  `> f`/`>> f`/`tee f` → `writes <basename>` and the step is `write`.
- **`cd X` is not a step.** It becomes an `in <X>` note on the steps after it.
  `git -C X …` gets the same note.
- **`|| true` / `|| :`** is not a step; the step before it gets `failure ignored`.
- **Effect per step**, with the strongest one winning for the whole command
  (exec > write > read):
  - read: cat/head/tail/less/bat/nl, `sed -n`, ls/tree/`rg --files`, grep/rg/ag,
    find/fd, wc/stat/file/which/whereis/`command -v`, git
    status/diff/log/show/branch/tag/remote/describe/rev-parse/ls-files/blame,
    gh `* view|list|diff|checks|status`, `gh api` without a write method or
    fields, df/du/pwd/echo/printf/env/printenv/date/uname/whoami/hostname/sleep,
    `jq` on a file.
  - write: `sed -i`, mkdir/touch/rm/rmdir/mv/cp/ln/chmod/chown, git
    add/commit/checkout/switch/reset/restore/stash/merge/rebase/cherry-pick/apply/rm/mv/clean/worktree,
    any `>`/`>>`/`tee` target.
  - exec: everything else, including unknown programs, interpreters, package
    scripts, curl/wget, git push/pull/fetch/clone, and gh create/merge/comment/edit.
    Unknown is never `read`.
- **Phrases** (doing / done + target):
  - `gh pr view 70` → Reading / Read `PR #70`, and `gh pr view` → `the current PR`.
  - `gh pr list` → Listing / Listed `pull requests`.
  - `gh pr diff 70` → Reading / Read `PR #70 diff`.
  - `gh pr checks 70` → Checking / Checked `CI on PR #70`.
  - `gh issue view 12` → Read `issue #12`.
  - `gh run view 9` → Read `workflow run 9`.
  - `gh api repos/x/y` → Called / Calling `GitHub API repos/x/y`.
  - `sed -n '1,20p' f` → Read `f` with the note `lines 1 to 20`.
  - `df -h /p` → Checked `disk space`.
  - `git checkout b` → Switched to `b`, `git commit` → Committed `changes`.
  - `git push` → Pushed `branch`. Existing git read phrases keep their wording.
  - Unknown → Running / Ran `<bin> <first arg>`.
- **Chain facet.** A single step uses that step's verb, glyph, doing/done,
  target and path. Several steps lead with the first step that has the
  strongest effect, and the target gets ` + N more steps` appended. Verb:
  read/search when every step is read-only (the first step's verb), otherwise
  `run`. The `effect` covers the whole command.
- Results are cached per command string, so re-reading a command on every output
  delta stays cheap.

### Tool facet (`src/transcript/toolCall.ts`)

- `ToolCallDisplay` gains `intent?`, `effect?` and `steps?`.
- `intent` is Claude `Bash` `input.description`. ACP `title` counts only when
  it is not the command itself. The model is never asked to summarise.
- `command` is the unwrapped script, so check detection works for Codex:
  a wrapped `cargo test` is a test check.
- `steps` is present only when there are two or more steps after `cd` folding.
- Glyph: git/gh marks keyed off the lead step's program.

### Row (`AgentConversation.tsx` `ActionRow`)

- Title: `intent` if present, else the existing `doing`/`done` + target. Check
  rows keep the command as their title.
- When the title is not the command, the command follows the title as a
  single-line, muted, truncated mono suffix (sm and up).
- Shell rows that are not checks get an outlined neutral effect chip:
  `read-only`, `changes files`, `runs code`.
- `CappedOutput` reads `Show earlier output`, with no em dash.

### Command panel (`src/components/CommandOutput.tsx`)

- Views: `Steps` (two or more steps), `Rendered` (structured output detected),
  `Output` (the existing terminal block: `❯ command` + raw output).
- The tab bar shows only when there are two or more views. Default view:
  Rendered, else Steps, else Output. A Copy button copies the command.
- Steps lists the numbered steps with glyph, phrase, target in an inline code
  chip, and notes as outlined chips.

### Structured output (`src/transcript/outputRender.ts`, React-free)

- `detectStructuredOutput(command, output)` returns
  `{ kind: "gh-pr" | "gh-issue" | "gh-run" | "gh-list" | "json", data, truncated }`
  or `null`.
- Only output that starts with `{` or `[` is tried, and only up to 512 KB.
- A truncated document (`| head -c 20000`) is repaired: the open string is
  closed, a dangling key or comma is dropped, and brackets are closed. It is
  flagged `truncated`; if repair fails the result is `null`.
- Kind comes from the lead step: `gh pr view` + object with `title` → gh-pr,
  `gh issue view` → gh-issue, `gh run view` → gh-run, and `gh pr|issue list`
  + array of objects with `number` and `title` → gh-list. Any other parsed
  object or array → json.
- Renderers:
  - PR card: repo/#, state, title, author, head → base, files, ±lines, commits,
    reviews, body as Markdown, first 6 files and `+N more`.
  - Issue card: #, state, title, author, labels, body.
  - Run card: title, workflow, status/conclusion, jobs.
  - List rows: `#n title state`.
  - JSON tree: nested `<details>`, the first two levels open, strings with
    newlines pre-wrapped.
  - Truncated output shows the note `Output was cut off; showing what arrived.`
- Detection runs only when the panel is open, never at ingestion.

### Now card (`ActivityGroup`)

- While the group is live and its newest tool is not a check, a `data-now-card`
  card replaces today's one-line current-step row. It shows a spinner, the title
  (intent, or doing + target), elapsed seconds when the call has a parseable
  `createdAt`, the command suffix, and the last 3 non-empty output lines.
- Plan chips come from the newest plan item in the group (`data.plan[{step,status}]`)
  or the newest TodoWrite (`input.todos[{content,status}]`). Completed chips
  show a tick, in-progress chips a spinner, and pending chips are plain.
- The card is gone once the turn ends or the group stops being live.

## Unit tests

- `src/transcript/command.test.ts`: unwrapping (double, single,
  `'"'"'`, not-a-wrapper), quote/`$()`/heredoc-aware splitting, every note
  mapping, cd and `git -C` notes, `|| true`, the effect table including the
  unknown→exec and strongest-wins cases, each phrase above, the chain lead step
  and `+ N more steps`, and cache identity.
- `src/transcript/outputRender.test.ts`: gh-pr, gh-issue, gh-run and gh-list
  detection, unknown JSON, non-JSON, a `head -c` truncated PR (repaired and
  flagged), unrepairable input → null, oversize → null.
- `src/conversation.test.ts`: update the chain case. `git status && git diff`
  reads `Checked git status + 1 more step` with verb read and two steps; the
  mixed chain stays `run`. Add: wrapped Codex command unwrapped, wrapped
  `cargo test` is a check, and Claude `description` becomes the intent.
- `src/designSystem.test.ts`: the base layer turns ligatures off on mono surfaces.

## Integration / functional tests (jsdom, `AgentConversation.transcript.test.tsx`)

- Claude Bash with `description` shows the description as the row title, the
  command as the suffix, and `read-only`.
- Opening a 3-step command shows Steps by default with 3 numbered steps;
  Output shows `❯` and the raw output.
- `gh pr view 70 --json …` output opens on Rendered with the PR title, and the
  body's markdown table renders as a `<table>`.
- A live group shows `[data-now-card]` with the intent and plan chips, and the
  card is gone once the turn ends.
- The existing `Ran bun run migrate`, `Ran N commands` and failure-only
  assertions still pass.

## Smoke tests

- `bun run check`, `bun run build` and `bun run test` are green. The
  flood/history perf tests stay within their budgets.

## E2E tests

N/A: there is no browser E2E harness for the transcript. Covered by jsdom tests
plus a mock-mode visual check.

## Manual checks

- A throwaway Vite entry renders `AgentConversation` with a Claude described
  command, a wrapped Codex chain, a `gh pr view` JSON output and a live turn.
  Take dark, light and expanded screenshots at 2x and put them in the PR body;
  delete the entry before committing.
- Confirm `70 --json` renders with its space in the screenshot.
