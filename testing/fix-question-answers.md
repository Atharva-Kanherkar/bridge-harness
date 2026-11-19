# fix-question-answers: agent questions are never auto-answered

## Contract

1. Claude `AskUserQuestion` arriving on `can_use_tool` is persisted as `question.requested` (requestMethod `claude.askUserQuestion`), never `permission.requested`.
2. Full access ("Auto-approve provider permissions") never answers a question; the session goes `waiting`.
3. Answering sends `control_response` `{behavior: allow, answers: {<question text>: <label[, label]>}, annotations?}`; the sidecar returns it as the tool's `updatedInput`.
4. Skipping denies the tool with a message telling Claude the person declined, not that they were silent.
5. The resolved entry records `submittedAnswers` for every non-secret field; the card shows them after reload.
6. The question card answers every shape: Claude/OpenCode choices (single, multi, descriptions, previews, typed Other), Codex `requestUserInput` (free text, secret, isOther), MCP elicitation (enum, boolean, number/integer, array, defaults, required).
7. Elicitation answers are typed by schema (boolean, number, array).
8. The attention banner for a question reads "<Harness> has a question" with the question text.
9. A Claude question left open when the turn stops, errors or the session ends is settled, so no stale card stays answerable.

## Tests

- `agent::tests::claude_ask_user_question_is_a_question_not_a_permission` (1)
- `live_turn::permission_policy_tests::full_access_never_answers_a_claude_question_and_the_card_does` (1, 2, 3, 5)
- `claude_adapter::tests::a_question_answer_rides_the_control_channel_as_answers` (3, 4)
- `api::question_reply_tests::*` (3, 5, 7)
- `attention::tests::a_question_banner_says_what_is_asked`, `src/attentionCopy.test.ts` (8)
- `sidecar/claude-agent/test/subagents.test.mjs` "AskUserQuestion answers reach Claude as updatedInput" (3, 4)
- `src/questionFields.test.ts`, `src/components/QuestionCard.test.tsx` (5, 6)
- `void_orphaned_questions` now covers `claude.askUserQuestion` (9)

Renders: `testing/screenshots/fix-question-answers/`.
