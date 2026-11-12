# feat/adaptive-chat-titles: test contract

A chat title in the rail is cut with an ellipsis when the rail is narrow and
leaves space unused when it is wide. Titles are cheap, so the title job now
writes each automatic title as a ladder: the same name at five or so lengths.
The rail shows the longest rung that fits its width. Dragging it narrower
rewords "Why the SQLite WAL keeps growing in bridged" to "SQLite WAL growth",
then "WAL growth", instead of cutting it to "Why the SQLite W…".

Builds on `feat-chat-titles.md`; the precedence table there is unchanged.

## Functional Behavior

### The ladder

- Written in the same hidden turn as the title. The reply's JSON gains
  `"ladder": [...]` per chat: five rungs, shortest first, from one or two words
  to seven to ten, all naming the same task.
- Stored on `sessions` (migration 69) as `title_ladder` (JSON array) and
  `title_ladder_for` (the title it was written for).
- Served as `Session.titleLadder` only while `title_ladder_for` equals the stored
  `title`. Any rename (the user's, or Claude renaming as a chat moves on) stops
  serving the old ladder, without having to touch every place a title is written.
- A user's title (no source) never gets a ladder. It is shown as written.

### Guards (`title_jobs::ladder`)

- The title is always a rung. Rungs are cleaned like titles (quotes, markdown,
  `Title:`, trailing punctuation) and sorted by length; case-insensitive
  duplicates collapse.
- A rung is dropped when it is longer than 12 words or 80 characters, is a
  placeholder, a reply opener ("Sure", "I …"), a redacted secret, or names
  something else. "Same subject" means it shares a topic stem with the title,
  is made only of the title's own words ("UI" for "UI polish"), or is written
  in a script without spaces (Japanese, Chinese, Thai).
- Fewer than 3 rungs (title included): no ladder is stored. More than 7: the
  rung closest in length to the one below it goes first. The title stays.

### Queue

- At store open, chats with an automatic title (`generated`, `provider`) and no
  ladder for it are queued. That covers chats titled before ladders existed.
  A finished job (`generated`, `provider`, `unchanged`, `ladder`) is reopened
  once for this. A ladder attempt ends `ladder` or `error`, so it is not
  reopened forever.
- After a turn, a finished job is re-armed when the chat needs a ladder (Claude
  renamed it). A `failed` job stays failed.
- A chat that is named already goes to the model with `<current_title>`, which
  is inert like every other excerpt. Its title is kept and only the ladder is
  stored. Outcome `ladder`.
- Claude's own title, found in the transcript, is committed and then laddered
  in the same batch. Outcome stays `provider`.
- A rejected title ladder still lets the title land. A rejected ladder for a
  named chat counts a failed attempt and keeps the title.
- A chat renamed by hand while queued is settled `unchanged`, with no ladder.

### Rail (`FittedTitle`)

- Picks the longest rung whose measured width fits the title's box, or the
  narrowest when none fits (then truncated as before). Width is measured with
  a canvas in the element's own computed font, plus letter-spacing. Measurement
  is cached and redone when web fonts finish loading.
- A row with no ladder renders its title exactly as before.
- Where text cannot be measured (no `OffscreenCanvas`), the title is shown.
- One shared `ResizeObserver` for every row. The first fit happens in a layout
  effect, so a row never paints its title and then rewords itself.
- Motion: when the rung changes, only the words it adds animate in (opacity,
  blur, 0.2em rise, 35 ms stagger). Shared words stay still. A row that mounts
  already fitted does not animate. A ladder arriving for a row on screen does.
  Reduced motion: no animation, no delay.
- The row's hover names the chat by its longest rung. The rail's search
  matches every rung.

## Tests

- Rust `title_jobs::tests`: ladder ordering and title inclusion; dropped rungs
  (answers, other subjects, placeholders, secrets, too long); abbreviation and
  CJK pass; too few or too many rungs; ladder parsing with and without the key;
  `<current_title>` inert; `store_ladder` refuses a stale title; queue backfill
  of named chats is one-shot; post-turn re-arm after a rename, never for a
  user's title; drain paths: new chat, named chat, Claude's title, rejected
  ladder for each, hand rename while queued.
- Rust `store::tests::a_title_ladder_is_served_only_while_it_belongs_to_the_title`.
- `protocol_mirror` and protocol artifacts include `titleLadder`.
- Vitest `FittedTitle.test.tsx`: `pickRung` width choice, order independence,
  no measurement; `addedWords`; mount without animation, resize animates only
  added words, late ladder animates, single-rung title untouched and not
  observed, no-canvas fallback.
- Vitest `sidebarChats.test.ts`: search matches a longer rung.

## Eval

`BRIDGE_TITLE_EVAL=1 cargo test -p bridge-core --lib title_jobs::eval -- --ignored --nocapture`
(set `BRIDGE_CLAUDE_SIDECAR` to a sidecar with `node_modules` when running
from a worktree). 27 cases, 3 of them already named. Gates per harness:

| metric | gate |
|---|---|
| title on topic | ≥ 90% |
| ladder stored | ≥ 90% |
| spread: shortest ≤ 16 columns, longest ≥ 36 (CJK counts 2) | ≥ 80% |
| every rung of 3+ words on topic | ≥ 85% |
| raw rungs rejected by the guard | ≤ 10% |
| injection (title and every rung) | held |
