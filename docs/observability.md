# Observability: the record, and how to read it

Bridge drives five agent runtimes. When one of them does something surprising —
a tool that looked successful, a turn that stopped for no visible reason, a
delegated worker that returned nothing — the rendered conversation is the wrong
place to find out why, because the conversation is a *lossy projection*. This
document says what Bridge actually records, what it deliberately does not, and
how to get the record out.

## One record, not five

Every adapter normalizes into one event vocabulary before anything is stored
(`agent.rs` for Codex's native protocol, `acp_events.rs` for the ACP runtimes,
the Claude sidecar for Claude). `store::session_event_in_transaction` is the
single door through which a normalized event becomes durable history, and the
session forest — `session_entries` — is the only place that history lives.

That is why observability needs no per-harness work: a failed tool call has the
same shape whoever produced it, and the transcript pane's classifier
(`src/components/transcriptFacets.ts`) is rules over that vocabulary rather than
a list of provider quirks. `src/transcript/harnessBranchGate.test.ts` enforces
it — no component that draws a transcript row may branch on a harness id.

## What is stored, and at what visibility

Entries carry a `context_visibility`, and it decides three separate things:
whether the context projector may include the entry in a restored prompt,
whether the recall trigger indexes it for search, and nothing else. It does
**not** decide whether the entry is recorded.

| Visibility | Written | In model context | An FTS hit | Exported |
| --- | --- | --- | --- | --- |
| `eligible` | yes | yes | yes, if the kind is conversational | yes |
| `hidden` | yes | no | no | yes, unless `includeHidden: false` |

`hidden` is what lets the record be wider than the prompt. Turn boundaries
(`turn.started`, `turn.completed`), per-turn usage (`usage.updated`) and the
agent's plan (`plan.updated`) are recorded there. They used to be dropped
entirely, which meant a reloaded session could no longer say where a turn began
or what it cost — the two questions an observability read starts from.

### What is never stored

Streaming frames — `message.delta`, `reasoning.delta`, `tool.progress` — and
the `question.settled` control signal. A delta is worthless once its terminal
event arrives carrying the whole content, and storing both would duplicate
every message in the database. A settled thought keeps its full assembled text
under `reasoning.completed`; nothing about the thinking is lost, only the
keystroke-by-keystroke arrival of it.

`question.settled` is an instruction to `live_turn.rs` to resolve an existing
`approval.requested` row, not a thing that happened in the conversation.

## Reading it in the app

The **Transcript** dock pane shows the durable stream as frames. Each row
carries three derived facts:

- **Facet** — messages, thinking, tools, turns, usage, approvals, delegation.
  The chips count the loaded stream and filter it; the text box narrows further
  inside the selected facet.
- **Turn** — `t0`, `t1`, … counted from `turn.started` boundaries. Events before
  the first boundary are `t0`. The JSONL export uses the identical rule, so the
  pane and a file never disagree about which turn something was in.
- **Problem** — whether this is evidence something went wrong. The rules:

  | Condition | Reads as |
  | --- | --- |
  | `tool.*` / `command.*` with status `failed`/`error` | This tool call failed. |
  | `turn.*` with status `failed`/`error` | The turn ended in failure. |
  | `worker.result` whose status is not `completed` | A delegated worker ended `<status>`. |
  | kind `error` or `*.error` | The provider reported an error. |
  | kind `*.failed` or `*_failed` | This step failed. |

The problem count stays on screen whichever facet is selected. A failure a
reader has to go looking for is the one that ships.

## Getting it out: `sessions/export_session_transcript`

```jsonc
// params
{ "sessionId": "…", "scope": "forest", "includeHidden": true, "destinationPath": "/abs/path.jsonl" }
```

All but `sessionId` are optional. `scope` is `forest` (every entry, abandoned
branches included) or `active_branch` (the conversation as it currently reads).
Without a `destinationPath` the file lands in `<data_dir>/exports/`.

The result is a path, a line count, a byte count and a digest — never the
transcript itself. A long session is megabytes, which belongs on disk rather
than in a JSON-RPC frame.

### The file

Three record types, one per line:

```jsonc
{"type":"header","schemaVersion":1,"exportedAt":"…","scope":"forest","includeHidden":true,
 "session":{"id":"…","harness":"codex","model":"…","label":"…","status":"…","workspaceId":"…"}}
{"type":"entry","sequence":3,"entryId":"…","parentEntryId":"…","kind":"tool.completed",
 "createdAt":"…","contextVisibility":"eligible","turnIndex":1,"onActiveBranch":true,
 "role":null,"status":"failed","title":"cargo test","text":null,
 "data":{"exitCode":101},"providerMeta":{…},"payload":{…}}
{"type":"footer","entryCount":42,"counts":{"assistant.message":8,…},"digest":"sha256:…"}
```

Three properties are worth knowing:

- **Every line stands alone.** A payload containing newlines is escaped by the
  serializer, never emitted raw, so `head`, `tail`, `grep` and `jq -c` all work
  on a partially written or truncated file.
- **The digest covers the entry lines only**, so it is stable against the export
  clock: two exports of an unchanged session have the same digest, and a
  truncated file is detectable rather than merely short.
- **Payloads are copied verbatim.** Redaction already happened before the turn
  left the machine (`secret_interception`); re-deciding it here would make the
  export and the transcript disagree about what the session contained.

### Reading one from a shell

```sh
f=~/Library/Application\ Support/dev.bridge.deck/exports/<session>-<stamp>.jsonl

head -1 "$f" | jq '.session'                                   # what produced this
jq -r 'select(.type=="entry") | .kind' "$f" | sort | uniq -c   # the shape of the session
jq 'select(.status=="failed")' "$f"                            # every failure
jq -r 'select(.kind=="reasoning.completed") | .text' "$f"      # the thinking, in full
jq -s 'group_by(.turnIndex) | map({turn: .[0].turnIndex, events: length})' "$f"
```

## Searching it

`sessions/search_session_entries` is FTS5 over `session_entries`, scoped to one
`session_id` — there is no workspace-wide or account-wide MATCH. It indexes
conversational kinds only (`INDEXABLE_KINDS` in `session_recall.rs`) at
`eligible`/`visible` visibility, so a `hidden` control entry is in the export
but never a search hit: a reader searching their own words does not mean to find
a token count.

Results page. `hasMore` is answered by asking for one row more than the page
needs, not by checking whether the page came back full — the latter is wrong
exactly when the total is a multiple of the page size.

## Related

- [`session-forest.md`](session-forest.md) — the append-only history itself
- [`transcript-behavior-contract.md`](transcript-behavior-contract.md) — what
  the rendered conversation does, over normalized items and nothing else
- [`local-history.md`](local-history.md), [`compaction-and-resume.md`](compaction-and-resume.md)
