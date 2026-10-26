# fix/agent-starting-state — Test Contract

A worker the orchestrator just delegated opens in the Agents pane before its
first message arrives. Today that view says "Say the word." (the new-chat
greeting), labels the worker "Research · standard", and the footer claims
"This worker has finished" while the header says STARTING.

## Functional Behavior

- **Topic.** When Bridge reserves a worker session it stores a title cut from
  the delegation `objective`: the first non-empty line, first sentence, at most
  60 characters on a word boundary with `…` when cut. An empty objective stores
  no title. The orchestrator's objective is what names the work.
- **Name.** Every worker gets a stable personal name derived from its session
  id (same id → same name, across renders and reloads). No backend storage.
- **Header and rows.** The Agents pane shows the name in bold, then the role
  (`Research`, from the label) muted, then the status. The line below is the
  topic. The open-agent header keeps the harness · model · family mono line.
  A worker with no topic falls back to its label for that line.
- **Starting state.** A worker that is not terminal and has no transcript yet
  shows a spinner with "<Name> is starting", the topic, and "Waiting for its
  first message", never the chat greeting.
- **Ended empty.** A terminal worker with no transcript shows "No messages
  recorded", never the chat greeting.
- **Footer.** While a worker is starting / resuming / restored (active tone but
  not steerable) the footer says it is starting and steering opens once it is
  running. "This worker has finished" only appears for a worker that has
  reported or reached a terminal lifecycle.
- Ordinary chats keep the greeting unchanged.

## Unit Tests

- Rust `delegation::tests::worker_title_*`: first sentence, 60-char word cut
  with ellipsis, multi-line objective, blank objective → None.
- Rust `live_turn` reservation test: the reserved worker row has `title` set
  from the objective.
- `src/components/agentIdentity.test.ts`: `agentName` is deterministic and
  differs across ids; `agentRole("Research · standard") === "Research"`;
  `agentTopic` returns the title, or undefined when the title equals the label.

## Integration / Functional Tests

- `AgentChat.test.tsx`: starting worker with empty forest → spinner status with
  name + topic, no "Say the word", footer says starting, not finished.
- `AgentChat.test.tsx`: finished worker with empty forest → "No messages
  recorded", no greeting.
- `AgentChat.test.tsx`: existing "finished notice once reported" still holds.
- `TasksPane` tests: row and header show name, role and topic.

## Smoke Tests

- `bun run check`, `bun run build`, `bun run test` green.

## E2E Tests

N/A — no E2E harness covers live delegation; covered by the manual step below.

## Manual / cURL Tests

- `bun run dev` (mock data): open the Agents pane on a starting worker and
  confirm the spinner, name, topic and starting footer in dark and light.
