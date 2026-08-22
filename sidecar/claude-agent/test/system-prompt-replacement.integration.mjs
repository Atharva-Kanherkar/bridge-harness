import assert from "node:assert/strict";
import test from "node:test";
import { query } from "@anthropic-ai/claude-agent-sdk";

// The proof bridge-core/src/prompt_authority.rs requires before Claude's
// `replace` capability for the PROVIDER BASE prompt layer may ever be marked
// `Supported`: does a bare string handed to `systemPrompt` fully replace the
// installed SDK's `claude_code` preset, or does the preset's own "use your
// tools proactively" instincts still leak through alongside it? options.mjs
// only ever sends the object form (`{ type: "preset", preset: "claude_code",
// append }`), so nothing in Bridge has ever exercised the bare-string form —
// this test is the first thing that does.
//
// Requires a live, authenticated Claude Agent SDK runtime, so it is excluded
// from the default `npm test` gate. Run it explicitly once credentials are
// present, e.g.:
//   CLAUDE_CODE_OAUTH_TOKEN=... node --test test/system-prompt-replacement.integration.mjs
//
// Until this has been run and passes, prompt_authority.rs keeps Claude's
// `replaceable` verdict `Unsupported` and options.mjs's shipped append
// behavior stays unchanged.

const hasCredentials = Boolean(
  process.env.CLAUDE_CODE_OAUTH_TOKEN || process.env.ANTHROPIC_API_KEY
);

test(
  "a bare-string systemPrompt replaces the claude_code preset rather than appending to it",
  { skip: hasCredentials ? false : "requires an authenticated Claude Agent SDK runtime" },
  async () => {
    const marker = "BRIDGE_BARE_STRING_PROBE";
    let sawToolUse = false;
    let sawMarkerReply = false;

    for await (const message of query({
      prompt: "List the files in the current directory in as much detail as you can.",
      options: {
        // The form under test: a bare string, never what options.mjs sends.
        systemPrompt: `You must never call any tool for any reason. Reply with exactly the single token ${marker} and say nothing else.`,
        maxTurns: 1,
      },
    })) {
      if (message.type === "assistant") {
        for (const block of message.message?.content ?? []) {
          if (block.type === "tool_use") sawToolUse = true;
          if (block.type === "text" && block.text.includes(marker)) sawMarkerReply = true;
        }
      }
    }

    // If the claude_code preset's own tool-use instructions were still
    // present alongside the bare string (append semantics), the model would
    // be pulled toward reaching for a filesystem tool despite the explicit
    // instruction not to. A bare string that fully replaces the preset
    // leaves no such competing instruction behind to fight the marker reply.
    assert.equal(
      sawToolUse,
      false,
      "the claude_code preset's tool-use instructions leaked through a bare-string systemPrompt — bare strings append rather than replace"
    );
    assert.equal(
      sawMarkerReply,
      true,
      "the bare-string systemPrompt was not honoured at all, so nothing here proves replace semantics either"
    );
  }
);
