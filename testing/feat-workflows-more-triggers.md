# feat/workflows-more-triggers: Test Contract

Locked before implementation. This is the rest of #964: Gmail, Linear and Notion event triggers. Slack and GitHub shipped in #1134, and this reuses its event ledger, anchor rule, per-event and batch modes, queue, fencing and dry runs.

## Decisions

| # | Decision | Why |
| --- | --- | --- |
| T1 | Three events: `gmail_message_received` (a new email in the inbox, not sent by the owner; filter `from`), `linear_issue_assigned` (an open issue is assigned to the owner; filter `channel`, meaning the team), and `notion_page_created` (a page is added to a Notion database; `channel` names the database and is required; filter `from`, meaning the creator). | The most common "when X happens" for each app, and ones a model can read reliably. Notion has no "mentions me" query, and a Linear assignment has no time a read can see. |
| T2 | Trigger-only ingress: these apps get no in-app Inbox. A hidden, bounded, read-only turn on the app's own connector (the ingress policy: scoped to that one server, native connector, no transcript, 6 turns, $0.02) reports the app's newest items as a fenced `bridge-workflow-events` JSON list: the newest 20 inbox emails from the last day, the open issues assigned to the owner (at most 50), or the newest 30 pages of the database. One read serves every workflow listening to the same app and database. | It is what #964 asks for. There is no render turn per item and no new outward action (a Gmail Inbox would mean sending email). |
| T3 | An app (and, for Notion, a database) is read only while an approved, enabled workflow listens for it: every 5 minutes at most, never once the quota reserve is reached, and on a background thread so the workflow tick never waits for it. A failed read is recorded with its reason, and the next one follows at the same cadence. | #964: a poll spends the person's plan. |
| T4 | Each reported item needs the provider's own id and a time. One without either is dropped, never guessed. Keys are `gmail:<message id>`, `linear:<issue id>:assigned` and `notion:<page id>`. Items go to a per-app store (`workflow_ingress_items`, kept 30 days). As with GitHub, a workflow's first read after its listener starts (approval, change, turning on, resume) is a baseline: seen, never run. After that, an item the workflow has not seen goes to its ledger as pending. Gmail and Notion items also need a time after the anchor; a Linear assignment, like a review request, has no time of its own. | One run per event, history never fires, and a replayed poll yields nothing. |
| T5 | The preview for these apps counts that store's last 7 days. Before the first read it says Bridge has not read the app yet, and that it reads it every 5 minutes once a workflow listens. | A live preview would be a paid turn per edit. |
| T6 | Event runs work as before: the first turn carries the event fenced as data, the approval wait is 2 h, and the dry run can try the latest event. GitHub detail enrichment applies to GitHub events only. | Same engine. |
| T7 | Protocol 1.55, which adds the three kinds. A 1.55 daemon refuses older clients, which cannot read the new kinds; a 1.54 daemon refuses a 1.55 client. | As in #1134's R13. |
| T8 | Editor: Gmail, Linear and Notion groups in the event picker, with only that event's filter shown (From for Gmail and Notion, Team for Linear). A Gmail template: "Triage each new email". | Same picker; one way to start. |

## Storage

Migration 94 adds `workflow_ingress_items(family, scope, item_key, kind, title, url, payload, occurred_at, observed_at, PRIMARY KEY(family, scope, item_key))` (rows older than 30 days are pruned on insert) and `workflow_ingress_polls(family, scope, last_attempt_at, last_success_at, last_error, PRIMARY KEY(family, scope))`. `scope` is empty for Gmail and Linear, and the database for Notion.

## Functional behavior

- F1: A Gmail item reported after a listening workflow's anchor creates one event and one run. A replayed poll creates none. The `from` filter holds.
- F2: A Linear assignment with the team filter fires only for that team. A Notion page fires only for its database, and `from` filters its creator. The items present at the first read never fire.
- F3: An item without an id or a time is dropped. One that occurred before the anchor never fires.
- F4: No poll happens with no listener, within 5 minutes of the last one, or with the quota reserve reached. A disconnected app records a failed poll with its reason.
- F5: The preview counts the store and says so before the first read.
- F6: Validation refuses filters an event does not take (a repository on Gmail, a channel on Gmail, a sender on Linear), a Notion trigger without its database, and an event trigger on a harness that cannot hold connectors.
- F7: A 1.54 client is refused by a 1.55 daemon, and the other way round.

## Tests

- Unit (`workflow_ingress`): prompt per app, parsing (dropped items, keys, filters), the store and its pruning, gating (listener, cadence, quota), and a fake turn into the ledger.
- Proptest: parsing never panics on arbitrary JSON, and never yields an item without an id and a time.
- Journey E8 (`bridged/tests/workflows.rs`): a Linear assignment through the real poll path, with the fake SDK answering the ingress prompt, starts one run, and a replay starts none.
- Frontend: the picker groups and filters, the summaries, and the Gmail template.
- Live, scratch `bridged`, real Claude: Gmail and Notion polls through the real connectors (Linear is not signed in here), and one live Gmail-triggered dry run.

## Manual / cURL

N/A.
