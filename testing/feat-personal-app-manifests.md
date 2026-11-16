# Personal app manifests contract

Slice 2 of the personal MCP apps work. Core stores personal apps as versioned,
hash-locked manifests. A person locks a draft by its id and content hash.

## Functional behavior

- A personal app manifest names one MCP server on one harness, its read tools,
  its write tools, its view HTML, a name and an intent. Validation refuses: an
  id that is not kebab-case or is longer than 40 characters; a blank or overlong
  name or intent; a harness other than `claude`; a blank server; no tools, or
  more than 40; a tool name that is not a bare lowercase `[a-z0-9_-]` name; a
  tool listed twice or as both read and write; a read that
  `briefing_policy::is_read_tool_name` does not call a read; an empty view or a
  view over 256 KiB.
- The content hash is `sha256:` over the canonical JSON of the manifest with
  tool lists sorted, so the same app always hashes the same and any change to
  name, intent, harness, server, tools or view changes it.
- `create_draft` stores a validated draft (for slice 3) and returns its id and
  hash. Drafts older than 24 hours are refused and pruned.
- `apps/lock_personal_app {draftId, contentHash, allowWider}`:
  - refuses an unknown or expired draft, and a hash that does not match the
    stored draft (the person locks only what they previewed);
  - for a new app id, stores version 1 and reports `locked`;
  - for an existing app whose latest version has the same hash, stores nothing
    and reports `unchanged`;
  - for an existing app, stores version n+1 when every read and write was
    already in version n; when the draft adds any tool or moves a read to a
    write or back, it reports `widens` with the added tools and stores nothing
    unless `allowWider` is true;
  - a harness or server change counts as widening;
  - consumes the draft on `locked` and `unchanged`, keeps it on `widens`.
- `apps/list_personal_apps` returns the latest version of each app, newest
  lock first. `apps/get_personal_app {id}` returns its latest version or a
  not-found error. `apps/delete_personal_app {id}` removes every version and
  every draft for it and is idempotent.
- Lock and delete publish `apps/changed`.
- Protocol 1.38: a 1.38 client refuses a 1.37 daemon; a 1.37 client still pairs
  with a 1.38 daemon.

## Unit tests

- Validation: one case per refusal above, plus a valid app.
- Hash: stable across tool order, changes with each field.
- Lock: new, unchanged, narrowed, widened refused, widened allowed, server
  change, harness change, hash mismatch, expired draft, draft consumed.
- List order, get not found, delete idempotent and removes drafts.
- Migration 81 adds both tables.
- Wire types round-trip, params refuse unknown fields, camelCase names.
- Dispatch: each method reaches core; list takes no params.

## Verification

- Regenerate protocol artifacts; `cargo test -p bridge-protocol`.
- `bun run build` and `bun run test` before the PR.
- No UI in this slice; reachability comes with slices 3 and 4.
