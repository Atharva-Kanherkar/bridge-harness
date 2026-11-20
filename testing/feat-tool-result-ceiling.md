# feat/tool-result-ceiling: Test Contract

Each reply of a Bridge tool passes one limiter before the MCP helper sends it
to the harness. A reply above the ceiling of its class is cut on a boundary of
the structure, says so, and names the argument that gets the rest. Locked
before implementation. Plan note: the comment "Plan note: size ceiling and
paging for Bridge tool results (T2)" on the issue of this change.

## Functional Behavior

### The limiter (`bridge_core::tool_limit`)

- Pure: no I/O, no clock, no database. The same input gives the same output.
- `bound_reply(tool, arguments, reply)` takes the MCP reply of one
  `tools/call` and returns the reply that the harness gets.
- The size of a reply is the byte length of its text block. For a forwarded
  tool, the text block is `serde_json::to_string_pretty` of the result.
- Each tool has one class. Each class has one ceiling constant:

| Class | Ceiling | Tools | Part with pages | Unit of `offset` |
| --- | --- | --- | --- | --- |
| `List` | 32 KiB | `notes_search`, `memory_search` | `pages`, `records` | items |
| `Document` | 32 KiB | `notes_read` | `markdown` | characters |
| `Bounded` | 32 KiB | `chats_search`, `chats_read`, `settings_read`, `visualize` | none | none |
| `Write` | 32 KiB | `apps_draft`, `apps_open`, `notes_create`, `notes_edit`, `notes_append`, `notes_draw`, `notes_set`, `memory_propose`, `outgoing_draft`, `settings_change`, `notify_user` | none | none |

- An error reply (`isError: true`) of each tool has one ceiling of 8 KiB.
- The values are provisional. They change when the size distribution from a
  large database is available. A new value is a change of one constant.
- `limit_of(tool)` is `Some` for each name in `all_definitions()` and for
  `visualize`. `bound_reply` gives a tool with no entry the `Write` limit, so
  no reply leaves without a ceiling.
- **At or below the ceiling**, `bound_reply` returns the reply with no change:
  no new field, the same key order, the same bytes.
- **Above the ceiling, `List`**: the reply keeps the first `k` items of the
  list. `k` is the largest count for which the final text is at or below the
  ceiling. An item is never cut between two of its fields. If the first item
  alone does not fit, the reply keeps that one item with its longest texts
  shortened, and `cut` says so. `next.offset` always moves on by at least 1.
- A `List` reply never leaves out an item with no notice. If one item is above
  the ceiling when each of its texts and lists is empty, the reply keeps no
  item, and `cut` says that the item at that `offset` is too large for one
  reply.
- **Above the ceiling, `Document`**: the reply keeps the longest start of the
  text that fits and that ends directly after a line break. If no line break
  is in that range, the cut is at a character boundary. A cut is never inside
  a UTF-8 sequence. The reply keeps at least one character.
- The text of a `Document` never loses a character between two pages. If the
  parts with no pages cannot become small enough, the limiter removes them,
  and `cut` names them. It removes first the part that stays largest after
  the limiter shortens that part, so a part with only one long text stays.
- A cut reply of a tool with pages gets two new keys, last in the object:
  `cut` (text for the model) and `next` (`{"offset": N}`). `N` is the `offset`
  of the call (0 when the call has none) plus the count of units in the reply.
- If the part with pages is whole and only other parts are shortened, the
  reply gets `cut` and no `next`. A next call has nothing more to return.
  The same rule applies when a list has one item and the limiter shortens
  that item.
- A `notes_read` reply of a database that gets `cut` and no `next` says that
  `notes_search` with `database` pages the rows.
- `cut` says: that the reply is cut, the ceiling in bytes, how many units the
  reply holds, how many units of this call are left out, the name of the tool,
  and the `offset` value for the next call.
- **Parts with no pages** (for example `rows`, `tasks` and `database` of a
  `notes_read` reply): if the reply without the content of the part with
  pages is larger than half of the ceiling, the limiter shortens those parts
  to half of the ceiling. The part with pages gets the remaining space.
- **`Bounded` and `Write`** have no part with pages. Above the ceiling, the
  limiter shortens the result down to the ceiling and adds `cut` with no
  `next`.
- **How the limiter shortens a part with no pages**: it first removes full
  items from the end of the largest lists and keeps at least one item in each
  list. If that is not enough, it shortens the longest texts (at a line break,
  else at a character boundary) and keeps at least 256 bytes of each text. If
  that is not enough, it removes the remaining items. If that is not enough,
  it shortens each text below 256 bytes, down to an empty text. The last
  fallback is an object that holds only `cut`.
- `cut` names each shortened part by its JSON pointer with the kept amount and
  the full amount. If the shortened parts are more than 8, `cut` names the
  first 8 and gives the count of the others.
- `cut` of a `Bounded` tool says how to narrow the call (`settings_read`:
  pass `search`. `chats_search` and `chats_read`: use a narrower `query`).
- `cut` of a `Write` tool says that the change itself is complete and that the
  agent must not send it again.
- **A reply with no `structuredContent`** (an error, a `visualize` summary):
  the limiter keeps the longest start of the text that fits (at a line break,
  else at a character boundary) and adds one last line that says how many
  bytes the full text has. `isError` does not change.
- The final text is at or below the ceiling for every input.
- `structuredContent` of a bounded reply holds the same bounded value as the
  text block.
- The work of the limiter does not grow with the square of the result size.

### Where the limiter runs

- The `tools/call` arm of `VisualServer::handle` calls `bound_reply` one time
  for each reply: a forwarded tool, `visualize`, success or error.
- `notes::tool::serve_for` and the loopback route do not change. They return
  the full result.
- No tool cuts its full result on its own. `history_tools::fit` and
  `MAX_RESULT_CHARS` do not exist after this change. The character caps of the
  parts of a `chats_*` reply stay (`snippet`, `first`, `summary`, `recent`,
  `windows` text), and so do the caps on the counts of chats and windows.

### The `offset` argument

- Optional integer, minimum 0, default 0, on `notes_search`, `notes_read` and
  `memory_search`. The JSON schema of each of the three tools has it, with a
  short description. A value that is not a non-negative integer is a 400.
- The tool descriptions do not change. The tool order of `tools/list`, the
  limits of `limit`, the `required` lists and `additionalProperties: false`
  do not change.
- No other tool declares `offset`. Each other tool treats `offset` as it
  treats each unknown argument.
- `notes_search`, plain path: skips the first `offset` matches of the same
  ordered list (`rank` for a text query, then `updated_at DESC, id DESC`). The
  notes service returns a largest list. A call that goes past it returns the
  matches up to that limit. If `offset` is at or above that limit, the call is
  a 400 that says to narrow the search.
- `notes_search`, database path: the same rule with the limit of a database
  query. `total` stays the total of the query.
- `notes_read`: `offset` counts characters (Unicode scalar values) of
  `markdown` as the tool returns it (drawings are markers). A call with
  `offset` above 0 returns only `page`, `markdown` and `note`. An `offset` at
  or after the end returns empty `markdown`.
- `memory_search`: skips the first `offset` records in the order
  `rank, updated_at DESC, id`.
- A call with no `offset`, or with `offset` 0, returns the same bytes as
  before this change.

### No gap and no repeat

For each of the three tools, and for a database that does not change between
the calls: start with no `offset`, then call again with `next.offset` until a
reply has no `cut`.

- `notes_read`: the joined `markdown` texts equal the full `markdown`.
- `notes_search` and `memory_search`, with a `limit` at or above the count of
  matches: the joined items equal the items of the result with no limiter, in
  order, each item one time.

### Same bytes below the ceiling

- For each tool, a reply at or below its ceiling has the same bytes as the
  reply of the base branch.
- One intended difference: a `chats_*` reply above 6,000 characters lost items
  with no notice on the base branch. It comes back whole now, because the
  largest `chats_*` reply is below the ceiling.

### Before and after figure

- A test builds one fixture above the ceiling for each of the three tools with
  pages and prints one table: tool, bytes before, bytes after, and the T1
  estimate (`entry_tokens::estimate` on a `tool.completed` payload that holds
  the reply text) before and after. The PR shows that table.

## Unit Tests

`tool_limit::tests`:

- `a_reply_below_the_ceiling_is_returned_unchanged`
- `a_reply_exactly_at_the_ceiling_is_returned_unchanged`
- `a_list_over_the_ceiling_keeps_whole_items_and_says_how_to_continue`
- `a_document_over_the_ceiling_is_cut_after_a_line_break`
- `a_cut_never_splits_a_multi_byte_character`
- `a_line_longer_than_the_ceiling_is_cut_at_a_character`
- `nested_json_inside_an_item_is_never_cut`
- `a_single_item_over_the_ceiling_is_shortened_and_the_offset_moves_on`
- `an_item_with_many_texts_is_kept_with_shorter_texts`
- `an_item_that_cannot_fit_is_named_in_the_notice`
- `a_document_keeps_each_character_when_other_parts_are_too_large`
- `a_whole_part_with_pages_gets_no_next`
- `a_lone_shortened_item_gets_no_next`
- `a_part_that_only_has_a_long_text_stays_when_parts_are_removed`
- `a_database_with_no_text_names_notes_search_for_its_rows`
- `a_document_page_ends_at_the_last_line_break_that_fits`
- `large_parts_without_pages_are_shortened_and_named`
- `a_bounded_reply_over_the_ceiling_loses_whole_items_first`
- `a_write_reply_over_the_ceiling_says_the_change_is_complete`
- `an_error_text_over_its_ceiling_is_cut_and_stays_an_error`
- `a_text_reply_with_no_structure_is_cut_with_a_last_line`
- `the_notice_names_the_tool_the_ceiling_and_the_next_offset`
- `the_bounded_text_is_never_over_the_ceiling` (many shapes and sizes, made
  without a random source)
- `the_structured_content_equals_the_text_block`
- `pages_of_a_list_join_to_the_source`
- `pages_of_a_document_join_to_the_source`
- `a_tool_with_no_entry_gets_the_write_limit`
- `every_bridge_tool_has_a_size_class` (fails when a new tool has no class,
  and when the table names a tool that does not exist)
- `the_before_and_after_figures` (prints the table)

`mcp_apps::server::tests`:

- `a_large_forwarded_reply_is_cut_at_the_reply_boundary`
- `a_large_visualize_refusal_is_cut_and_stays_an_error`
- `a_small_reply_of_each_tool_keeps_its_bytes`

`mcp_apps::notes_tools::tests`:

- `the_three_paged_tools_declare_offset_and_stay_strict`
- The pinned tests stay green with no change of their assertions:
  `history_definitions_are_read_only_and_strict`,
  `every_listable_tool_has_a_definition_and_back`, the test of the tool order.

`notes::tool::tests`:

- `notes_search_pages_with_offset_without_a_gap_or_a_repeat` (the plain path
  and the database path)
- `notes_read_with_offset_returns_the_rest_of_the_text`
- `a_later_page_of_notes_read_holds_only_the_text`
- `an_offset_past_the_end_returns_empty_markdown`
- `a_call_with_no_offset_reads_the_same_bytes_as_before`
- `an_offset_past_the_service_limit_is_refused`
- `a_page_near_the_service_limit_returns_the_matches_up_to_it`
- `a_bad_offset_is_refused`

`memory_tools::tests`:

- `memory_search_pages_with_offset_without_a_gap_or_a_repeat`
- `a_bad_offset_is_refused`
- `an_offset_past_the_end_returns_no_records` (also an `offset` above the
  32-bit range)

`history_tools::tests`:

- `the_largest_reply_of_each_chat_tool_is_below_the_ceiling` (replaces
  `output_is_bounded`. It uses text with the longest JSON escape.)

`settings_tools::tests` or `tool_limit::tests`:

- `the_largest_default_reply_of_each_read_tool_is_below_its_ceiling`
  (`notes_search` with 20 summaries and previews of 280 characters of 3
  bytes, `settings_read` with no key)
- `the_largest_default_memory_search_reply_pages_in_whole_records` (10
  records with bodies of 4,000 characters are about 40 KB, above the ceiling.
  The reply keeps 7 or more whole records, and `next.offset` is the count of
  the kept records.)

## Integration / Functional Tests

- `mcp_apps::server::tests::a_large_note_pages_through_the_helper_and_the_route`:
  the helper with a real `BridgeCore::for_tests` behind the loopback route. A
  note above the ceiling comes back with `cut` and `next`. The next calls
  continue it. The joined text equals the source. The text block of each reply
  is at or below the ceiling.
- `history_tools_eval::offline` stays green: no change of retrieval and no
  leak.
- `apps::builtin::tests::every_bridge_tool_is_owned_by_one_app_or_core` stays
  green.

## Smoke Tests

- `bun run build`
- `bun run test`
- `node scripts/clippy-ratchet.mjs`

## E2E Tests

None is new. This change needs no model judgement, so it has no live eval. The
live part of the chat history eval does not change.

## Manual Tests

- `bun run tauri dev`. Make a note longer than 32 KiB. Ask a chat to read it.
  The first tool card shows `cut` and `next`, and the chat calls `notes_read`
  again with `offset`.

## Out of Scope

- The native tools of a harness (shell, file reads).
- Connector results in a live chat. Bridge does not see them.
- The browser shell tools. Their output is native shell output with its own
  caps.
- A summary of a result by a model.
- The description of each tool and the instructions of the server.
- An `offset` for `chats_search` and `chats_read`. Their ranked lists are not
  stable between two calls, and their replies are below the ceiling.
- A lower ceiling for one harness. Each harness gets the same reply.
