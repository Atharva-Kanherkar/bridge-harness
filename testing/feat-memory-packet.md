# feat/memory-packet

Memory reaches a prompt through one gate — bounded, cited, audited, and
disclosable. Delivery is the frozen snapshot: the packet is a variable suffix
compiled at session start, restore, and worker spawn, for every harness alike.
One product, one audit writer. Mid-session pins apply from the next session.

## Schema 34

- `memory_retrieval_audits` lands beside its writer: one row per built packet
  with recipient session, objective hash, candidate count, selected ids,
  deterministic exclusion codes, and the serialized token estimate. Actual
  token usage stays in the usage ledger where it already lives.
- `memory_injection_settings`: per-scope enabled flag, default on. Off means
  no packet, and no audit row — the setting is the record of why.

## The packet

- Eligibility is honest to the schema that exists: only active records in
  scope enter; proposed, rejected, superseded, and tombstoned are the
  exclusion classes, each audited by code. A body carrying an envelope tag or
  a secret marker is excluded as unsafe rather than escaped into the prompt.
- Rank is deterministic: explicit pins first (newest first), then approved
  suggestions by confidence. Each selection carries a reason string.
- The budget is a hard character cap. Over budget degrades by dropping whole
  records — never truncating one mid-body — and the floor is no packet at all.
- The rendered packet names itself as the user's pinned account memory, cites
  every item by id, and states that it is not conversation instructions.
- The packet rides `variable_section("memory_packet", ...)` only. The stable
  prefix hash is bytewise unchanged when memory content changes.

## The wire and the chip

- `memory/get_memory_injection` and `memory/set_memory_injection` read and
  write the flag; the toggle lives on the Memory surface.
- `memory/get_packet_audit` returns the newest audit for a session: selected
  items with body, kind, and reason, plus the token estimate. Empty selection
  means no packet.
- The "Memory used (N)" chip above the composer is audit-backed — never
  inferred from events — and does not render at zero. Its disclosure lists
  each item and why it was selected.

## Boundaries restated

- `account_pins_are_not_session_recall` stays. Its sibling here:
  pins enter prompts only through the packet gate — the compile paths carry
  no other read of the ledger.

## Out of scope

Per-turn delivery over `application_context` (a follow-up once a harness
verifiably receives it — the Claude adapter's trait default silently drops
per-turn context today), a one-turn disable flag (nothing per-turn exists to
gate), FTS-ranked objectives, workspace scopes, TTL, and the evaluator.
