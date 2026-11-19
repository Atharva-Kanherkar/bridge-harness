# Size ceiling of a tool reply

Each reply of a Bridge tool goes through one limiter, `bridge_core::tool_limit`,
before the bridge MCP server sends it to the harness. The `tools/call` arm of
`VisualServer::handle` calls `bound_reply` one time for each reply. The loopback
route and `notes::tool::serve_for` return the full result.

## Classes and ceilings

The size of a reply is the byte length of its text block. Each class has one
ceiling:

- `List`, 32 KiB: `notes_search` (`pages`) and `memory_search` (`records`).
  The unit of `offset` is one item.
- `Document`, 32 KiB: `notes_read` (`markdown`). The unit of `offset` is one
  character (a Unicode scalar value).
- `Bounded`, 32 KiB: `chats_search`, `chats_read`, `settings_read`, `visualize`.
- `Write`, 32 KiB: each tool that changes data, and each tool with no entry.
- An error reply of each tool: 8 KiB.

The values are provisional. They change when the size distribution of a large
database is available. A new value is a change of one constant.

## Cut, next and offset

A reply at or below its ceiling does not change. Above the ceiling, the limiter
cuts the reply on a boundary of the structure: a list keeps whole items, and a
document ends after a line break, else at a character boundary. Two keys come
last in the object. `cut` tells the model the ceiling, the units kept and left
out, the tool and the next `offset`. `next` is `{"offset": N}`, where N is the
`offset` of the call plus the units kept. If the part with pages is whole and
only other parts are shortened, the reply gets `cut` and no `next`, because a
next call has nothing more to return. A `Bounded` or `Write` reply gets
`cut` and no `next`. Its `cut` says how to narrow the call, or that the change
is complete and that the agent must not send it again.

`notes_search`, `notes_read` and `memory_search` take an optional `offset`
(integer, minimum 0, default 0). Start with no `offset`, then send each
`next.offset` until a reply has no `cut`. If the data does not change, the
replies join to the full result with no gap and no repeat. `notes_search`
pages through the first 2,000 matches, or 500 on the database path. A page
that goes past that limit returns the matches up to it. An `offset` at or past
the limit is a 400 that says to narrow the search. The chat tools keep
the caps on their parts and counts, and their largest reply is below the
ceiling, so they have no `offset`.
