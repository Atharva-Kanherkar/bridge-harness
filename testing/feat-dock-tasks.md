# feat/dock-tasks — test contract

Locked before implementation. One workstream: the background-tasks pane —
one list answering "what is Bridge doing right now, and what did it just
finish?" for the active session's workspace. Workers, queued delegations,
and background shells already exist across Mission Control, the worker
cards, and the terminal strip; the failure mode this pane removes is the
specific expensive one: something finished or failed while you were looking
elsewhere, and nothing said so. The pane displays and routes. It grants
nothing, cancels nothing the backend does not already offer, and never
invents an action the policy engine would refuse.

## Shape of the thing

`src/components/TasksPane.tsx` renders rows from data App already holds.
Worker rows come from the forest's runtime records joined to their sessions
through the existing workerStatus vocabulary — tone, label, detail — with
elapsed activity, retry count, and the reported summary where one exists.
Queue rows state why they wait: the queue status and the request's reason,
never an unexplained pending. A shells row summarises the terminal
activity and routes to the terminal pane. Failed and stalled rows read
differently from everything else and stay until dismissed; dismissal hands
the session id to the host, which keeps the acknowledged set. Rows act
through existing wiring only: open the worker's session, expand its detail
panel, retry a failed worker through the existing retry path. In App the
tasks pane joins the dock last (the sixth chord), available in every
session, and the switcher badge is computed from the forest whether or not
the pane has ever mounted: the count of running workers plus running
shells, and the attention mark while any unacknowledged failure exists.
The unknown-pane persistence fixture moves to a still-unknown id.

## 1. The roster — `src/components/TasksPane.test.tsx`

| # | Behaviour | Assertion |
|---|---|---|
| 1.1 | Worker rows speak the house status vocabulary | a working worker renders WORKING and its task family; a completed one renders DONE with its reported summary |
| 1.2 | Retries stay attributed | a runtime with retries renders the retry count on its row |
| 1.3 | A queued delegation states why | the row carries the queue status and the request reason, with its objective |
| 1.4 | Shells are one row | the terminal activity renders as a running-shells row |
| 1.5 | Nothing running says so | with no workers, queue, or shells, the pane states there is nothing in flight |

## 2. Failures persist until acknowledged — same file

| # | Behaviour | Assertion |
|---|---|---|
| 2.1 | A failure reads differently and stays | a failed worker row carries the failure treatment and a dismiss control; other rows carry none |
| 2.2 | Dismissal is a callback, not a deletion | the dismiss control calls back with the session id; an acknowledged id no longer renders its failed row |

## 3. Rows act through existing wiring only — same file

| # | Behaviour | Assertion |
|---|---|---|
| 3.1 | A worker row opens its session | the open action calls onOpenSession with the worker's session id |
| 3.2 | A worker row expands its detail | the expand action calls onExpandWorker with the session id |
| 3.3 | Only a failed worker offers retry | retry renders on the failed row only and calls onRetryWorker with its session id |
| 3.4 | The shells row routes to the terminal pane | it calls onOpenTerminal, and nothing else |

## 4. App wiring — `src/App.test.tsx` (extended)

| # | Behaviour | Assertion |
|---|---|---|
| 4.1 | The sixth chord opens the pane with the live roster | the seeded workers render: a WORKING implementation worker and a DONE verification worker, and the queued request with its reason |
| 4.2 | The badge tells without the pane ever mounting | in a fresh session view, the tasks descriptor carries the running count from the forest |

Explicitly **not** changed: the policy engine and every approval flow; the
worker retry path; Mission Control and WorkerDetail as the deep surfaces
this pane routes to; the dock shell lifecycle; prior dock contracts except
the named fixture evolution. The design-system guard stays green with no
new allowlist entries.
