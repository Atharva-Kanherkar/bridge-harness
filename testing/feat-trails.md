# feat-trails — Test Contract

Issue #987. A **trail** is one page per piece of work: the chats, notes, pull
requests and drafted messages that belong to it, in time order, with what needs
the person and a way to start the next chat already carrying all of it.
Plan note: `bridge://p/01a1d8d0-aabb-78e1-a8d6-b77d2896ed61`.

## Functional Behavior

### Model

- A trail has `id`, `title` (1 to 120 chars after trimming; blank refused),
  optional `workspaceId`, `createdAt`, `updatedAt`, `archivedAt`.
- Members are typed references. `kind` is one of `session`, `note`,
  `pullRequest`, `outgoing`; `ref` is the session id, note page id, pull
  request URL, or outgoing message id. Each member carries `addedBy`
  (`person` or `auto`) and `addedAt`.
- One record sits on at most one trail. Adding a record already on another
  trail is refused with a message naming that trail. Adding it twice to the
  same trail is a no-op.
- Removing a member records a dismissal for that trail and record. A dismissed
  record is never added to that trail automatically again. Adding it by hand
  clears the dismissal.
- Archiving hides the trail from the default list and keeps every member.
  Unarchiving restores it. Neither changes a session, note, PR link or draft.
- Deleting a trail is not offered in v1.

### Timeline (derived on read, nothing copied)

| Member | Event | Time | Tone |
|---|---|---|---|
| session | "Chat" with title (or label), harness, model when known | session `started_at`, else the first forest entry, else added time | plain |
| note | "Note" with title, last editor (`you` or agent harness) | note `createdAt` | plain |
| pullRequest | "Pull request" `#n title`, `+state` | link `created_at` | plain |
| pullRequest with a snapshot | "Checks" line: `N of M passing`, failing names | snapshot `fetchedAt` | danger if failed > 0, success if merged, else plain |
| outgoing | "Message" to label, state (`draft` / `copied` / `discarded`), body excerpt (first 280 chars) | `updated_at` | warning if `draft` |
| any member whose record is gone | "Removed" with the kind | added time | plain |

- Ordered by time ascending; ties broken by kind order (note, session,
  pullRequest, outgoing) then by ref, so the order is stable.
- Each event names its member, so the UI can open it or remove it.

### Derived state and stage

- `needsYou` when any of: an open PR snapshot has `failed > 0`; an outgoing
  member is in `draft`; a session member has an unresolved `needs_you`
  attention row. `needsYouReason` names the first that applies, in that order.
- `shipped` when not `needsYou`, the trail has at least one PR, and every PR
  snapshot is `merged` or `closed` with at least one `merged`.
- Otherwise `inProgress`.
- Stage index: 0 none, 1 Plan (has a note), 2 Chats (has a session), 3 PR (has
  a PR), 4 Shipped (state shipped). The highest reached wins.
- `open` items: one per failing check (`name` from check details, PR number),
  one per draft message, one per chat waiting on the person.
- `harnesses`: distinct harnesses of session members with their chat counts.
- `lastEvent`: the newest timeline event's title and time.

### Automatic adds (hard links only, no model call)

When a trail has a session member S:

1. A pull request attached to S joins the trail (`ref` = its URL).
2. An outgoing message drafted in S joins the trail.
3. A note page whose `session_ids` contains S joins the trail.
4. A session whose `fork_parent_session_id` or `parent_session_id` is S joins
   the trail.

- Rules apply transitively (a chat forked from an auto-added chat joins too)
  within one sweep, until nothing new is found.
- A record on another trail, or dismissed for this trail, is skipped.
- Each automatic add is reported once: the sweep returns it and the
  `trails/changed` notification carries it in `added` with the trail id, trail
  title, kind, ref and a label. A second sweep with no new links reports
  nothing.
- A maintenance loop runs the sweep at most once per 1.5 s after a relevant
  core event (PR links changed, checks changed, outgoing changed, notes page
  changed, state changed) and once at start.
- (Review, PR #993) When the batch held a member-state event (checks, PR
  links, drafts, notes, attention), the loop publishes a plain refetch hint
  even if it also published additions for some trail, so every other trail's
  state is refetched. `trails/list_trails` and
  `trails/read_trail` also sweep before reading.

### Brief

- `preview_trail_brief` returns items in this order: notes, pull requests,
  chats, messages. Each item has `id` (`note:<id>`, `pr:<url>`, `chat:<id>`,
  `message:<id>`), `label`, `tokens` (`ceil(bytes / 4)` of the exact text that
  would be carried), and `defaultOn` (true for all four kinds). `totalTokens`
  sums the default-on items. `budgetTokens` is what a new chat on the default
  128k-token window can carry: 90% of the restoration budget, in tokens.
- Discarded drafts and removed records produce no item. A chat with no stored
  history produces no item.
- Item text: notes carry their Markdown clipped to 12 000 bytes; PRs carry
  number, title, URL, state, check counts and failing check names; a chat
  carries its restoration projection at a 24 000-token window; messages carry
  destination, state and body.
- There is no "full history" item: restoration clips each entry and caps the
  whole carried context, so a full transcript cannot reach the model intact.
  (Contract change during implementation; see the commit that made it.)
- `carry_trail_brief {trailId, sessionId, items}` refuses a session that
  already has a user message, refuses unknown item ids, and refuses a
  selection whose tokens exceed the target chat's budget (90% of
  `restoration_budget_bytes` for that chat's window, in tokens) with a message
  naming both numbers. Otherwise it writes the chosen items in preview order
  as `handoff.brief` forest entries with payload `{text, trailId, briefItem}`
  (no `summary`, so restoration reads the full text). An item longer than the
  per-entry restoration allowance is split on line boundaries into several
  entries, each labelled `part k of n`, so nothing is clipped. It adds the
  session to the trail as a `person` member and returns the number of entries
  written. Carrying zero items still adds the member.
- After a carry, `restoration::checkpoint_context` for the new session
  contains every carried item's text; unchosen items are absent.
- (Review, PR #993) A carry checks every refusal (started chat, unknown item,
  budget, the chat already on another trail) before writing anything, and
  writes the entries, the membership and the audit row in one transaction. A
  refused carry leaves the chat's history unchanged.
- (Review, PR #993) Budget is the serialized cost restoration actually
  charges: for every entry the carry writes (after splitting and part
  labels), the entry text plus `handoff.brief: ` and a newline. An item's
  `tokens` is `ceil(cost / 4)` of its entries at the default window; a carry
  admits a selection only if its cost at the target chat's window fits the
  90% budget. An admitted selection restores every entry it wrote.
- A carry changes nothing about the session's harness, model, access mode,
  worktree, write scope or kind.

### Protocol (minor 1.32)

Methods in the `trails` domain: `list_trails`, `read_trail`, `create_trail`,
`update_trail`, `add_trail_member`, `remove_trail_member`,
`preview_trail_brief`, `carry_trail_brief`. Notification `trails/changed`
`{trailId, added: TrailMemberAdded[]}`. Every params type refuses unknown
fields. Results are typed wire structs built in Rust, never hand-built JSON.

- `create_trail {title, workspaceId?, sessionId?}`: with `sessionId`, that chat
  is the first member and the trail takes its workspace when none is given.
- `update_trail {trailId, title?, archived?}`.
- `list_trails {includeArchived}` returns summaries, needs-you first, then in
  progress, then shipped, each newest `updatedAt` first.
- `read_trail {trailId, refresh}`: `refresh` re-reads PR state from GitHub for
  member PRs before deriving (network, so the desktop shell runs it on a
  blocking thread).

### UI

- Sidebar "Trails" row between Projects and Notes, with a count badge of
  needs-you trails (hidden at zero).
- Trails screen: heading, one-line description, "New trail" button, sections
  "Needs you", "In progress", "Shipped" (zero sections hidden), each row with
  title, workspace name and last event, harness marks, a 4-segment stage bar
  and a status word. A "Show archived" toggle. Empty: one line of copy and
  "New trail".
- New trail: an inline form (title, project picker); Enter creates and opens it.
- Trail page: breadcrumb back to Trails, title (click to rename), chips
  (state, branch of the newest PR, newest PR state, chat count), stage bar,
  "Continue this trail" primary button, an archive action. Timeline grouped by
  day. Right rail: "A new chat starts with" (default-on brief items with their
  tokens), "Open" items, "Agents" (harness chips with chat counts).
- (Review, PR #993) The trail page shows only a response for the trail that
  is selected now and for the newest request; a slower response for a trail
  the person left, or an older request, is dropped, success or error, so
  rename and archive always act on the trail on screen.
- Timeline rows open their record: a chat selects it, a note opens it in
  Notes, a PR opens its URL, a message selects its chat. Each row has a
  remove action; removing an auto member shows no confirm.
- Continue: a dialog with the composer text, the harness picker of the new chat
  composer, and the brief item checkboxes with tokens and a running total
  against `budgetTokens`; Send is disabled over budget.
  Send creates the chat in the trail's workspace (direct when none), carries
  the checked items, then sends the text. The new chat appears on the trail.
- (Review, PR #993) Before creating the chat, Send re-reads the brief; if a
  checked item is gone it stops, names it, refreshes the list and keeps the
  text. If the carry still fails after the chat exists, the first message is
  not sent, the empty chat is archived, and the dialog stays open with the
  error, the text and the remaining selection.
- A chat that is a member shows a "Trail: <title>" chip in its toolbar that
  opens the trail.
- A chat's toolbar menu has "Add to trail…": a picker listing open trails and
  "New trail from this chat".
- On `trails/changed` with `added`, a toast "Added to a trail" names what was
  added and the trail, with "Open trail" and "Undo" (Undo removes the member,
  which records the dismissal). One toast per notification.
- Reachable from a fresh install with the UI alone: the sidebar row, New trail,
  Add to trail and Continue need no setting.

## Unit Tests

Rust (`bridge-core::trails`):

- `store::tests::create_refuses_blank_titles_and_trims`
- `store::tests::a_record_sits_on_one_trail`
- `store::tests::adding_twice_is_a_no_op`
- `store::tests::removing_records_a_dismissal_and_manual_add_clears_it`
- `store::tests::archive_keeps_members_and_touches_no_source_rows`
- `timeline::tests::orders_events_across_sources_with_stable_ties`
- `timeline::tests::gone_records_render_as_removed`
- `timeline::tests::state_needs_you_shipped_in_progress`
- `timeline::tests::stage_is_the_highest_reached`
- `auto::tests::pr_draft_note_and_fork_of_a_member_join`
- `auto::tests::non_member_links_add_nothing`
- `auto::tests::dismissed_and_claimed_records_are_skipped`
- `auto::tests::a_second_sweep_reports_nothing`
- `brief::tests::preview_orders_items_and_counts_tokens`
- `brief::tests::carry_writes_entries_restoration_reads`
- `brief::tests::carry_refuses_a_started_chat_and_unknown_items`
- `brief::tests::carry_refuses_over_budget_and_splits_long_items`
- `brief::tests::carry_changes_no_session_policy_fields`

Protocol (`bridge-protocol`): every trails params type refuses unknown fields;
`TrailMemberRef` and `TrailEvent` round-trip camelCase.

Frontend (Vitest):

- `trails.test.ts`: grouping and ordering of summaries, stage labels, day
  grouping of events, brief total for a selection.
- `TrailsScreen.test.tsx` (jsdom): sections render with counts, empty state,
  New trail form submits, row opens the page.
- `TrailPage.test.tsx` (jsdom): every event kind renders, remove calls the
  API, Continue opens the dialog, unticking an item changes the total and the
  carried ids.
- `BridgeSidebar` test: Trails row shows the needs-you badge.

## Integration / Functional Tests

- `api::tests` (bridge-core): `create_trail` with a session, attach a PR row to
  that session, `read_trail` returns the PR as an auto member and a
  `TrailsChanged` event with it in `added`.
- `bridged` dispatch: `trails/list_trails` and `trails/read_trail` decode and
  answer through `dispatch`.
- Both hosts start the trail maintenance loop (source check like workflows).
- Protocol artifacts regenerate with no drift; registry and Tauri handlers stay
  1:1 (`bridge-deck` tests).

## Smoke Tests

- `bun run build` and `bun run test` green.
- `bun run dev` mock mode: sidebar shows Trails with a badge; the list shows
  the mock trails; a trail page renders the timeline; Continue opens the
  dialog.

## E2E Tests

N/A as automated tests: there is no E2E harness for the desktop app. Covered
by the manual run below against a real daemon.

## Manual / cURL Tests

Against a built `bridged` on a scratch data dir (never the live DB):

1. `create_trail` with an existing chat, `read_trail`: the chat is on the
   timeline.
2. Attach a PR to that chat (`github/github_attach_pr`), wait 2 s, `read_trail`:
   the PR is a member with `addedBy: auto`.
3. `remove_trail_member` the PR, attach again: it stays off.
4. `preview_trail_brief`, then create a chat and `carry_trail_brief` with two
   items, then read that chat's forest: two `handoff.brief` entries.

Screenshots of the list, the page, and the Continue dialog in dark and light,
taken from mock mode, go in the PR body.
