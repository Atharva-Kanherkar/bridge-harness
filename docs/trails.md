# Trails

A trail is one page per piece of work. It lines up the issues, chats, notes
pages, pull requests and drafted messages that belong to that work, in time order,
says what waits on the person, and starts the next chat already carrying it.

## Model

A trail is a read model over records Bridge already keeps. It stores only:

| Table | What it holds |
|---|---|
| `trails` | id, title, optional project, created/updated/archived times |
| `trail_members` | `(kind, ref)` pointing at a GitHub issue (URL), chat (session id), notes page (page id), pull request (URL) or draft (outgoing id), plus who added it |
| `trail_dismissals` | records the person took off a trail, so they never come back on their own |
| `issue_snapshots` | the last read of each issue member: state, labels, body, last five comments, the PRs that close it |
| `trail_issue_links` | hard links between a PR URL and an issue URL, from either side |
| `trail_pr_link_reads` | which PRs' closing references were read, and when |

`PRIMARY KEY(kind, ref)` keeps every record on at most one trail. Removing a
member or archiving a trail changes no chat, page, PR link or draft.

Everything a page shows (timeline, state, stage, open items) is derived on
each read in `trails::timeline`. Notes live in their own database, so callers
read them into a `NoteSnapshot` before taking the core database lock.

## Issues

Bridge has no other issue store, so a trail keeps its own: "Add issue" on a
trail page takes the issue's link, `owner/repo#n`, or `#n` when the trail has
a project. The core reads it with `gh issue view --repo`, from any folder,
stores the snapshot, and refuses an issue `gh` cannot read. The member is the
issue's canonical URL. Issues sit first on the timeline and in the brief
(body clipped to 8 KB, the last five comments to 1.2 KB each), count toward
the Plan stage, and never make a trail need the person.

Links come from both sides: the issue's `closedByPullRequestsReferences`, and
a separate trail-only `gh pr view --json closingIssuesReferences`. That field
is never added to the chat PR card's own `gh pr view`, because `gh` refuses
an unknown `--json` field and an older `gh` would lose every card.

## Fresh pull request state

The GitHub poller writes what it reads (state and checks) to every chat row
of that PR before it publishes `GithubChecksChanged`, and the trail loop
arms the poller for every open PR on an open trail, at start and after each
sweep. So a red check moves a trail to Needs you, and a merge moves it to
Shipped, with no chat or trail page open: within one poll (15 s while checks
run, 2 min otherwise) plus the 1.5 s debounce. A failed read stores nothing.

## Automatic adds

`trails::auto::sweep` follows hard links from every chat on an open trail:
a PR attached to it, a draft it wrote, a notes page linked to it, a chat
forked from or spawned by it; and from its PRs and issues: an issue a member
PR closes, and a PR attached to any chat that closes a member issue. Rules
repeat until nothing new joins. No model decides membership. After each
sweep the loop reads, outside the database lock, issue members with no
snapshot and PR members whose closing references were never read, then
sweeps again if a link landed. A failed background read waits ten minutes;
Refresh on the page never waits. `trails::live` runs the sweep 1.5 s after a relevant core
event and publishes `trails/changed` with each addition once; the app shows
an "Added to a trail" toast whose Undo removes the member (and records the
dismissal).

## Starting a chat on a trail

`trails/preview_trail_brief` lists what a new chat can start with: issues,
notes (clipped to 12 KB), PR state and failing checks, each chat's restoration
projection at a 24k-token window, and drafts. `trails/carry_trail_brief`
writes the chosen items into the new chat as `handoff.brief` forest entries
before its first message, the path the `$harness` shortcut uses. The cold
start projects them into labelled restoration context for whichever harness
runs the chat.

Restoration clips one entry to a sixth of its budget and stops once the
budget is spent, so a carry splits a long item into parts that each fit, and
budgets what restoration actually charges per entry (the text, the
`handoff.brief: ` prefix, part labels and a newline). It refuses a selection
over 90% of the target chat's budget; an admitted selection restores whole.
Every refusal happens before anything is written, and the entries, the
membership and the audit row commit in one transaction. In the app, Continue
re-reads the brief before creating the chat; if the carry still fails, the
empty chat is archived and the first message is not sent. A carry never
changes the chat's harness, model, access, worktree or scope.

## Protocol

Protocol 1.33: `trails/list_trails`, `read_trail` (`refresh` re-reads member
PRs, issues and closing references from GitHub), `create_trail`, `update_trail`, `add_trail_member`,
`remove_trail_member`, `preview_trail_brief`, `carry_trail_brief`, and the
`trails/changed` notification. Protocol 1.35 adds the `issue` member and
event kinds; a 1.35 daemon refuses 1.33 and 1.34 clients, which read trails
but cannot draw an issue.

## Measuring recall

`trails::eval::live` (gated, `BRIDGE_TRAILS_EVAL=1`) checks the point of the
brief: a chat started on a trail, on a harness no earlier chat used, answers
questions whose answers live only in the trail (an 8-second undo, a 14-day
reclaim, a failing check, an unsent draft, the issue's ask). A control chat
with nothing carried must miss. `trails::eval::fixture_brief_carries_every_fact`
is its deterministic half and runs with the normal suite.

## Not yet

Model-suggested membership, sharing a trail, editing members inline on the
page, and bulk import of a repository's issues.
