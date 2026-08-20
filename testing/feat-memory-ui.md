# feat/memory-ui

Memory becomes a surface, not a slash command. The ledger underneath is
unchanged and still thin: one writable scope, four kinds, tombstone deletes.
Nothing here injects a pin into a prompt.

## memory-changed

- `(MemoryChanged, "memory-changed", Transient)` — a refetch hint, like
  `learning-job-changed`. Payload is `{"scopeKey": …}` and nothing else; the
  authoritative list comes from `memory/list_memory_records`.
- Published after the write commits, from **both** paths: the `api` bodies
  (`save_memory_record`, `delete_memory_record`) and the `/pin` and `/unpin`
  arms in `live_turn`. A failed save publishes nothing.
- Reconciled after lag, like the other hints: a client that missed it while
  lagging still has to learn it must re-read.

## The dialog

- `modal === "memory"`. Lists active `account:local` records, saves, forgets.
- Header names the three-product boundary in prose: Bridge account memory on
  this machine, **not** this chat's history, **not** the helper picker, **not**
  provider `/memory`.
- Kind chips filter the list client-side over the four kinds. No chip means all.
- The composer refuses over `MAX_MEMORY_BODY_CHARS` (4000) with a visible count;
  the save button is disabled rather than clipping.
- Refetches on `memory-changed` while open. Every read takes a generation, and a
  read that is no longer the newest is dropped — a slow earlier read cannot
  clobber a newer one whichever resolves last. Same shape as
  `RouterSettingsDialog`'s `learningReadGeneration`.
- Closed keeps no state.

## Reachable from a direct chat

- The sidebar's Memory row is footer nav next to Projects/Marketplace/Settings.
  It is **not** gated on a workspace: account memory is not workspace memory, so
  a plain chat with no project reaches it identically.

## Remember this

- Assistant message bubbles only. Worker rows, tool cards, activity groups,
  reasoning, approvals, and the user's own bubble carry no button.
- `aria-label="Remember this"`.
- At or under 4000 characters: saves directly with the session id as
  `sourceSessionId`. The open dialog live-updates through `memory-changed`.
- Over 4000: opens the dialog with the **full** text pre-filled and issues no
  save. Never a clip, never a truncated write.

## Slash popover badging

- Each row's badge is keyed strictly off the catalog's `harness` field:
  `harness === "bridge"` is Bridge-local ("this Mac"), anything else is
  provider-owned and shows that harness's label.
- Never a provider-name comparison. An unrecognised future harness badges as
  provider-owned, not as local.

## memory/get_memory_capabilities

- Zero params. Answers what memory exists, so the dialog can be honest about
  what it does not own.
- Bridge half: the ledger exists, scope `account:local`, body limit, the four
  kinds.
- Provider half: derived from `slash::list_commands`, filtered to non-Bridge
  builtins with a memory-command name (`claude /memory`, `codex /memories`).
  Derived from the catalog, never from a hardcoded harness comparison in the
  API body — an unavailable adapter contributes nothing.

## Out of scope

Edit or supersede (save-then-forget is the only rewrite), workspace-scoped
writes, a "Memory used" chip — nothing is injected into any prompt yet — a
review queue, automatic extraction, provenance other than `user_explicit`,
search or FTS over the ledger, and cross-machine sync.
