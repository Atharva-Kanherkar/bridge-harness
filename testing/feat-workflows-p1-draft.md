# feat/workflows-p1-draft: Test Contract

Slice P1.5 of Workflows. The phase contract (`testing/feat-workflows-phase-1.md`) is the
authority; this file names what this branch must prove.

## Functional Behavior

Phase contract C14, F30 to F33 and F32a, plus:

- `workflows/draft_workflow_from_text { text, projectId? }` asks a tool-less Claude turn for a
  JSON draft, then a deterministic validator builds the `WorkflowDefinition`: servers only
  from Claude's configuration (an unknown or unconnected one becomes a "connect X first"
  note), writes only under F32a, the schedule parsed or turned into Run now, the name trimmed,
  the instructions defaulting to the sentence, the agent Claude with its fast default model,
  `files: "none"`. Nothing is saved.
- `workflows/draft_workflow_from_session { sessionId }` reads the chat: its first user message
  becomes the instructions, its title the name, its harness and model the step; grants come
  from the chat's `tool.started` entries mapped back to configured servers. No model call.
  Worker sessions and workflow runs are refused.
- Both return `{ draft: WorkflowDraft }` with `definition` and `notes`.
- UI: a "Describe a job" bar on the Workflows page drafts and opens the editor prefilled with
  the notes shown; "Save as workflow" in the sidebar chat menu drafts from that chat and opens
  the Workflows page with the editor prefilled.
- The drafting model is behind a `DraftModel` trait so tests use a scripted one.

## Unit Tests

Phase contract U-D1 to U-D5, plus:

- `the_draft_parser_takes_only_the_json_object_it_asked_for`
- `a_draft_never_names_a_server_claude_is_not_configured_with`

## Integration / Functional Tests

- I-2 replay: `testing/fixtures/workflow-drafts/cases.json` holds 30 sentences, each with
  the servers available, a recorded model reply and the expected trigger and grants. Replay
  through the validator meets the C14 bar: zero over-grants, at least 27 of 30 triggers exact,
  at most 3 under-grants.
- The recorded replies come from an ignored live test that calls the real model and prints
  the same metrics, run by hand before merge.
- vitest: the describe bar opens a prefilled editor and saves nothing until confirmed (V-6).

## Smoke Tests

- Deck parity, protocol drift and naming gates stay green with the two new methods.

## E2E Tests

Covered by P1.7.

## Manual / CLI Tests

- In mock mode, describe a job and see the prefilled editor with its notes; use Save as
  workflow on a chat from the sidebar menu.
