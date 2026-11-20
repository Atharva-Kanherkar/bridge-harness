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

## Model switches

A switch inside one app (Opus to Sonnet on Claude Code, or between Codex models) keeps the provider thread: the next turn resumes it under the new model. Anything else needs a handoff, and the rules for it are:

- **The outgoing model's summary is a checkpoint, never a boundary.** When the outgoing process is still alive, Bridge asks it for a summary in the background. That model saw its own thread, not the whole chat: after a round trip it may have seen two turns. So its reply lands as a plain `checkpoint` that heads the handoff text, and the stored history still fills the tail under the usual budget. It never writes a `compaction` entry or moves the head.
- **A failed summary builds nothing.** No reply, no JSON, a timeout or a stopped process records `compaction.failed` and nothing else. The stored history is the handoff. (A reconstructed checkpoint glued from the last twelve messages used to be committed as a boundary here, which turned a two-hundred-message chat into one paragraph.)
- **Going back resumes the thread the chat left.** Every switch that leaves a provider thread records it in `session_provider_threads`, under its continuation key (the app plus the home its account reads threads from) and the last forest sequence it saw. A later switch to the same app and account, on an adapter with native resume, puts that thread back on the session and plans a native start. No summary is asked for.
- **The returned thread gets only what it missed, once.** Its first native start adds a `conversation_since_you_left` section to the session-context frame: the messages and worker results after its recorded sequence, oldest first, within the restoration budget. It rides beside the next message, never in the system prompt, so the resumed thread's prefix is unchanged. If the resume fails, the remembered thread is forgotten and the ordinary checkpoint-restored projection carries the whole history.
- **Bridge writes its own header, with no model.** Most switches have no summary to wait for: the old process was reaped, or the provider is out of quota, which is often why the person switched. So every handoff carries a facts block Bridge builds from stored history (`handoff_facts`): `goal:` (the latest ask), `started with:` (the first ask, only when the tail no longer reaches it), `files touched:` (file-change tool calls, worker results, artifacts and the newest checkpoint's list, newest first, at most 15), and `links:` (GitHub and GitLab pull requests, merge requests and issues seen in messages and tool calls, newest first, at most 8). A model summary, when one lands, comes first and the facts follow. The block is at most 4 KiB and its room is reserved before the tail is cut, so the whole handoff stays within the incoming model's budget. Git status is not read: the handoff is built under the database lock, and the new model can run it itself.
- **The tail carries tool facts, not tool output.** The tool calls between two messages ride as one `tools:` line: what ran and how it ended (`ran \`cargo test\` (exit 101)`), which files a tool wrote, a tool's name and target. A line names the newest six calls of the run in at most 480 bytes, after a count of the earlier ones. Facts are scrubbed before they are clipped. Tool lines spend the same budget and do not count as messages. The delta a returned thread is handed carries them too.
- **The handoff says where the rest is.** When the tail could not carry every message and the chat is a top-level chat, the facts block ends with `earlier: N older messages are not in this handoff; call chats_history with before=S to read them, tool output included.` `chats_history` pages the chat's own history, scrubbed ([chat history tools](chat-history-tools.md)).
- **An account change on the same app carries the thread.** Two accounts that read threads from different homes used to hand over like two different apps. Now, on Claude Code and Codex, Bridge copies the thread's transcript into the new account's home before the switch commits (Claude: `projects/<folder>/<id>.jsonl` under the config folder; Codex: the rollout under the shared home, at the same relative path) and the next turn resumes the same thread natively, with no summary asked for. The copy goes to a temp file and is renamed into place, never outside the destination home, and a destination newer than the source is kept. If the copy fails, the switch records `session.thread_carry_failed` and takes the ordinary handover. OpenCode keeps threads in its own store and still hands over.
- **The switch notice says what the new model gets:** the summary, decisions and files when there is a header, and "the latest N of M messages" when the budget cuts the tail, or that the chat goes back to its earlier session and how many messages are added. With no model summary it says what Bridge carried: "all 12 messages, 3 files and 2 links (no model summary)".
- **The ledger records the handoff.** A checkpoint-restored start stores the compiled prompt that carried `restoration_context`, so `prompt_compilations` shows the real variable bytes. Its stable prefix hash is unchanged, so the hot-process check is unaffected.

### Switching or delegating

Switching the chat model is for continuing the same conversation with a different model. For one task on another model, delegate it to a worker instead: the chat keeps its own thread and context, and the worker gets a brief and returns a typed result. A switch to another app always costs something (a fresh thread, or a resumed one that missed part of the chat); a delegation costs neither.

## Compaction ownership

**Within a live process the harness owns its context window. Bridge does not compact it.** Bridge owns the durable record of what the harness did, and it owns the checkpoint that survives the process.

This is a decision, not an accident. The harness is what talks to the model, so it is the only layer that can shrink the request it sends, and all three managed harnesses already do:

| Harness | Reports its own compaction as | Accepts a compaction command |
| --- | --- | --- |
| Claude Code | `compact_boundary` system message, with `trigger`, `pre_tokens`, `post_tokens` | a `/compact [focus]` slash command on the input stream |
| Codex | a `contextCompaction` thread item (`thread/compacted` is deprecated) | `thread/compact/start`, whole conversation only |
| OpenCode | a `session.compacted` event (2.x: `session.compaction.ended`) | `POST /session/{id}/summarize` (2.x: `POST /api/session/{id}/compact`, no model needed), whole conversation only |

Three facts settled it. Bridge's own pressure trigger had never fired: of the successful compactions in a month of real use, every one was a phase boundary, a model switch, or a shutdown. A Bridge checkpoint frees no provider tokens, because committing one writes forest entries and moves the session head without touching the adapter. And making it free tokens would mean restarting the process to replay a shorter history, which discards the native session and the prompt-cache prefix that native resume and the byte-stable system prompt exist to preserve.

So the two layers divide cleanly:

- **The harness** decides when its live window is full, compacts it, and reports the boundary. Bridge normalizes that report into a durable `context.compacted` entry and draws the "Context compacted" card from it. The card is never drawn from a Bridge checkpoint, because a checkpoint on a hot session does not shrink the provider's context.
- **Bridge** decides when history needs a semantic summary that outlives the process, and writes a checkpoint. A checkpoint is a cold-start payload: `ContextProjector` consumes it when a session starts fresh or hands off to another model. On a hot session it is bookkeeping, and the UI says "Checkpoint saved" rather than claiming the context shrank.

While a native compaction runs, the transcript says so. Bridge normalizes the harness's own start signal (Claude's `compacting` status, Codex's `contextCompaction` item opening) into `context.compacting`, and announces the same event itself when it forwards `/compact`, because OpenCode reports only the boundary. The event is live only: the store writes no row for it, since a stored "compacting" is a claim history could never retract if the boundary never came, and a replay has no use for it. The transcript draws it as the status row's label ("Compacting context…", in place of "Thinking") from the earliest unsettled announcement until the boundary or the end of the turn, whichever comes first. It is never a row of its own, so the card the transcript keeps is still only the boundary.

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
