# Browser selection evals

Two opt-in evals check when agents request a Bridge browser clone. They run on macOS through Bridge's production Codex and Claude adapters, using installed and authenticated runtimes:

```sh
# What the agent actually does (the one that reproduces the bug)
cargo test --manifest-path src-tauri/Cargo.toml -p bridge-core live_browser_behavior_eval -- --ignored --nocapture
# What route the agent says it would pick
cargo test --manifest-path src-tauri/Cargo.toml -p bridge-core live_browser_selection_eval -- --ignored --nocapture
```

Set `BRIDGE_BROWSER_EVAL_PROVIDERS=codex` or `claude` to run one provider; both run by default. Set `BRIDGE_BROWSER_EVAL_CASES` to comma-separated case ids to rerun a subset, such as cases cut short by a provider rate limit. Each case gets each provider's default model, a fresh provider session, and an empty working directory. The real generated clone-request context arrives through the adapter's trusted turn-context path: `bridge.credentials` for Codex and a separate content block for Claude. No browser or cookie import runs, because no orchestrator sits behind the clone helper.

**Behavior** (`behavior-cases.json`, 7 cases per provider). The task is sent as a person would type it. A signup page is served on an ephemeral `127.0.0.1` port, and `index.html` is in the cwd. The agent runs with Bridge's default full-access mode and its tool calls execute. The turn stops at the first call that invokes the `clone-request-*` helper, after 8 tool calls, or at a 240-second deadline. The score is whether the agent requested a clone, compared with `clone` in the case. Provider errors fail the case. The first tool call is printed so a run shows what the agent reached for.

**Selection** (`cases.json`, 14 cases per provider). The agent is asked for one route word (`automation`, `direct`, `clone`, or `blocked`) without calling tools. The first word is scored. Any tool attempt, provider error, or timeout after 120 seconds fails the case. This format names the routes, so it states the boundary but does not reproduce the misfire. Old and new prompts both pass it, while only the new prompt passes the behavior eval.

To compare a previous context, set `BRIDGE_BROWSER_EVAL_CONTEXT` to a UTF-8 file containing that generated context. The literal `{tool}` is replaced with the session's helper path. For this PR's baseline, extract the raw-string context from `clone_browser_tool.rs` at `5c2e1316` and unescape Rust format braces (`{{` to `{`, `}}` to `}`). A wrong route makes the test fail, by design.

The ordinary test suite separately checks that the generated instructions keep the selection boundary and clone protocol, and that Bridge injects them through the live-turn context path. Existing consent and lifecycle tests still exercise the real helper and orchestrator.
