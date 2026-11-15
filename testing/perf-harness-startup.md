# perf/harness-startup: test contract

Getting a harness from "send" to "first token" must stop paying for work the
launch throws away. Written before the implementation; it is the definition of
done for the branch.

## Measured baseline

macOS, debug build, this machine's own provider sign-ins, cold chat (one send
after `create_chat_id`), measured by
`src-tauri/bridge-core/tests/launch_latency.rs`:

| Harness | `start_chat` invoke | first assistant frame |
| --- | --- | --- |
| Claude | 11.8 s, 12.9 s, 13.3 s, 21.3 s | 14.0 s, 16.1 s, 18.2 s, 25.8 s |
| Codex | 0.19 s, 0.47 s | 4.3 s, 5.1 s |

Model switch inside Claude (`claude-sonnet-5-5` → `claude-haiku-4-5`):
`update_chat_model` 1.06 s, then the next send is another cold launch
(`start_chat` 11.0 s, first assistant 13.2 s).

The `start_chat` figure is almost entirely pre-spawn. `spawning` is published at
the end of it, and the Claude adapter spawns the Node sidecar immediately after.
Standalone measurements of the two subprocesses the launch runs before spawn:

| Command | Time |
| --- | --- |
| `claude mcp list` | 10.2 to 10.8 s |
| `claude plugin list --json` | 0.67 to 0.72 s |

`claude mcp list` connects to every configured connector and reports health.
For an ordinary chat its output is discarded: `sidecar_mcp_servers` hands a
chat no explicit connector list at all (a chat's connectors arrive natively,
with the account's own sign-in), and `launch_configuration.connector_health` is
filtered to the launch's own server map, which is empty. Only a briefing run,
or a networked read-only worker, uses that discovery.

## Functional behavior

### 1. A chat launch does not run connector discovery

- `start_chat` for an interactive, non-briefing, non-read-only session must not
  invoke `claude mcp list`, and must not wait on it.
- Plugin discovery still runs: a chat still receives its enabled plugin paths,
  and `plugins` still reaches the sidecar config.
- A briefing run (`briefing: Some(policy)`) still discovers connectors; its
  explicit server list is unchanged.
- A networked read-only worker still discovers connectors; a non-networked one
  still discovers nothing.
- Settings and workflow surfaces that show connector names or health still get
  a real answer (`claude_sdk_configuration` unchanged for them).

### 2. Connector health is honest when it was not probed

- A path that never probed connectors must not report them as connected, nor
  report a discovery failure it did not have. Empty/unknown is the only
  acceptable answer.

### 3. Discovery results are reused, not recomputed per launch

- Two launches within the cache window run provider discovery once.
- Concurrent launches share one discovery (single flight), not one each.
- After the window expires the next launch discovers again, so a plugin
  installed in Claude Code takes effect without restarting Bridge.

### 4. Switching model or harness stays a real switch

- Model and harness switching keep their current semantics: the outgoing
  runtime is retired, the stored selection commits, and the next send launches
  the selected provider.
- The switch itself must not gain a cold launch: `update_chat_model` stays at
  or below its baseline (about 1 s, dominated by the 3 s-capped settle wait
  which returns as soon as the turn is idle).

### 5. Nothing regresses for a hot session

- A second send on a live session still takes the hot path and starts no
  process.

## Targets

Measured with `launch_latency.rs` on the same machine and build profile.

| Metric | Baseline | Target |
| --- | --- | --- |
| Claude `start_chat` invoke | 11.8 to 21.3 s | ≤ 2.5 s |
| Claude first assistant frame | 14.0 to 25.8 s | ≤ 8 s |
| Claude switch then first assistant | 13.2 s | ≤ 8 s |
| Codex `start_chat` invoke | 0.19 to 0.47 s | no regression |

## Unit tests

- `claude_mcp_discovery_is_skipped_for_a_chat_launch` — a chat launch issues
  no `mcp list`-shaped discovery, and the plugin list still reaches the config.
- `briefing_launch_keeps_connector_discovery` — a briefing launch does.
- `discovery_cache_single_flight` — N concurrent callers run the probe once.
- `discovery_cache_expires` — after the window, the probe runs again.
- `discovery_cache_keys_by_provider` — Claude's result never answers a Codex
  question.
- `connector_health_is_empty_when_not_probed` — no invented state.

Existing adapter and live-turn tests must keep passing unchanged
(`cargo test -p bridge-core`).

## Integration / functional tests

- `launch_latency.rs` cold-launch bench, Claude and Codex, before and after.
- A launch records the phases it observes; the bench reads them.
- Claude live smoke (`scripts/smoke-builtin-adapters.sh`) still runs a turn.

## Smoke tests

- `bun run build` green.
- `cargo test -p bridge-core` green.
- Launch Claude in the app, send "hi": first output appears in seconds, and the
  chat still has its plugins and skills.
- Launch Codex in the app: unchanged.

## E2E tests

N/A: the app's chat journey is covered by the existing App tests plus the live
smoke above; no new UI surface is introduced.

## Manual / cURL tests

- `BRIDGE_LAUNCH_TIMING=1` on a debug build prints one launch timing line per
  cold launch with phase durations.
- Switch a chat from Claude Sonnet to Claude Haiku and send: no provider
  discovery delay before the answer.

## Out of scope

- Prompt compilation and transcript rendering (separate perf branches).
- Provider-side time to first token.
- Changing what a chat's connector set is.
