# fix/image-only-send: Test Contract

Pasting an image (or attaching an image file) and sending with no text fails
with "Message cannot be empty". The images are the message, so the send must
go through.

## Functional Behavior

- Root cause: `sessions/prepare_turn` refuses empty text, but it runs before
  `sessions/submit_input`, the call that carries the attachments. The UI sends
  an image-only message as `prepare_turn("")`, which fails first.
- `prepare_turn` with empty or whitespace-only text returns the text unchanged
  with no interceptions. It still refuses a session that does not exist.
- `submit_input` stays the single emptiness gate: empty text with at least one
  image starts a turn; empty text with no image is refused with
  "Message cannot be empty".
- Text with secrets is still sanitized by `prepare_turn` exactly as before.
- Both composers (main chat and side chat) share this path, so both are fixed.

## Unit Tests

In `src-tauri/bridge-core/src/live_turn.rs`:

- `preparing_an_image_only_turn_accepts_empty_text`: `prepare_turn("")` and
  `prepare_turn("   ")` succeed and return the text as given.
- `preparing_a_turn_for_a_missing_session_is_still_refused`: an unknown
  session id is still an error.
- `an_image_only_send_prepares_then_delivers`: the UI's real sequence,
  `prepare_turn("")` then `submit_input_with_attachments("", [png])`, starts a
  turn and the provider receives the image.
- `an_empty_send_without_images_is_refused_at_submit`: `prepare_turn("")`
  succeeds, then `submit_input_with_attachments("", [])` is refused with
  "Message cannot be empty" and nothing reaches the provider.
- Existing image tests (`an_image_only_send_is_delivered_without_text`, etc.)
  still pass.

## Integration / Functional Tests

- `cargo test -p bridge-core --lib live_turn::` passes.
- `cargo test -p bridge-protocol` passes (no wire change, no artifact drift).

## Smoke Tests

- `bun run check` and `bun run build` are green.

## E2E Tests

N/A as an automated suite: the frontend runs on mock data in tests, and the
mock `prepareTurn` never refused empty text, so a vitest case cannot reproduce
the bug. The core tests above drive the same two calls the UI makes.

## Manual / cURL Tests

Against a daemon built from this branch:

1. Open a chat on Claude or Codex, paste an image, leave the text empty, press
   Send. The bubble shows the image and the agent answers about it.
2. Press Send with an empty composer and no image: nothing happens (the UI
   returns early, as before).
