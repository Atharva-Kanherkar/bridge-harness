# feat/dock-transcript — test contract

Locked before implementation. One workstream: the transcript pane — the raw
normalized event stream and the session forest behind the rendered chat,
inspectable beside the conversation. The rendered view is a lossy
projection; when it and reality disagree, this pane is where the
disagreement becomes visible. It reads the same durable data the UI reads,
adds no storage, and changes nothing.

## Shape of the thing

`src/components/TranscriptPane.tsx` renders one pane with two views behind a
segmented control, "Stream" and "Entries". The stream is the normalized
event sequence: sequence number, kind, status, and time per row, ascending,
tailing new events while a turn runs. The first page arrives through a
loader prop backed by replaySessionEvents with its tail flag; "Load
earlier" pages backward until sequence 1 is on screen; live events merge in
by id, deduplicated and ordered by sequence. A kind/text filter narrows the
loaded rows client-side — entry search stays with the existing recall
surface. Selecting a row opens its raw payload as pretty JSON with a copy
affordance, and the filtered stream can be copied as one JSON array. The
Entries view lists the forest: kind, short id, and context visibility per
entry, the active branch marked by walking parent links from the active
entry, a summary line naming the head and leaf count, and a per-entry
reveal that hands the entry id back to the host — the conversation sits
beside the dock, so reveal scrolls and highlights rather than navigates.
In App, the pane joins the dock for every session — a transcript needs no
repository — and the mock replay gains fidelity: per-session slices with
afterSequence, limit, and tail honoured, instead of an empty array.

## 1. The stream — `src/components/TranscriptPane.test.tsx`

| # | Behaviour | Assertion |
|---|---|---|
| 1.1 | The first page loads through the tail loader | on mount the loader is called once with the tail request; its events render as rows with sequence, kind, and time |
| 1.2 | Live events merge without duplication | an event already loaded by id renders once; a new one appends in sequence order |
| 1.3 | Earlier pages load backward | with a full first page, "Load earlier" is offered and fetches the window before the oldest loaded sequence; at sequence 1 it is not offered |
| 1.4 | The filter narrows loaded rows | typing into the filter leaves only rows whose kind or text matches; clearing restores |
| 1.5 | The filtered stream copies as JSON | the copy-stream affordance writes a parseable JSON array of the visible events |

## 2. The row inspector — same file

| # | Behaviour | Assertion |
|---|---|---|
| 2.1 | Selecting a row opens its raw payload | the JSON block contains the event's kind and its data fields verbatim |
| 2.2 | The payload copies | the row copy affordance writes parseable JSON for that event |
| 2.3 | Selecting another row swaps the inspector | one inspector at a time |

## 3. Entries and branches — same file

| # | Behaviour | Assertion |
|---|---|---|
| 3.1 | The segmented control swaps views | "Stream" and "Entries" render their own lists |
| 3.2 | Entries show their forest identity | kind, short id, and context visibility per row |
| 3.3 | The active branch is marked | entries on the parent chain of the active entry carry the active marker; entries off it do not |
| 3.4 | The branch summary states head and leaves | the head's short id and the leaf count render |
| 3.5 | Reveal hands the entry back to the host | the per-entry reveal calls the callback with that entry id |

## 4. App wiring — `src/App.test.tsx` (extended)

| # | Behaviour | Assertion |
|---|---|---|
| 4.1 | The transcript joins the dock for repo sessions | the fourth pane chord opens it; the seeded mock events for the session render as stream rows |
| 4.2 | A direct chat has a transcript too | in a direct chat the transcript pane opens with content, not an unavailable frame |
| 4.3 | Reveal scrolls the conversation | revealing an entry scrolls to and highlights the rendered item (scrollIntoView observed on the entry's element) |
| 4.4 | The mock replay serves per-session slices | replaySessionEvents outside Tauri returns that session's events honouring afterSequence, limit, and tail |

Explicitly **not** changed: the conversation renderer and its projection;
SessionRecallSearch as the entry-search surface; the session forest data
itself (this pane reads, never writes); the policy engine; the contracts in
testing/feat-dock-shell.md, feat-dock-changes.md, and feat-dock-code.md.
Payload JSON renders raw — provider-reported, measured, and estimated
values keep whatever labels the data carries. The design-system guard
stays green with no new allowlist entries.
