# Contract: agents can ask for the person (attention/post, notify_user)

Branch `feat/attention-post`, stacked on `feat/attention-web-banners`.
Implements #913, slice 9 of #904. Locked before implementation.

## What this changes

An agent that hits a decision only the person can make, or a blocker it
cannot get past, can only write it into the transcript and hope someone
reads it. This gives it a narrow, rate-limited way to raise a notice: a
`posted` row in the attention ledger, which every client shows like any
other notice and which opens the chat.

## Functional behavior

Core (`attention::post`):

- `post(db, session_id, title, body, severity, now)` writes one `posted` row
  for the person (`audience` human) and returns its sequence, or says it was
  rate limited.
- Sanitizing, like herdr: control characters are stripped, whitespace runs
  collapse to one space, both ends are trimmed; the title is cut to 80
  characters and the body to 240 (on character, not byte, boundaries). An
  empty title after that is refused with `invalid` (1000), the code every
  core validation uses; `invalid_params` stays for malformed params such as
  unknown fields. An empty body is stored as none.
- Rate limit: at most 3 posts per session in any 5 minutes. The fourth is
  refused as `rateLimited` and writes nothing.
- The session must exist and not be archived; a hidden kind (briefing,
  connector run, …) may not post.
- The row supersedes the chat's older unseen rows, like every other kind, and
  publishes `attention/changed`.
- Posting grants nothing: it is text and a link to the chat.

Protocol 1.29:

- `attention/post { sessionId, title, body?, severity? } -> { shown, reason }`,
  strict params. `severity` is `informational` (default) or `actionable`.
  `reason` is `shown` or `rateLimited` (with the `other` fallback).
- `AttentionKind` gains `posted`. A 1.28 client decodes it as `other` and
  shows nothing, as `escalation` did for 1.24, so it still pairs.
- `sessionId` is required in this slice: a post with no chat would have
  nothing to open, and clients skip session-less rows.

MCP tool `notify_user` on the existing Bridge tool server:

- Listed for top-level chats only (the same `Write` access as `notes_create`);
  a worker asks through its orchestrator.
- Arguments `{ title, body?, actionable? }`; the result says `shown` or
  `rateLimited`. The description says it is for decisions and blockers, not
  progress or results.
- Scoped to the calling chat by the route's per-chat token; it cannot post
  for another chat.

Desktop:

- A `posted` row shows a toast with the row's own title and body; it opens
  the chat. Actionable posts use the request tone, sound and 8 s; others the
  finished-turn tone, sound and 5 s.
- Settings → Notifications gains "An agent asks for you", default toast,
  banner and sound on.
- Unread: a `posted` row counts. In the rollup it ranks below `needsYou` and
  `escalation` and above `failed` and `finished`; `needsYou` in the summary
  still counts only requests and stranded results.

Out of scope: posting to other chats or other people; a `sound` argument
(the person's settings decide); "disabled" or "no client" reasons.

## Unit tests (bridge-core)

- `attention::tests::post_sanitizes_and_truncates`
- `attention::tests::post_refuses_an_empty_title`
- `attention::tests::the_fourth_post_in_five_minutes_is_rate_limited`
- `attention::tests::posts_need_a_live_visible_chat`
- `attention::tests::a_post_counts_as_unread_below_requests`
- `notes::tool` route: `notify_user` posts for the token's chat, is listed
  for a top-level chat and not for a worker, and a fourth call says
  `rateLimited`.
- `mcp_apps::server`: `notify_user` is forwarded to the core like the memory
  tools.

## Integration / functional tests

- bridged dispatch: `attention/post` writes a row `list_attention` returns;
  an empty title is `invalid`; unknown fields are `invalid_params`.
- Protocol gates, a handshake test for 1.29, and a 1.28 client decoding
  `posted` as `other`.

## Frontend tests

- `attentionCopy.test.ts`: posted copy comes from the row.
- `attentionPolicy.test.ts`: posted durations by severity; the posted switch.
- `App.attention.test.tsx`: a posted row toasts with its title and opens the
  chat.

## Smoke tests

- `cargo test -p bridge-core attention`, `cargo test -p bridge-protocol`,
  `cargo test -p bridged dispatch`
- `bun run check`, `bun run build`, `bun run test`

## E2E / Manual

Live daemon from this branch: `bridge exec --method attention/post` for a
seeded chat returns `shown` three times and `rateLimited` the fourth, and
`list_attention` holds three `posted` rows.
