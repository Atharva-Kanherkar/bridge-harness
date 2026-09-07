import assert from "node:assert/strict";
import test from "node:test";

import { userContentBlocks } from "../input.mjs";

test("an image attachment is forwarded beside its text block, untouched", () => {
  const frame = {
    type: "user",
    message: {
      role: "user",
      content: [
        { type: "text", text: "what is this?" },
        { type: "image", source: { type: "base64", media_type: "image/png", data: "iVBORw0" } },
      ],
    },
  };
  assert.deepEqual(userContentBlocks(frame), frame.message.content);
});

test("the legacy plain-text frame shape still produces exactly one text block", () => {
  assert.deepEqual(
    userContentBlocks({ type: "user", message: { role: "user", content: [{ type: "text", text: "hi" }] } }),
    [{ type: "text", text: "hi" }],
  );
});

test("a bare string content degrades to a single text block", () => {
  assert.deepEqual(
    userContentBlocks({ type: "user", message: { role: "user", content: "hi" } }),
    [{ type: "text", text: "hi" }],
  );
});

test("absent or all-blank content produces nothing to push", () => {
  assert.equal(userContentBlocks({ type: "user", message: { role: "user" } }), null);
  assert.equal(
    userContentBlocks({ type: "user", message: { role: "user", content: [{ type: "text", text: "" }] } }),
    null,
  );
});
