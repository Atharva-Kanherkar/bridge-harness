# Adding a connector family

The connector surface renders notifications from any MCP server the harness has
authenticated. Slack is the one that ships with an inbox; this is how to add the
next one.

The design rule, and the thing to preserve: **everything that differs between
one product and the next lives in one table.** Nothing else in the feature
matches on a family. A test enforces it —
`connector_surface::tests::no_module_outside_the_registry_singles_out_one_family`
reads the production half of every connector module and fails if any of them
names a family, in either the enum or the string form. If you find yourself
wanting `if family == Gmail` somewhere, that is the test telling you the
behaviour belongs in the registry instead.

## The short version

Five edits, two files. Every one of them is demanded by the compiler or by a
test, so you cannot finish half a family and not find out.

| # | Where | What |
| --- | --- | --- |
| 1 | `work_connectors.rs` | `ConnectorFamily` variant, `ALL`, `as_str()` |
| 2 | `work_connectors.rs` | `allowed_hosts()` — permalink hosts |
| 3 | `work_connectors.rs` | `source_activity_at()` — which field is the date |
| 4 | `work_connectors.rs` | `resolve_evidence()` — which field is the identity |
| 5 | `connector_surface.rs` | the `profile()` row |

Edits 1–4 are provenance, and they are per-product because every product
identifies and timestamps things differently — there is no registry row that
could answer "is this a `ts` or an `internalDate`" for you. Edit 5 is everything
else.

That is genuinely the whole list. It was verified by adding a throwaway family
and running the suite: with those five in place, ingress prompts, render
prompts, action prompts, availability, the dock pane, the toasts, the refresh
control and the approval sentences all work in the new family's own vocabulary,
with no frontend change at all.

## Steps 1–4 — the family exists and can be cited

`ConnectorFamily` lives in `work_connectors.rs`, because provenance is the older
and stricter concern: a family is only ever offered to a model when Bridge can
say, on its own, where a result came from.

```rust
pub enum ConnectorFamily {
    Slack, Gmail, GitHub, Linear, Notion,
    Discord,                                  // ← new
}
```

Then four things about it:

```rust
// as_str + ALL — the wire name and the iteration order.
ConnectorFamily::Discord => "discord",

// allowed_hosts — an external link is produced only when Bridge resolved it
// *and* its host is on this list.
ConnectorFamily::Discord => &["discord.com"],

// source_activity_at — the provider's own activity time. Observing an old item
// must never renew its lifetime, so this reads the product's field, not a clock.
ConnectorFamily::Discord => &["timestamp", "edited_timestamp"],

// resolve_evidence — the field this product identifies things by. Getting the
// grain wrong mints two canonical ids for one resource; there is no forgiving
// `a.or(b)` here on purpose.
ConnectorFamily::Discord => ("discord.message", text("id")),
```

`every_family_declares_at_least_one_host` and the two non-exhaustive `match`
arms fail until all four are present.

## Step 5 — the registry row

In `connector_surface.rs`, `profile()`. This is the whole of it:

```rust
ConnectorFamily::Discord => ConnectorProfile {
    family,
    display_name: "Discord",
    inbox: Some(InboxProfile {
        attention_items:
            "direct messages, @-mentions of the account owner, and replies in threads \
             the owner is part of",
        direct_label: "direct message",
        mention_label: "mention",
        thread_label: "thread reply",
        container_noun: "channel",
        reply_verb: "send",
        reaction: Some(ReactionProfile { noun: "reaction", default_token: "eyes" }),
    }),
    no_inbox_reason: None,
},
```

Write the words the product uses, not Slack's. They are interpolated straight
into prompts, so `container_noun: "mailbox"` and `reply_verb: "reply"` is what
makes a Gmail ingress run read like an instruction about email instead of a
confused one about channels. `reaction: None` removes the one-click
acknowledgement rather than offering one the product does not have.

If the family should *not* have an inbox, set `inbox: None` and give
`no_inbox_reason` a sentence a user can act on. That sentence is what the pane
shows, so "not supported" is not good enough — say whether it is not written yet
or served better elsewhere, the way GitHub's row does.

## Then run the tests

```bash
cargo test --manifest-path src-tauri/Cargo.toml -p bridge-core --lib connector
```

Four tests exist specifically to catch a half-finished family:

| Test | Catches |
| --- | --- |
| `every_family_has_a_registry_row_that_describes_itself` | A missing row, or one with neither an inbox nor a reason |
| `every_inbox_row_is_fully_populated` | A blank field, which would put an empty word in a prompt |
| `every_prompt_is_written_in_the_familys_own_vocabulary` | A prompt that stopped reading the registry |
| `no_module_outside_the_registry_singles_out_one_family` | A new `if slack` anywhere in the feature |

Then add a replay fixture and an eval pass — see
[`testing/evals/connector-render.md`](../testing/evals/connector-render.md). A
family with no recorded render run has never been shown to work.

## What you do *not* have to touch

The frontend. It reads `family`, `displayName`, `hasInbox` and `harness` off the
wire and keeps no table of its own, so a new family appears in the pane, the
dock badge, the refresh control and the toasts without a line of TypeScript. The
one exception is cosmetic: `FAMILY_ICON` in `ConnectorToasts.tsx` maps a family
to a glyph and falls back to a generic message icon, so adding an entry is
optional polish rather than a requirement.

## Adding a card block kind

Rarer, and deliberately more work, because the closed block set is the reason a
connector message can never become markup — it is why this surface needs no
iframe and has no sandbox to get wrong.

1. `connector_surface::CardBlock` — the core variant, plus its `validate()` arm
   (every field needs a length bound).
2. `bridge_protocol::messages::connectors::ConnectorCardBlock` — the wire twin.
3. The block list in `connector_runs::render_prompt`, so a run knows it exists.
4. The `Block` switch in `ConnectorPane.tsx`.
5. Regenerate: `cargo run -p bridge-protocol --bin generate-protocol-artifacts`.

The compiler demands 1, 2 and 4; `tsgen::checked_in_artifacts_match_the_contract`
demands 5. Only step 3 is on you to remember.

**Older clients are safe.** A block kind a client does not know renders through
`UnknownBlock`, which prints whatever readable strings the block carries rather
than dropping it. A newer host talking to an older client loses formatting, not
content.

## Adding a harness

`connector_runs_live::discover_harness_connectors()` returns what each harness
reports having connected. Today only Claude exposes an MCP inventory to Bridge,
so the list has one entry; when Codex or OpenCode grow an equivalent of
`claude mcp list`, append a row.

Nothing downstream needs changing. Availability already carries *which* harness
owns each connection, and `run_profile` sends the run there — because a
connector run has to go to the harness whose own configuration holds the
credential, and sending it to a "better" one sends it somewhere that cannot see
the account at all.

## What a new family still cannot do

Two limits are deliberate, not oversights:

- **No auto-actions from inbound content.** Every write is a human approval on
  the literal text. A family cannot opt out of that.
- **No raw HTML.** Blocks only. A family that wants a richer view adds a block
  kind, which is reviewed once, rather than a rendering escape hatch that is
  reviewed never.
