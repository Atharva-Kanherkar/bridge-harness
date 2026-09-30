# Why vague search recall is bounded without a model

Measured on the labelled corpus, 2026-09-30. This is the evidence behind the
recall numbers in `chat-search-measurements.md`, kept separately because it is
the part a future change most needs to not re-derive.

## The corpus is built to be adversarially lexical

`testing/fixtures/chat_search/corpus.json` gives every background chat a title
of the form `Refactor the <module>` and keys that repeat the module name, so a
query's surface words are always present in chats that are *not* the answer:

| Query | Decoy cluster | Real target |
|---|---|---|
| `bill printout missing a row` | 8× `… the billing` | `target-17` Invoice PDF rendering |
| `login redirect for the desktop app` | 8× `… the login` | `target-26` OAuth PKCE flow |
| `sidebar overlay stays on phone` | 8× `… the sidebar` | `target-19` Mobile drawer scrim |

It also gives ten labelled targets a same-topic twin at another time
(`twin-NN`), so a query with no time phrase has two equally correct answers.

## Eight of the fifteen vague queries share no word with their answer

Checked token by token against the target's title, keys and digest:

| Query | Expected | Overlap |
|---|---|---|
| password prompt every time | target-15 Keychain ACL reset | none |
| bill printout missing a row | target-17 Invoice PDF rendering | none |
| old toggles everywhere in code | target-21 Feature flag cleanup | none (`old` ≠ `older`) |
| making the bot think faster | target-22 Chess engine move ordering | none |
| messages not delivered to customers | target-23 Email bounce handling | none |
| login redirect for the desktop app | target-26 OAuth PKCE flow | none |
| money tracking sheet | target-28 Budget spreadsheet formulas | none |
| uploads taking forever to process | target-29 Video transcoding queue | none |

Each needs a bridge only a language model has: `bill`→`invoice`,
`money`→`budget`, `toggles`→`flags`, `bot`→`engine`, `messages`→`email`.

**Consequence.** T1's lexical ceiling on this corpus is 25 (exact + time) plus
the 7 vague queries that do overlap, or **32/40 = 80%**. The remaining 8 points
have to come from T2, so the funnel target of ≥ 90% is only reachable if the
model stage answers at least 4 of those 8. A retrieval change alone cannot.

## Porter stemming was measured and rejected

`porter` is available in the bundled SQLite and was tried against every case
above. It does not close the gap:

- `older` and `old` stay **separate terms** (Porter only strips a trailing
  `E`, not `ER`), so `old toggles everywhere in code` still misses
  `remove flags older than ninety days`.
- `billing` stems to `bill`, which pulls `bill printout missing a row`
  *further* toward the eight decoy chats, not toward `Invoice PDF rendering`.
- It does help words that already match (`fields`→`field`, `failing`→`fail`),
  but every query it helps is one the index already gets right.

Cost would be a full re-tokenize of `session_entry_fts` and `chat_digest_fts`
on every existing database — 3.8 GB on the measured real store — plus a change
to per-chat `session_recall` behaviour, for no recall gain. Rejected.

## The index's one recoverable miss, and a fix that did not pay

`sidebar overlay stays on phone` is the single recoverable miss. The decoys
match the *common* term `sidebar` three times plus a digest; `target-19`
matches the *rare* term `overlay` once, in an entry the digest does not carry.
The multi-hit bonus counts repeated rows, so three repeats of a common word
beat one hit on a rare word.

Weighting each chat by the rarity of the terms it matched
(`1/(1+document_frequency)`, summed per chat, normalised by the query's total
rarity) was implemented and measured. It moved `target-19` from 0.021 to 0.051
— closing 89% of the gap — but it did **not** change recall (31/40) or
confidence (22/40), because the eight `… the sidebar` decoys also hold a digest
match, which is worth the same order as the whole bonus. Reverted rather than
shipped: complexity that moves no measured metric is not worth its maintenance.

Closing it properly needs the digest to carry more of the chat's own
vocabulary. That is a real design change (a keyword-extraction step inside a
trigger that runs on every entry write) and it would still only reach 32/40,
because no amount of extra chat vocabulary invents the word `invoice` from
`bill`. It was not worth the trigger cost for one query.


## The confidence gate has an honest ceiling

`resolved without the model` is 22/40. The reachable ceiling is **25/40
(62.5%)**, not the > 70% the PR originally projected:

- `exact` 12/15 — the other 3 are `target-00`/`twin-00` style pairs
  (`plugins catalog stall`, `kanji flashcards`, `subagent rate limit`). Without
  a time phrase these are genuinely ambiguous, and opening one at random on
  Enter would be a coin flip. The index is right to stay unsure.
- `time` 10/10 — the window separates each twin.
- `vague` 3/15 at best. The 7 lexically reachable ones include three more twin
  pairs (`game character animation images`, `sound delayed with headphones`,
  `tests failing randomly in ci`); the 8 unreachable ones can never be
  confident because the index is answering about the wrong chats.

Reaching > 70% would need either a time signal the user did not give, or
semantics in the index. The gate must also keep `confident but top hit wrong`
at 0, which is the invariant that stops Enter opening a wrong chat.
