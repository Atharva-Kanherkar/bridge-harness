// stdin frame → SDK message content, one seam so image blocks cannot be
// silently flattened away.
//
// Bridge's Rust adapter always sends an array of Anthropic content blocks:
// `[{type:"text",…}, {type:"image",source:{type:"base64",…}}, …]`. Flattening
// to text here would drop images exactly the way image paste exists to fix,
// so blocks are forwarded as-is. A bare string or absent content still
// degrades to a single text block for tolerance.

export function userContentBlocks(frame) {
  const content = frame?.message?.content;
  const blocks = Array.isArray(content) && content.length > 0
    ? content
    : [{ type: "text", text: typeof content === "string" ? content : "" }];
  // An all-blank text frame carries nothing an SDK turn could consume; the
  // legacy stdin loop skipped empties and image-bearing frames must not get
  // caught in that net.
  const blank = blocks.every((part) => part?.type === "text" && !part?.text);
  return blank ? null : blocks;
}
