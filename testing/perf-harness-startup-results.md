# perf/harness-startup: measured results

Every number here was produced on this machine, in a debug build, with the
provider sign-ins already on it. The bench is
`src-tauri/bridge-core/tests/launch_latency.rs`.

## Experiment 0: where the time actually went

`start_chat` published `spawning` at the end of its own 12-13 second invoke, so
the provider process had not existed for any of it. The Claude launch ran two
provider subprocesses before the spawn:

| Command | Time |
| --- | --- |
| `claude mcp list` | 10.2, 10.4, 10.8 s |
| `claude plugin list --json` | 0.67, 0.67, 0.72 s |

A chat discards the `mcp list` answer: `sidecar_mcp_servers` hands a chat no
explicit connector list, and `launch_configuration.connector_health` is filtered
to that same empty map. The plugin paths are kept and reach the sidecar.

## Experiment 1: the sidecar boot, isolated

Driving `sidecar/claude-agent/index.mjs` directly, warm disk:

| Options | `system:init` | first token |
| --- | --- | --- |
| `settingSources: ["user"]`, `skills: "all"` | 1.5 to 2.2 s | 2.8 to 3.8 s |
| `settingSources: []` | 1.1 to 1.2 s | 2.7 s |
| `settingSources: ["user"]`, no skills | 2.2 s | 3.7 s |

The first ever run after `bun install` was 8.5 s to init, which is disk-cold
page-in, not a launch cost. Dropping `settingSources` would save about 0.4 s and
remove the person's skills, commands and native connector sign-in, so it was
rejected.

## Experiment 2: ask discovery only for what the launch reads

Discovery is now asked for by part. A chat keeps plugins and drops connectors; a
briefing keeps connectors and drops plugins (it runs under `strictMcpConfig`, so
the compiled list is its only route); a networked read-only worker keeps
connectors; a toolless or worktree run keeps neither.

Claude `start_chat`, cold chat, one send:

| | Before | After |
| --- | --- | --- |
| round 0 (process cold) | 11.8 to 21.3 s | 0.77 to 1.9 s |
| round 1+ (same process) | 11.8 to 21.3 s | 0.05 to 0.12 s |
| first assistant frame | 14.0 to 25.8 s | 1.9 to 3.0 s |

`BRIDGE_LAUNCH_TIMING=1` for the first launch in a process:

```
bridge: launch harness=claude total_ms=864 resolve_ms=17 (+17) prompt_ms=101 (+84) context_ms=102 (+1) spawn_ms=856 (+754) persist_ms=864 (+8) state_ms=864 (+0)
```

The 754 ms in the spawn step is `claude plugin list`, paid once per process.
Bridge's own planning is 100 ms.

## Experiment 3: reuse the answer

The launch path now reuses a discovery answer for 60 s, single-flight behind one
mutex. A model switch, a worker pool, or two chats opened together probe once
instead of each paying.

| | No cache | 60 s cache |
| --- | --- | --- |
| cold `start_chat`, second launch in the process | 0.77 to 1.9 s | 0.04 to 0.12 s |

The window is short enough that a plugin installed through Claude Code takes
effect without restarting Bridge, and settings/workflow surfaces still call the
fresh, uncached probe.

## Experiment 4: a model switch

`claude-sonnet-5-5` to `claude-haiku-4-5` on a live chat:

| Step | Before | After |
| --- | --- | --- |
| `update_chat_model` | 1.06 s | 1.17 s |
| next send's `start_chat` | 11.0 s | 0.055 s |
| switch to first assistant frame | 13.2 s | 2.9 s |

The switch command itself is unchanged because it is dominated by stopping the
outgoing provider: `stop_session` measures 0.89 to 1.04 s, almost all of it
waiting for the provider to exit after `SIGTERM`.

## Designs considered and rejected

- **Prewarm the next provider after a switch.** Bridge has no idle reaper, so a
  prewarmed process would sit in memory until the app quits if the person never
  sends. The stated constraint is protecting RAM, so no.
- **Switch model in place through the SDK's `setModel`.** The SDK supports it,
  but every adapter control frame today is fire-and-forget; the reader thread
  has no path to route a `control_response` back to the caller. Doing it
  properly is its own change, and it alters what a switch means (the same
  provider query continues rather than resuming the thread), so it does not
  belong in a performance fix.
- **Move the teardown off the switch's path.** The outgoing process must be gone
  before the next launch resumes the same provider session: Claude rejects a
  second process on a session id (`Session ID ... is already in use`, observed
  while measuring). Backgrounding the wait would trade a second of latency for a
  launch that can fail.
- **Read the plugin list from Claude's config files.** Faster on paper, but it
  binds Bridge to an undocumented format instead of the CLI that owns it. The
  answer is cached instead.

## What remains

After the fix a cold chat send is: 0.04 s of Bridge planning, one provider spawn,
then the provider's own boot. The remaining seconds belong to Claude Code, not to
Bridge:

| Segment | Time |
| --- | --- |
| Bridge planning and spawn | 0.04 to 0.12 s |
| Sidecar boot to `system:init` | 1.1 to 2.2 s |
| Provider first token | 0.7 to 1.8 s |
| Provider teardown on stop | 0.9 to 1.0 s |
