# feat/worker-card-redesign — the worker card in a conversation

The card that shows one delegated worker inside the conversation that spawned
it. This contract covers only what the card *says*; the worker control plane
behind it (stop, retry, failover, typed results) is contracted in
[`feat-worker-control-plane.md`](feat-worker-control-plane.md) and
[`feat-worker-result-and-timeline.md`](feat-worker-result-and-timeline.md).

## What was wrong

A finished card opened with a dozen lines of unbroken result prose, printed a
lone "```bridge-worker-result" where its one-line progress note belongs, then
repeated the first sentence of the same prose — truncated — three bands lower,
and closed with two buttons that both led to the same worker.

## The contract

### One fact per band

1. **The objective is the ask, and a result does not replace it.** Folding a
   `delegation.result` onto its spawn merges the result's *data*; the panel's
   text stays the objective the spawn carried. A result's text fills the
   objective only when the spawn had none (an orphan, or truncated history).
2. **The summary is shown once**, read from the typed envelope on
   `worker_runtime.lastResult`, in its own band. It is never also rendered as a
   truncated tail on the metrics strip.
3. **A card never shows the same words twice.** When the objective and the
   summary are the same string, only the summary band renders.
4. Every band below the header is conditional: a worker that has only started
   is a header and an objective, not a scaffold of empty rows.

### Bounded prose

5. The objective is clamped to two lines. The summary is clamped to three, with
   exactly one affordance ("Read the full result") that appears *only* when the
   summary is long enough to have been clamped.
6. Clamping is CSS, not truncation: expanding shows the full text, and the
   untruncated string is always in the DOM for search and copy.

### Wire chatter is not activity

7. A bare machine fence — ```` ```bridge-worker-result ````, ```` ~~~bridge-delegate ````,
   a bare ```` ``` ````, or a bare tag name — is dropped from the progress
   summary and from the feed. It is the envelope, not the work.
8. A fenced line that carries content after the tag (```` ```rust\nlet x = 1; ````)
   is prose to a reader and survives.

### One way in

9. The footer has exactly one action: **Open session**. "Expand" is gone —
   it opened the same worker in an overlay and read as a second, different
   thing the reader had to choose between. `onExpandWorker` no longer reaches
   `AgentConversation` at all; `TasksPane` keeps its own expand affordance,
   which is a different surface with a different job.
10. **Stop** stays in the header, for as long as the worker has not reported,
    and is the only destructive control on the card.

### What the header says

11. Name, status label, harness mark, requested model and effort, retry count,
    elapsed, stop — in that order, with the right-hand cluster in its own
    rhythm so the retry count, the clock and the stop button do not read as one
    unpunctuated string.
12. The harness mark is new: the card carried the model alone, so a failover
    that moved the work to another provider changed nothing a reader could see.
13. Tone is carried by a 7px dot and a small-caps label, never a tinted panel.
    A terminal state is terminal: no pulse, and the clock reports how long the
    work took rather than how long ago it started.

## Covered by

- `src/conversation.test.ts` — the fold keeps the objective; fills an empty one.
- `src/components/workerPanel.test.ts` — `legibleWorkerLine` and the projection's
  sanitisation of the progress summary and the feed.
- `src/components/AgentConversation.test.tsx` — one button, not two; the ask
  survives the outcome; the summary appears exactly once; a long summary is
  clamped behind one affordance.
- `src/api.test.ts` — the demo's typed result carries the `{command, status}`
  test shape the real envelope uses, so mock mode renders the same bands the
  app does.

## Not in this slice

- The unrendered ledger events (`worker.retry.declined`,
  `router.harness_substituted`, `router.harness_quota_exhausted`,
  `router.no_eligible_route`, `capability.harness_disabled`) and the router's
  persisted `selection_reason`, which no protocol method returns yet.
- `MissionControl.tsx`, which still maps `cancelled` to a destructive tone of
  its own rather than reading `workerStatus`.
- A user-facing peek.
