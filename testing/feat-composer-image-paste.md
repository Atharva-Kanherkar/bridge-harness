# feat/composer-image-paste — Test Contract

Solves the composer half of issue #290: pasting an image into Bridge's chat box
must produce a visible, removable preview chip and deliver the image to a
provider that can consume it — never silently swallowed.

Locked before implementation. Requirements changes amend this file first.

## Functional Behavior

### Paste interception (composer)
- Pasting clipboard data containing at least one image file item onto the
  composer textarea:
  - does NOT insert anything into the text value;
  - appends one attachment `{ id, mediaType, dataUri }` per image to the
    composer's attachment list;
  - renders an immediately removable thumbnail chip for each attachment.
- A paste with only text behaves exactly as today: default insertion,
  attachments untouched.
- A mixed paste (image + text): the image becomes an attachment; no duplicate
  text is inserted. (Some platforms put image and markup on separate flavors.)
- Non-image files copied as files (e.g. PDF) do not become chips; they are
  ignored by the paste path (out of scope), without breaking text paste.
- Attachments can be removed individually with the chip's × button before send.
- `sendPrompt` clears the attachment list after a successful submit; on failure
  both the composer text and remaining attachments are restored.

### Delivery semantics
- Empty text but ≥1 attachment is sendable.
- New turn: submitInput carries the images; disposition flows back unchanged.
- Steered turn (provider supports active-turn steering, i.e. Claude): mid-turn
  submission with images steers; the image rides along in the same user frame.
- Queued route + images → explicit refusal: "Images cannot be held in the
  queue — wait for the current step to finish, then send again." Nothing is
  enqueued and nothing silently drops.
- Provider without image support (Codex/OpenCode) + images → explicit
  `BridgeError::Invalid` naming image attachments; App surfaces it via
  `setError` toast. No silent drop anywhere.
- Images accompanying a slash command that Bridge answers locally → explicit
  refusal, because a local answer has nowhere to carry them.
- Images accompanying an answered pending question → explicit refusal.
- Secret interception applies to the message text only, never to base64 image
  data; `@file` context still appends after text.

### Claude adapter pass-through
- The stream-json user frame keeps its existing shape, with content blocks:
  `[{"type":"text","text":...}, {"type":"image","source":{"type":"base64",
  "media_type":..., "data":...}}, ...]` — one block per image, in order.
- Pure-text turns emit exactly today's frame (single text block). Byte-level
  behavior change: none.
- Credential context, when present, still composes (`send_turn_with_context`
  family) alongside images.

### Conversation rendering
- A persisted/pending user turn whose event data carries
  `data.attachments = [{ mediaType, dataUri }]` renders the thumbnails under
  the bubble text, plus live optimistic pending rows.
- Reload (forest replay projection) shows the same attachments — they are read
  from durable entry payloads, not from transient state.

## Unit Tests

Frontend (Vitest, colocated):
- `pasteAttachmentsFromClipboardItems` helper: maps image clipboard items →
  attachments; ignores non-image kinds/files; keeps order.
- `ComposerPill.test.tsx`: paste of image file calls handler, does not mutate
  textarea value; remove button drops exactly its own chip; Enter sends when
  only attachments exist.
- `conversation.ts` tests: `attachmentUris(data)` extracts valid attachments
  from payload shapes and returns [] for absent/malformed.

Rust (cargo test):
- bridge-protocol params round-trip: serde camelCase deserialization of
  `SubmitInputParams` with and without `attachments`; absent field stays
  backward compatible; unknown fields keep rejecting where declared.
- claude_adapter: `start_turn_with_images` writes one JSON line whose content
  array is [text, image…]; `start_turn` (no images) unchanged; image source
  carries `base64`, `media_type`, exact data.
- AdapterRuntime default contract: non-overriding runtimes report
  `supports_images() == false` and `send_turn_with_images` errors with the
  not-supported message (behavioral guard for Codex/OpenCode paths).

## Integration / Functional Tests
- `live_turn::submit_input` with images on a session backed by a
  supporting runtime routes like plain text (same InputRoute decisions).
- Refusal paths return `BridgeError::Invalid` with the specific messages above
  (queue+images; local slash+images; non-supporting runtime) and perform no
  enqueue/no persistence of a provider-bound turn.
- Persisted user turn stores attachment data URIs in event `data.attachments`.

## Smoke Tests
- `bun run build` green (tsc -b && vite build).
- `bun run test` green (vitest run + cargo test).
- Generated protocol artifacts in sync after wire changes:
  regenerate `src/protocol/generated/protocol.ts` via
  `cargo run -p bridge-protocol --bin generate-protocol-artifacts`.

## E2E Tests
N/A — not applicable for this change in CI (needs a real Claude session +
desktop webview paste). Manual verification covers it below.

## Manual / cURL Tests
1. `bun run tauri dev`, open a chat using a Claude model.
2. Copy any screenshot to clipboard (⌘⇧⌃4 on macOS) → focus composer → ⌘V:
   a thumbnail chip appears; press Enter. The user bubble (optimistic and
   after reload) shows the same image. Provider receives the image (ask
   "what's in this image?").
3. Same paste against a Codex chat → red inline toast: "…does not accept image
   attachments…" and the composer restores text + chip.
4. Copy plain text → paste → unchanged behavior, no chips.
5. While a turn is running in Claude: paste + send → steers, message marked
   "Steered", image included.
6. Paste into a chat with `/usage` typed → explicit refusal, nothing dropped.
