# Test contract: reviewer activity and reviewer settings in the Reviews app

Branch `feat/review-activity-and-models`, issue #1159. Locked before implementation.

## Behaviour

1. **Recording.** `SessionStart.activity: Option<&dyn ActivitySink>`. The reviewer turn
   records, in order:
   - `prompt`: the text Bridge sent (the review turn, and the repair turn);
   - `message`: each assistant message once, its final text (a `message.completed`, or the
     accumulated deltas of an item that never completed, flushed at turn end);
   - `tool` / `command`: each `tool.started` / `command.started` invocation once (same item
     id once), title = the tool name or command, text = a short detail (a path, pattern,
     query or the command line) when the event carries one;
   - `denied`: each refused permission request or question;
   - `error`: a provider error that ends the turn.
   With no sink, nothing changes.
2. **Store.** Migration 97: `review_activity(job_id, pass_id, seq, kind, title, text, at)`,
   `seq` from 1 per pass. At most 500 entries per pass (later ones are dropped, and the last
   kept entry says the log was cut), text cut to 4,000 characters. Rows go with their pass.
3. **Publishing.** Each recorded entry is stored and published as `reviews/activity`
   `{jobId, entry}` (transient). `reviews/review_activity {jobId}` returns every entry of the
   job's passes, pass by pass in slot and attempt order, each in seq order. Protocol 1.57; a
   1.56 client still pairs.
4. **Dashboard detail.** An Activity view: one thread per pass, headed by its harness and
   status. Bridge's prompt is a collapsed bubble, assistant messages render as Markdown, tools
   and commands are compact rows, refusals and errors are marked. While the job runs, new
   entries arrive live (from `reviews/activity` for that job) and a "working" row shows. A
   running job's detail opens on Activity; a drafted one on Findings. Empty: "Nothing yet:
   the reviewer's messages and tool calls appear here as it works."
5. **Settings in the Reviews app.** An Overview / Settings switch. Settings holds, per
   harness, model and effort; the reviewer instructions; and the review job settings (post
   mode, auto-review, skip drafts, incremental, second opinion, default and fallback
   reviewers, reviews at once, checkouts). The Workers settings page and the settings search
   no longer have them.
6. **Pane picker.** Each harness row shows its configured model and effort (or "default").

## Tests

Rust:
- `session_tests`: `activity_records_prompt_messages_tools_and_refusals` (scripted transport:
  kinds and order, tool details, one entry per invocation, deltas flushed at turn end, the
  repair prompt), `no_sink_records_nothing`.
- `store_tests`: `activity_is_capped_and_cut_per_pass` (500 and the cut note, 4,000 chars,
  seq per pass, order across passes).
- Protocol gates, the notification registry, the shell parity check.

Frontend:
- `ReviewActivity.test.tsx`: renders a pass thread (prompt collapsed, message Markdown, tool
  row), appends a live entry for the job and ignores another job's, shows the working row
  while running, and the empty state.
- `ReviewsScreen.test.tsx`: a running job opens on Activity; the Settings tab saves a Claude
  model and effort through `saveReviewerSettings`.
- `WorkersPage` no longer renders the reviewer settings; settings search has no reviewer
  entries.
- `GitHubPane.test.tsx`: the picker shows a harness's configured model and effort.

Live: a real Claude review through a real `bridged`, watched in the dashboard while it runs,
with the model and effort chosen in the Reviews Settings tab shown on its pass. Screenshots
light and dark.
