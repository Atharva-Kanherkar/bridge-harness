# fix-context-lens-mcp-and-compact — Test Contract

Two fixes to the Context lens.

## Functional Behavior

1. **Deferred MCP tools are not a per-turn cost.** With tool search on, Claude
   Code sends an MCP tool's schema only after the model looks it up. The SDK's
   `getContextUsage().mcpTools[]` still lists every schema's size, flagging
   each with `isLoaded`. The sidecar sums `tokens` from loaded tools only
   (`isLoaded !== false`), and adds `loaded` (count) and `deferredTokens` per
   server. `tools` stays the total.
2. **Consumer copy.** Rust shows an MCP server as a consumer only from a
   frame that carries `loaded`. Its tokens are the loaded tools. The detail
   reads `N tools loaded` when all are loaded, otherwise
   `L of N tools loaded · rest on demand`. A server with nothing loaded has
   zero tokens and drops out. A frame from before the split, without
   `loaded`, is skipped, because it summed every schema. The words
   "every turn" are gone.
3. **Compact button.** Every window's detail in the lens has a Compact action
   for that session: chat, orchestrator or worker. It submits `/compact`
   through `sessions/submit_input`, so the existing slash routing decides.
   Claude, Codex and OpenCode run their own compaction. A harness without one
   (Cursor, Grok) gets a Bridge checkpoint, and the button says "Checkpoint"
   and that it does not shrink the window. A busy session's refusal message
   is shown inline as an alert and the button is offered again. After a send
   the button reads "Compacting" until a new reading arrives.

## Tests

- Sidecar `context-usage.mjs`: loaded-only sums, `loaded`, `deferredTokens`.
- Rust `context_windows::claude_breakdown_feeds_segments_consumers_and_auto_compact`:
  all-loaded, partly-loaded, none-loaded and legacy servers.
- `ContextLensDialog.test.tsx`: compact calls the selected session id for the
  chat and a worker, Cursor shows Checkpoint, a refusal renders as an alert.

## Smoke

`bun run build`, `bun run test` green.
