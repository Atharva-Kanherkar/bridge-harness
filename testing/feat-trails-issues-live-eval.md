# feat-trails-issues-live-eval — Test Contract

Follow-ups to Trails v1 (`testing/feat-trails.md`, issue #987, PR #993).
Plan note: `bridge://p/01a1dab1-3654-7042-85b9-4dd7e600b8c2`.

Three gaps, one PR:

1. Criterion 4 of #987 was never measured: a chat started on a trail, on a
   harness no earlier chat used, answers from the brief.
2. GitHub issues cannot be trail members.
3. The Trails list reads stored PR snapshots that the GitHub poller never
   writes, so a trail does not change group when its PR's checks or state
   change.

## Functional Behavior

### 1. The recall eval (`trails::eval`, live, gated)

- Runs only with `BRIDGE_TRAILS_EVAL=1` under `--ignored`. Harness from
  `BRIDGE_TRAILS_EVAL_HARNESS` (default `claude`), model from
  `BRIDGE_TRAILS_EVAL_MODEL` (default: the harness's own).
- Fixture, seeded through the real stores (no hand-built forest JSON): a
  trail "Archive a chat" with
  - chat A on harness X and chat B on harness Y, both different from the
    harness under test, each with user and assistant messages in its forest
    that record decisions;
  - a plan note with decisions;
  - an open PR #12 with a failing check `vitest (macos)` in its snapshot;
  - an unsent draft to "Yamada";
  - (after slice 2) an issue #41 in the issue store with a body that states a
    requirement.
- Facts are arbitrary so a guess cannot pass: undo lives in a toast for
  **8 seconds**; a worktree is reclaimed after **14 days**; the keyboard
  shortcut was dropped because it collides with **Arc**; the archive button
  sits **before the harness mark**; the issue asks for archived chats to be
  **searchable** from ⌘K.
- Treatment: a new session on the harness under test, `carry_trail_brief`
  with the preview's default selection, instructions built exactly as a cold
  direct-chat start builds them (same configured prompt, same prompt stack,
  `restoration::checkpoint_context`, `compile_purposed_session_prompt`), one
  question per chat, asked the way a person would ("what did we decide about
  undo?").
- Control: the same question to a fresh session with no carry.
- Scoring: on the final assistant message only. Each case has `all` keyword
  groups (each group is any-of, case-insensitive) that must all match.
- Gate: treatment passes every case but at most one; control passes at most
  one case. Every chat must start; a harness that does not start is a
  failure, not a skip. Output prints per-case `ok`/`MISS`, the reply on a
  miss, and the totals.
- Deterministic sibling (runs in the normal suite,
  `trails::eval::fixture_brief_carries_every_fact`): after the carry, the
  compiled cold-start instructions contain every fact string; the control's
  contain none.

### 2. Issues as members

Store (migration 78):

- `issue_snapshots(url PRIMARY KEY, repo_host, repo_owner, repo_name, number,
  title, state, state_reason, author, labels, body, comments, created_at,
  closed_at, fetched_at)`. `labels` and `comments` are JSON arrays.
  `comments` keeps the last 5, each `{author, body, createdAt}`.
- `session_pull_requests.closing_issues` (JSON array of issue URLs, nullable).
  Written whenever a PR brief is read (attach, card refresh, poller), from
  `closingIssuesReferences`.

References:

- `TrailMemberKind::Issue` (`"issue"`), ref = canonical issue URL
  `https://<host>/<owner>/<name>/issues/<n>`.
- `parse_issue_reference(raw, default_repository)` accepts the URL (trailing
  slash, `#comment` fragment and query ignored), `owner/name#n`, and `#n` /
  `n` when a default repository is given. Anything else, a PR URL, or
  `#n` without a default repository is refused with a message saying what
  to paste.
- `add_trail_member` with kind `issue` canonicalizes the ref, reads the issue
  from GitHub, stores the snapshot, then adds the member. An issue `gh`
  cannot read is refused with the `gh` error and adds nothing. The default
  repository is the trail's project's repository.

Timeline:

- Issue member with a snapshot: event kind `issue`, at `created_at`, title
  `#n title`, detail `open` / `closed` / `closed as not planned` plus
  `· k comments` when k > 0, url = issue URL, tone success when closed as
  completed, else plain.
- Issue member without a snapshot: event kind `issue`, at added time, title
  `Issue #n`, detail `not read from GitHub yet`.
- Ties: kind order becomes issue, note, session, pullRequest, outgoing.
- An issue counts toward stage 1 (Plan), like a note. Issues never make a
  trail `needsYou` and are not `open` items.

Brief:

- Issue items come first (`issue:<url>`), label `Issue #n title`, default on.
  Text: header, `#n title`, URL, state, labels, the body clipped to 8 000
  bytes, then up to the last 5 comments each clipped to 1 200 bytes with
  author and date. An issue without a snapshot carries number, URL and
  "not read from GitHub yet".

Automatic adds (hard links only, no model call), applied with the existing
rules in the same transitive sweep:

5. An issue URL in a member PR's `closing_issues` joins the trail.
6. A PR (any `session_pull_requests` URL) whose `closing_issues` contains a
   member issue joins the trail.

- Dismissed and other-trail records are skipped, as for the other rules.
- After a sweep adds issues without a snapshot, the maintenance loop reads
  them from GitHub outside the database lock and publishes a refetch hint.

Refresh:

- `read_trail {refresh: true}` re-reads member issues as well as PRs.

### 3. Fresh PR state

- `github_poll::poll_attached` writes the snapshot for every chat row of
  that PR (same repository and number), built from the brief and checks it
  just read, before it publishes `GithubChecksChanged`. It writes on every
  successful poll so `fetchedAt` is the last read. A failed read writes
  nothing.
- At startup and after a sweep, the trail loop arms the poller
  (`watch_attached`) for every PR member of an open trail whose stored
  snapshot is open (or has none). Merged and closed PRs are not watched.
  Arming is idempotent.
- Result: with no chat or trail page open, a check that fails on a trail's
  PR moves that trail to Needs you on the next poll, and a merge moves it to
  Shipped. Lag is at most one poll cadence (15 s while checks run, 2 min
  otherwise) plus the 1.5 s debounce.

### Protocol (minor 1.35)

- `TrailMemberKind` gains `issue`; `TrailEventKind` gains `issue`. No new
  methods. Artifacts regenerated.

### Frontend

- Trail page: an issue row with an issue icon opens the URL; "Add issue"
  in the page header opens a one-field form (URL, `owner/repo#n`, or `#n`
  when the trail has a project), shows the core's refusal inline, and closes
  on success.
- `memberNoun` names `issue`. Mock data has one issue member.

## Unit Tests

- `trails::issues::tests::parses_every_accepted_form` and
  `refuses_pull_requests_and_bare_numbers_without_a_repository`.
- `trails::issues::tests::snapshot_round_trip_keeps_last_five_comments`.
- `trails::timeline::tests::issue_rows_and_ties` (open, closed, not planned,
  no snapshot, tie order issue before note).
- `trails::timeline::tests::an_issue_counts_as_plan_and_never_needs_you`.
- `trails::brief::tests::issue_item_comes_first_and_is_clipped`.
- `trails::auto::tests::a_member_prs_closing_issue_joins` and
  `a_pr_that_closes_a_member_issue_joins` and
  `a_dismissed_issue_is_not_re_added`.
- `session_prs::tests::closing_issues_are_stored_with_the_brief`.
- `github_poll::tests::a_poll_writes_the_snapshot_before_it_publishes`
  (subscribe, poll, on the event read the row: snapshot has the new checks).
- `github_poll::tests::a_failed_poll_keeps_the_snapshot`.
- `trails::live::tests::open_trail_prs_are_watched_and_merged_ones_are_not`.
- `trails::eval::fixture_brief_carries_every_fact` (deterministic).
- Store migration test: `migration_78_adds_the_issue_store`.

## Integration / Functional Tests

- `trails::live::tests::a_failing_check_moves_a_trail_to_needs_you_without_a_read`:
  core with a trail whose PR snapshot is green; a fake `gh` answers a failing
  check; one `poll_once`; `list_trails` (no refresh) shows `needsYou`.
- Same for merged → `shipped`.
- Protocol round trip for `TrailMemberKind::Issue` and `TrailEventKind::Issue`;
  `tsgen::checked_in_artifacts_match_the_contract`.
- Vitest: `TrailPage` renders an issue row and the Add issue form posts
  `{kind: "issue", ref}` and shows a refusal.

## Smoke Tests

- `bun run build` and `bun run test` green.
- `cargo test -p bridge-protocol` green after regenerating artifacts.

## E2E Tests

- Live eval: baseline on main's code (treatment and control), then after
  slice 2 (with the issue case) and after slice 3. Results pasted in the PR.

## Manual / cURL Tests

Against a real `bridged` on a scratch data dir (never the live DB):

- Add `https://github.com/Atharva-Kanherkar/bridge-harness/issues/987` to a
  trail by URL; `read_trail` shows `#987 …` closed; `preview_trail_brief`
  lists `issue:…` first.
- Add `#987` to a trail with no project: refused, names the forms to paste.
- Add PR #993's URL as a PR member of a trail whose chat has it attached:
  the sweep adds issue #987 (its closing issue).
- UI in mock mode: issue row, Add issue form, refusal shown.
