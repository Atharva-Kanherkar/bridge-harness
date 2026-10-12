# Pi coding-agent integration

Bridge uses Pi's native RPC subprocess protocol. Install the `@earendil-works/pi-coding-agent` CLI (Pi 1.1+), configure a provider in Pi, and refresh Coding agents in Bridge. Pi owns authentication, provider configuration, local inference and its sessions. Bridge discovers provider-qualified model IDs and model-specific thinking levels from RPC; it never copies provider keys, model headers or endpoint configuration into its catalog.

Pi appears in Coding agents settings even when it has no usable model. Configure local GGUF models through Pi's llama.cpp router or compatible providers such as Ollama, LM Studio or vLLM using Pi's own configuration. No model weights or Pi installation are managed by Bridge.

Bridge starts Pi with an explicit session directory under `$PI_CODING_AGENT_DIR/sessions/bridge` (or `~/.pi/agent/sessions/bridge`) and resumes those session files. The child and its tool descendants use Bridge's process-group supervision and parent-death watchdog. RPC uses strict LF-delimited JSONL, independent command IDs, bounded input/output pressure and private stderr draining. `agent_end` may precede retries or recovery; only `agent_settled` completes the Bridge run.

## Permissions and supported scope

Pi has no built-in operating-system sandbox. Bridge therefore advertises only `danger_full_access`; read-only workers, workspace-write workers and orchestrator briefings are rejected before launch. Tool approval is authorization, not filesystem or network containment.

Bridge disables startup network operations with `--offline`, preventing automatic package installation before the approval gate. It explicitly denies project trust and disables discovered extensions, MCP, skills, templates, themes and context files. Only Bridge's bundled permission extension is loaded. Its readiness must be proven before any prompt can be sent. Every model tool call invokes Pi's `tool_call` hook and waits for Bridge approval. Missing UI, cancelled/denied approval and extension exceptions deny execution. Pi slash commands and direct shell input are disabled to keep this gate in place. Completion sessions disable tools.

Pi's RPC prompt has no trusted per-turn context channel. Bridge does not inject hidden memory/session packets as user text, and rejects credential-context turns. Use provider authentication configured in Pi itself. Initial compiled instructions use Pi's system-prompt channel. Bridge exposes the thinking levels that fit its shared effort vocabulary; automatic selection keeps Pi's own model setting when no shared level applies.

## Validation

`cargo test --manifest-path src-tauri/Cargo.toml -p bridge-core pi_adapter` exercises strict framing, correlation, pressure, supervised fake-Pi lifecycle, security declarations, permissions, terminal snapshots, retry/settlement and usage accounting. `node --test scripts/test/pi-permission.test.mjs` verifies the extension's fail-closed gate. `bun run check:builtin-adapters` includes Pi's compatibility and lifecycle checks.

For a credential-free real CLI smoke test, set `BRIDGE_PI_LIVE_BINARY` to an installed Pi 1.1+ executable and run the ignored `live_pi_rpc_proves_the_permission_gate_and_catalog_without_a_model_call` test. Use an isolated `PI_CODING_AGENT_DIR` if you want an empty provider catalog. Live paid-provider generation and packaged desktop interaction are follow-up checks when a configured provider is available.

Protocol references: [Pi RPC](https://pi.dev/docs/latest/rpc), [RPC commands](https://pi.dev/docs/latest/rpc-commands), [event stream](https://pi.dev/docs/latest/json), [security](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/security.md).
