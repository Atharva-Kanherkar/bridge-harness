# Trails

A trail is one page per piece of work. It lines up the chats, notes pages,
pull requests and drafted messages that belong to that work, in time order,
says what waits on the person, and starts the next chat already carrying it.

## Model

A trail is a read model over records Bridge already keeps. It stores only:

| Table | What it holds |
|---|---|
| `trails` | id, title, optional project, created/updated/archived times |
| `trail_members` | `(kind, ref)` pointing at a chat (session id), notes page (page id), pull request (URL) or draft (outgoing id), plus who added it |
| `trail_dismissals` | records the person took off a trail, so they never come back on their own |

`PRIMARY KEY(kind, ref)` keeps every record on at most one trail. Removing a
member or archiving a trail changes no chat, page, PR link or draft.

Everything a page shows (timeline, state, stage, open items) is derived on
each read in `trails::timeline`. Notes live in their own database, so callers
read them into a `NoteSnapshot` before taking the core database lock.

## Automatic adds

`trails::auto::sweep` follows hard links from every chat on an open trail:
a PR attached to it, a draft it wrote, a notes page linked to it, a chat
forked from or spawned by it. Rules repeat until nothing new joins. No model
decides membership. `trails::live` runs the sweep 1.5 s after a relevant core
event and publishes `trails/changed` with each addition once; the app shows
an "Added to a trail" toast whose Undo removes the member (and records the
dismissal).

## Starting a chat on a trail

`trails/preview_trail_brief` lists what a new chat can start with: notes
(clipped to 12 KB), PR state and failing checks, each chat's restoration
projection at a 24k-token window, and drafts. `trails/carry_trail_brief`
writes the chosen items into the new chat as `handoff.brief` forest entries
before its first message, the path the `$harness` shortcut uses. The cold
start projects them into labelled restoration context for whichever harness
runs the chat.

Restoration clips one entry to a sixth of its budget and stops once the
budget is spent, so a carry splits a long item into parts that each fit and
refuses a selection over 90% of the target chat's budget. A carry never
changes the chat's harness, model, access, worktree or scope.

## Protocol

Protocol 1.32: `trails/list_trails`, `read_trail` (`refresh` re-reads member
PRs from GitHub), `create_trail`, `update_trail`, `add_trail_member`,
`remove_trail_member`, `preview_trail_brief`, `carry_trail_brief`, and the
`trails/changed` notification.

## Not in v1

GitHub issues as members (no issue store yet), model-suggested membership,
sharing a trail, and editing members inline on the page.
