# Direct harness tool calls contract

Slice 1 of the personal MCP apps work. Outgoing's exact Slack calls run through
the harness's own MCP client with `mcp_call`, not through a hidden model turn.

## Functional behavior

- An exact outgoing call (`slack_send_message`, `slack_get_file_upload_url`,
  `slack_complete_file_upload`) on a Claude connection whose installed Agent SDK
  is 0.3.322 or newer takes the direct path. Older SDKs, an unknown version and
  every other harness take today's hidden turn, unchanged.
- The path is chosen before anything is sent. A direct call that fails after
  its frame was written is reported as a failure and never retried on the turn
  path, because the tool may already have run.
- The direct path launches what the turn path launches: the same
  `compile_exact_action` policy (marked `into_direct_call`), a `connector`
  session row settled on every exit, the same account and credentials, and the
  same wall-clock limit. It sends no user turn.
- A direct-call launch loads its connector natively. Claude Code brings up its
  own signed-in claude.ai instance on demand, so the launch skips connector
  discovery, hands over no explicit `claude.ai ...` copy (such a copy comes up
  `needs-auth` with no tools) and is not `strictMcpConfig`. Turn launches keep
  today's explicit list and strict config.
- Connector scopes compare servers in wire form (`claude.ai Slack` is
  `claude_ai_Slack` in tool names) in both the Rust policy and the sidecar gate,
  and the exact action is named by its wire tool name.
- The sidecar honours a direct call only when the launch carries
  `briefing.directCall` and `briefing.exactAction`, the frame's tool and arguments deep-equal it, and no
  call has been made yet. It sends `check_permissions: true`, so the SDK also
  runs the same exact-action gate as a model's own call would.
- A successful result is decoded into `TurnOutput.results` exactly as a tool
  result block is today, so Slack receipt and upload parsing are unchanged.
  `isError` results, SDK refusals ("The tool did not run: ...") and a missing
  response before the deadline are errors.
- Each exact call writes one `connector.action.direct` or
  `connector.action.turn` ledger event with the tool, the outcome and the
  elapsed milliseconds. Arguments and message text never enter the ledger.

## Unit tests

- Sidecar: a frame is refused without `exactAction`, with a different tool,
  with different arguments, and on a second call; an allowed frame forwards the
  exact `mcp_call` request with `check_permissions` and `result_not_for_model`;
  a rejection becomes `{ok:false,error}`; a result becomes `{ok:true,result}`.
- Rust: the SDK version gate (0.3.321 no, 0.3.322 yes, 0.4.0 yes, unreported
  no, malformed no); the path choice per harness and gate; the result decoder
  produces the same `results` as the turn path for the same tool content; an
  `isError` result and an `ok:false` frame become errors; the reader ignores
  unrelated frames and frames with another id, and times out with an error.
- Claude runtime writes the `mcp_call` frame on stdin with the given id; other
  runtimes refuse `call_tool`.
- Policy: `into_direct_call` only on an exact action; scoped reads, approved
  replies and the exact action all match `mcp__claude_ai_Slack__...`, and the
  raw `mcp__claude.ai Slack__...` spelling is refused. This test fails on
  main's logic.
- Sidecar options: a direct call is not strict, drops explicit claude.ai copies,
  loads no settings or plugins, and its gate still allows exactly one call.
- Launch discovery: a direct call skips it; the same send as a turn keeps it.

## Integration and smoke

- A real sidecar run against a local stdio MCP server (no network, no model):
  the tool runs once, the result frame arrives, no `assistant` or `result`
  frame appears, and a mismatched frame never reaches the server. Runs with
  `BRIDGE_SIDECAR_LIVE_TESTS=1` or Claude credentials in the environment.
- Manual, read-only: a direct `mcp_call` of `slack_search_channels` against the
  real claude.ai Slack connector succeeds when loaded natively, and fails with
  `needs-auth` / "MCP server not connected" through an explicit copy.

## Verification

- `node --test sidecar/claude-agent/test/*.mjs`
- `cargo test -p bridge-core connector_runs_live:: claude_adapter::`
- `bun run build` and `bun run test` before the PR.
- Live Slack send through the running app is a manual check; report it as not
  run if no Slack connector is available.

## Review fixes

- **P1, the Outgoing resolver must reach the signed-in connector.** A fresh
  Slack draft is resolved by a read-only model turn (`slack::resolve_outgoing`)
  before any approval exists. That launch loads claude.ai connectors natively
  (no explicit copy, not `strictMcpConfig`, no discovery), exactly like a
  direct call, while its gate stays the read-only scope on the one Slack
  server: reads allowed, every write and every other server denied. Inbox
  ingress, render and reply launches are unchanged.
  Tests: a native read policy keeps the read-only decisions; the resolver's
  policy is native, read-only, scoped to the Slack server, carries no action
  and is not a direct call; sidecar options for a native read launch (not
  strict, no claude.ai copy, read allowed, send and other servers denied);
  launch discovery is skipped for it; a flow test drives `send::run` from a
  Slack draft through the real resolution parser, approval, claim, Slack
  authorization, the direct-call policy, the result decoder and the receipt
  check to a sent message, and a failed resolution never offers approval or
  executes. Live and read-only: the resolver's launch reaches the real Slack
  connector through a model turn.
- **P2, the Clippy ratchet does not rise.** The ledger write handles a poisoned
  lock and a failed insert explicitly; neither changes the outcome of a call
  that already ran. Verified with `node scripts/clippy-ratchet.mjs`.
