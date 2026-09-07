# fix/composer-image-paste-runtime — Test Contract

## Functional Behavior

- Pasting an image into the welcome/new-chat composer shows a removable preview, just as it does in an existing chat.
- Submitting text plus pasted images creates the selected chat and delivers both in its first turn.
- Submitting a pasted image with no text still creates the selected chat and delivers the image.
- Text-only paste and text-only first-message behavior remain unchanged.
- A failed first-message delivery restores the text and pasted images in the newly created chat composer and surfaces the error.

## Unit Tests

- `Welcome` forwards pasted image clipboard items into preview attachments.
- `Welcome` can submit an image-only first message.
- The pending first-message handoff retains attachments until the created session is selected.

## Integration / Functional Tests

- The create-on-first-submit path accepts image-only input and carries attachments to `sessions/submit_input` after session creation.
- Existing-session image paste and send behavior remains green.

## Smoke Tests

- `bun run build` passes.
- `bun run test` passes, including frontend and Rust suites.

## E2E Tests

- N/A — no automated desktop clipboard harness exists in this repository; the React seam and full submit path are covered below and by existing protocol/core tests.

## Manual / cURL Tests

- On the welcome screen with Claude selected, paste a screenshot: a preview appears immediately.
- Press Enter with only the screenshot: a chat is created and the image appears in the first user turn.
- Paste plain text on the welcome screen: the text is inserted normally with no attachment preview.
- Remove the preview before sending: the image is not delivered.
