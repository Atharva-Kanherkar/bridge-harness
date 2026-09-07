# Compaction and resume

Bridge distinguishes stored history, provider-native context, and projected restoration context. Persisting an entry does not mean it is injected into a provider thread.

## Restoration modes

| Mode | Meaning |
| --- | --- |
| `hot` | The adapter process is still running. |
| `native` | The provider can resume its stored thread or session ID. |
| `checkpoint_restored` | Native resume is unavailable or failed, so Bridge starts fresh with a validated projection. |
| `fresh` | The task intentionally starts without prior conversation context. |

Native capability is discovered from the installed harness. A failed native resume is recorded before checkpoint fallback; Bridge never labels a fresh process as natively resumed.

## Compaction ownership

**Within a live process the harness owns its context window. Bridge does not compact it.** Bridge owns the durable record of what the harness did, and it owns the checkpoint that survives the process.

This is a decision, not an accident. The harness is what talks to the model, so it is the only layer that can shrink the request it sends, and all three managed harnesses already do:

| Harness | Reports its own compaction as | Accepts a compaction command |
| --- | --- | --- |
| Claude Code | `compact_boundary` system message, with `trigger`, `pre_tokens`, `post_tokens` | a `/compact [focus]` slash command on the input stream |
| Codex | a `contextCompaction` thread item (`thread/compacted` is deprecated) | `thread/compact/start`, whole conversation only |
| OpenCode | a `session.compacted` event | `POST /session/{id}/summarize`, whole conversation only |

Three facts settled it. Bridge's own pressure trigger had never fired: of the successful compactions in a month of real use, every one was a phase boundary, a model switch, or a shutdown. A Bridge checkpoint frees no provider tokens, because committing one writes forest entries and moves the session head without touching the adapter. And making it free tokens would mean restarting the process to replay a shorter history, which discards the native session and the prompt-cache prefix that native resume and the byte-stable system prompt exist to preserve.

So the two layers divide cleanly:

- **The harness** decides when its live window is full, compacts it, and reports the boundary. Bridge normalizes that report into a durable `context.compacted` entry and draws the "Context compacted" card from it. The card is never drawn from a Bridge checkpoint, because a checkpoint on a hot session does not shrink the provider's context.
- **Bridge** decides when history needs a semantic summary that outlives the process, and writes a checkpoint. A checkpoint is a cold-start payload: `ContextProjector` consumes it when a session starts fresh or hands off to another model. On a hot session it is bookkeeping, and the UI says "Checkpoint saved" rather than claiming the context shrank.

A native compaction never begins, cancels, or satisfies a Bridge compaction, and never moves the active branch. The two records sit side by side in history: one says the provider's window shrank, the other says Bridge summarised the conversation.

`/compact` is forwarded to the harness when the harness has a compaction command. Where that command takes no focus, the focus is reported as not applied rather than dropped in silence. A harness with no compaction command falls back to a Bridge checkpoint.

Rust's compaction controller decides when a checkpoint is needed. The session being compacted authors its own semantic checkpoint because it owns its decisions, risks, and incomplete work. The orchestrator receives typed worker results and checkpoints, not raw worker transcripts.

Checkpoints are triggered at phase boundaries, before suspension or model downgrade, before risky native resume, or on manual request. They are suppressed during tool calls and approvals, for short-lived workers with valid final results, and when no meaningful work occurred since the last boundary.

The `ContextPressure` trigger stays reachable only where Bridge is the sole owner of the window: a harness that reports a context gauge and has no compaction of its own, which today means the agent-protocol harnesses. A direct chat never auto-compacts, and that exclusion is deliberate rather than an oversight. Its harness owns the live window, so Bridge has no pressure to relieve, and its boundaries are the four above.

### What the model is asked for

A checkpoint request asks the session for meaning only: a summary, the decisions taken, the files touched, and what is still open. Bridge fills the bookkeeping itself, so the schema version, the source agent, the first retained entry, the token count, and the reason are never echoed by a model and can never be echoed wrongly. The reply is read by extracting its first JSON object, so a fenced block or a sentence of preamble parses.

Checkpoints and compaction boundaries are immutable forest entries. Original events remain stored. Validation failure gets one same-session repair; a dead process may use a recovery worker, whose result is explicitly marked reconstructed.

Schema validity is not enough for an agent-authored checkpoint. Before committing a boundary, Rust independently scans the active SQLite branch since the previous completed compaction and deduplicates durable worker decisions and file evidence. That evidence is given to the model in the request, so the first attempt already knows what it must account for.

An evidence gap is repaired rather than fatal. Rust appends the missing decisions and files to the checkpoint and stamps `provenance: "agent+controller"`, so the record says plainly which parts the model wrote and which parts Bridge supplied. A checkpoint that already accounts for its evidence keeps `provenance: "agent"`. Successful verification and its evidence counts are committed atomically as a `checkpoint.evidence_verified` audit event; reconstructed checkpoints continue to identify their independent provenance explicitly.

What remains a failure is a reply that never arrives, times out, or contains no JSON object at all. Malformed shape and mismatched bookkeeping are no longer failure modes, because the model is no longer asked to produce either.
