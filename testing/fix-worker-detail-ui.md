# fix/worker-detail-ui: Test Contract

Opening a worker in the Agents pane showed a view that read as a different app
from the lineage row it came from: a pulsing green dot, an all-caps status, a
raw model id in monospace that repeated the role, a native `▶` disclosure, cards
drawn as brackets, a permission card that printed its command twice, the same
permission card drawn twice, and a steer box whose one-line textarea showed
scrollbars.

## Functional Behavior

- **Header.** Name in bold, role muted, topic below. The third line is the
  worker state in sentence case with the same icon and colour its lineage row
  uses (spinner for working), then `harness · model label · elapsed` in the
  sans face. The task family is shown only when it differs from the role. No
  status dot, no pulse, no monospace.
- **Diagnostics.** The worker's reasons disclosure is a chevron row in meta
  type, newest reason first with a count; no browser disclosure triangle.
- **Cards.** Transcript notices and decision cards (permission, approval,
  question, prompt change, verification, errors) have one even 1px border.
  Failures keep a red outline and wash. No `border-x-2` side accents anywhere.
- **Permission command.** When the request text is the command itself, the
  card shows it once, in the code well. Resolution copy joins the reason with
  ` · `, never an em dash.
- **One card per permission.** A permission's live frame and its stored entry
  merge into one row. Two permissions that share a sidecar request id (a
  restarted sidecar counts from 1 again) stay two rows. A permission row never
  takes the identity of the tool row it is about.
- **Steer box.** One rounded field with the Steer button inside it. The
  textarea grows with the draft up to about five lines, then scrolls; at one
  line it shows no scrollbar. Helper copy fits one line in the dock. The
  finished notice has no em dash.

## Unit Tests

- `conversation.test.ts`: live + stored permission for the same call merge to
  one permission and one tool row; the permission's identity is not the tool's.
- `conversation.test.ts`: two permissions with the same request id on different
  calls stay two rows.

## Integration / Functional Tests

- `TasksPane.test.tsx`: open header says `Starting` (not `STARTING`), shows
  `Codex · m`, does not repeat the family, has no `.font-mono` and no
  `.mission-live-accent`.
- `AgentConversation.test.tsx`: verification card has no `border-x-2`; policy
  resolution reads `by policy · Auto-approve provider permissions` with no em
  dash; a permission whose text equals its command prints the command once.
- `SteerComposer.test.tsx`, `AgentChat.test.tsx`: existing steer behaviour
  (send, refuse empty, keep draft on failure, IME, starting and finished
  notices) still holds.

## Smoke Tests

- Dark and light screenshots of the open worker view with diagnostics, a
  degraded-continuation notice, a resolved permission card and the steer box.
