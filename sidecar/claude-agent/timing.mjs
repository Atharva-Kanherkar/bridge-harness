// Where one Claude turn spent its time inside the harness, from the moment
// its user message arrived on stdin. Bridge sees only stdin and stdout; this is
// the inside of that box, reported as one `system/bridge_timing` frame after
// the turn's result. Bridge's normalizer drops unknown system subtypes, so a
// reader that does not know this frame ignores it.
//
// Durations only: no text, tool input or model output.

export function createTurnTiming(boot, now = () => performance.now()) {
  let pendingBoot = boot;
  let turn = null;

  const at = () => Math.round(now() - turn.received);
  const once = (name) => {
    if (turn && turn.marks[name] === undefined) turn.marks[name] = at();
  };

  return {
    userFrame() {
      // A message sent mid-turn is steering, not a new turn: keep timing the
      // turn already in flight.
      if (!turn) turn = { received: now(), marks: {}, requests: 0 };
    },
    observe(message) {
      if (!turn) return;
      if (message?.type === "system" && message.subtype === "init") once("initMs");
      if (message?.type === "system" && message.subtype === "status" && message.status === "requesting") {
        once("requestingMs");
        turn.requests += 1;
      }
      if (message?.type === "stream_event") {
        const type = message.event?.type;
        if (type === "message_start") once("firstByteMs");
        if (type === "content_block_start") once("firstBlockMs");
        if (type === "content_block_delta") once("firstDeltaMs");
        if (type === "content_block_start" && message.event?.content_block?.type === "text") once("firstTextBlockMs");
      }
      if (message?.type === "assistant") once("firstAssistantMs");
    },
    finish() {
      if (!turn) return null;
      const frame = {
        type: "system",
        subtype: "bridge_timing",
        turn: { ...turn.marks, resultMs: at(), requests: turn.requests },
        ...(pendingBoot ? { boot: pendingBoot } : {}),
      };
      pendingBoot = null;
      turn = null;
      return frame;
    },
  };
}
