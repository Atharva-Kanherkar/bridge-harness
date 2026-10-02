# Steer interrupts the turn

Sending a message into a chat while its turn is running means "stop and do this
instead", on every harness (Claude, Codex, OpenCode, Cursor, Grok).

- A chat send mid-turn interrupts the provider in-band, queues the text at the
  front of the session queue, and answers `steeredActiveTurn`. Nothing is handed
  to the provider while the old turn still runs.
- The interrupted turn's own `turn.completed` is the boundary that delivers the
  steer. The error frames the interrupt provokes are dropped and the turn ends as
  `cancelled` ("Redirected"), not as a failure.
- A steer runs before follow-ups already queued (a worker report, say), and
  steers keep their own submission order.
- If the turn has not settled 5 s after the interrupt, Bridge stops it the hard
  way (the Stop button's path, queue kept), resumes the chat, and delivers.
- A checkpoint (compaction) is never cut short: input sent during one queues as
  before.
- Workers are unchanged: a human steer amends the objective (native steering or
  the queue), it never stops the worker.
- Image attachments mid-turn: a provider that folds input natively (Claude)
  still takes them into the live turn; elsewhere they are refused, as before.
- The composer says Steer mid-turn for every chat, and a steer is never shown as
  a queued follow-up.
