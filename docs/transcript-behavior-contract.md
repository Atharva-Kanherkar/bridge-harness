# Transcript behavior contract

What the conversation surface does, stated once, over the normalized item types
and nothing else.

Bridge draws one transcript for every agent it can drive. The reader is not
supposed to be able to tell, from how the transcript behaves, which agent
produced the turn. Everything below is therefore written in terms of
`ConversationItem` (`src/transcript/item.ts`) and its `type` and `status`.

**No harness name may appear as a condition anywhere in this contract, or in any
code that implements it.** A harness id is display, never behavior; see
[Identity](#identity) at the bottom. `src/transcript/harnessBranchGate.test.ts`
enforces that over every component, with a named allowlist.

Where a provider genuinely says less than another, the gap is closed at the
normalizer in `src-tauri/bridge-core/` (`agent.rs`, `acp_events.rs`) so that the
item the reducer sees is the same shape either way. It is never closed with a
branch in the UI. The four golden fixtures in `src/transcript/fixtures/` are the
proof: one logical turn per harness, reduced and rendered, asserted equal to
each other.

## Thinking

### One component, two states

Transcript-level thinking is drawn by exactly one component (`Reasoning` in
`src/components/AgentConversation.tsx`), and it reads exactly one input: the
item's `status`. It has two states, and no third.

**`streaming`.** The thought is still arriving. The card is open, showing the
text so far, with one animation beside the word `Thinking…`: the
`thinking-shimmer` sweep from `src/index.css`. The sweep is achromatic and says
one thing, "there is more of this coming". It is the only animation the
transcript uses to mean that, and every row that means it draws this same
component rather than a copy of the markup. An assistant reply whose first token
has not landed is that same statement, so it draws the same mark.

**`completed`.** The thought is finished. It collapses to a single summary line
(`Thought for …`, plus the thought's last line as a preview) with the same icon
it had while streaming, so the row does not change identity as it settles.
Collapsed is the default, always: a settled thought is evidence a reader can go
and look at, not something the transcript should keep spending vertical space
on.

**Expanding is a user action.** Nothing expands a settled thought on the
reader's behalf, and nothing re-collapses one they opened.

### What settles a thought

A thought leaves `streaming` when, and only when, one of these arrives:

- its own completion (`thinking.completed`, from a provider's terminal
  reasoning frame),
- the end of the turn (`turn.completed`, `turn.failed`),
- the session going idle (`session.idle`).

The last two exist because a provider can die mid-thought. They are a floor, not
the normal path: a thought that only ever settles at end of turn shimmers for
the length of the turn, which is the bug this contract was written after. Every
adapter Bridge ships emits a terminal reasoning frame for a thought that ended.
ACP has no such frame on the wire, so `acp_events.rs` assembles one
(`AcpThoughtRun`) at the first non-thought update after a run of thought chunks,
carrying the run's accumulated text under the same item id its deltas carried.

The consequence that matters to a reader: a thought is persisted. The session
forest refuses to store a `.delta`, so a thought that only ever existed as
deltas is a thought that disappears on reload. A thought that ends properly is
one card live and the same card after a reload.

### What is not the thinking presentation

Two other marks exist, they mean different things, and they are deliberately not
folded into the above:

- **`PulseDot`** means *this item is in progress*: a live run of tool work, a
  plan step being executed, a worker still going. It is a state of an item, not
  a statement about the model's attention.
- **The startup narration row** (`StartupStatusRow`, driven by
  `src/startupNarration.ts`) is session-level, not item-level. It says what
  Bridge itself is waiting on before any item exists: spawning the harness,
  waiting on a handshake, opening a session, switching models. It is driven by
  session phase rather than by a `ConversationItem`, and it unmounts as the
  first item starts streaming, handing off to the thinking presentation above.
  A surface outside the transcript (the composer, the sidebar, the worker
  roster) may have its own indicator for its own state; those are not the
  transcript's thinking presentation and are not governed by this section.

## Grouping and density

The grouping algorithm itself is not specified here, deliberately: it is one
implementation of these rules and free to change. What follows are the
invariants any grouping has to satisfy, so that a change can be checked against
something.

1. **Grouping is a function of the items alone.** Specifically: of an item's
   `type`, its position in the turn, and its `status`. Not of the harness, not
   of a wire kind, not of a provider payload field. Two harnesses whose turns
   reduce to the same items must group identically.
2. **A run of consecutive tool work is one group.** Reads, searches, commands
   and edits that follow each other with nothing else between them fold into
   one row, whatever mixture of verbs they are.
3. **A group states what happened, not how many frames arrived.** The summary
   counts work (files read, commands run), and the trailer reports a wall clock
   span, not a sum of overlapping call durations. A provider that emits ten
   progress frames per call must not read as ten times busier than one that
   emits one. Where a protocol reports no durations at all, the trailer is
   absent rather than zero, on the same rule as the exit code: a field the
   protocol does not have is not reported as a value.
4. **A completed group is collapsed**, and expansion is an explicit user action.
   A user's choice outranks liveness: a group the reader collapsed stays
   collapsed while it is still running, and one they opened stays open after it
   finishes. One exception, deliberate: a group holding a patch opens itself,
   because a diff the reader has to go digging for is not an inline diff.
5. **A live group is legible while it is live.** It says what is happening now,
   and it settles into its summary without the row changing identity or the
   scroll position jumping.
6. **An interleaved thought does not shatter a run.** A thought between two tool
   calls is a thought, not a boundary: the tool work either side of it belongs to
   one run of work.
7. **Nothing disappears into a group.** An approval, a question, an error, a
   plan, a delegation, a checkpoint and a diff each keep their own row. Only
   ordinary tool work folds.
8. **A frame Bridge has no name for is visible.** It renders as a collapsed raw
   card, never as an anonymous tool row, and never as nothing.

Invariant 6 holds. The rules live in `src/transcript/grouping.ts`, as pure data
with no React in them: a turn is the outer bound of a run, a thought or a plan
update that falls between two tool calls travels with the run rather than
closing it, and anything the reader has to read or answer ends the run and takes
its own row. An earlier implementation flushed the open group the moment a
thought arrived, so an agent that thinks between every pair of calls shattered a
long turn into one-call groups; that is the failure this invariant is written
down to prevent.

## Stream states

`ConversationItem.status` is a small open vocabulary. What each value means, and
what settles it, per item type:

| Item type | Values | Meaning | Settled by |
| --- | --- | --- | --- |
| `message` | `streaming`, then the provider's terminal status (normally `completed`) | text is still arriving / the message is final | the message's own completion frame |
| `reasoning` | `streaming`, `completed` | the thought is still arriving / it is final | its completion, the end of the turn, or the session going idle |
| `activity`, `diff` | `pending`, `inProgress`, `completed`, `failed` | the call is queued / running / finished / errored | the call's own completion frame, by item id |
| `approval`, `permission`, `question` | `pending`, then the decision | waiting on the reader / answered | the resolution frame naming the request's event id |
| `plan` | the provider's status | the newest plan is the plan | replaced wholesale by the next plan |
| `error` | `failed` | the turn hit something it could not continue past | nothing; an error is terminal |
| `delegation` | the worker's status | the worker's own lifecycle | the worker's result |
| `checkpoint`, `compaction`, `branch-summary` | the operation's status | durable history events | their own completion or failure |
| `raw` | none | a frame kept for inspection | nothing |

Three rules over that table:

- **Only thinking is settled by the turn.** `turn.*`, `session.idle` and
  `usage.updated` render nothing themselves, and the only state they change is
  a streaming thought's. A tool call left `inProgress` when a turn ends stays
  `inProgress`: that is a true statement about a call that never reported back,
  and overwriting it with `completed` would be a lie the reader cannot detect.
- **`streaming` and `inProgress` are not synonyms.** `streaming` means text is
  accumulating into this item. `inProgress` means an operation is running.
  A reader sees the shimmer for the first and a pulse for the second.
- **A status is never inferred from text.** An item is `failed` because a frame
  said so, not because its body contains the word "error".

## Identity

The harness id is display, and only display.

- **The id space is open.** `harnessLabel` (`src/utils.ts`) shows an agent
  Bridge has no bespoke label for under the id it was installed by, rather than
  relabelling it. A new ACP agent is a first-class agent the day it is
  installed, with no code change here.
- **The mark is never load bearing.** `src/components/harnessMarks.tsx` (lines 5
  to 9) states it: every row that shows a mark also names the harness beside it,
  so a session on an agent Bridge has no figure for reads exactly the same. A
  mark is a nicety for the built-ins, not a requirement for the fourth agent.
- **No behavior hangs off it.** Not a row's shape, not its grouping, not its
  animation, not its collapse state, not what it is called in the transcript.
  Anything a provider does differently is normalized in Rust and named on the
  typed event, so the difference is a field the UI can read rather than a
  provider the UI has to recognize.
- **Identity for merging is not the harness either.** Live and durable
  projections of the same item are matched on `ConversationItem.identity`
  (the provider item id, or an approval/request/question id), never on a
  per-store autoincrement and never on which agent sent it.

## See also

- [`testing/feat-issue-488-normalized-events.md`](../testing/feat-issue-488-normalized-events.md),
  the event-normalization contract this one is the visible half of.
- [`src/transcript/fixtures/README.md`](../src/transcript/fixtures/README.md),
  the four golden streams and what genuinely differs between them.
- [`docs/session-forest.md`](session-forest.md), for what is durable and what is
  only ever live.
