# Notes list load measurements, 2026-11-14

The Notes screen opens with three serial core reads: the list
(`notes/list_notes`), the tab strip's database (`notes/read_page`), and the open
tab's rows (`notes/query_database`). The third read dominated every open,
because it replayed one Yjs document per row to read cells the screen never
draws. It now asks for rows as pages (`notesOnly: true`) and opens no document.

Both halves are measured on a copy of a real store (77 pages, 67 rows in the
open tab) and on a seeded 600-page store.

## The screen, in a real browser

Chromium against the Vite build connected to a release `bridged` over its
WebSocket listener, with the store above. Time is from the click on Notes to
the 67th row painted, over six opens each, with the change stashed for the
"before" run.

| Open the Notes screen | Before | After |
|---|---:|---:|
| Median click to 67 rows | 1,494 ms | 59 ms |
| Best run | 1,356 ms | 53 ms |
| Worst run | 2,858 ms | 67 ms |
| `notes/query_database` | 652 to 816 ms | 2 to 5 ms |
| `notes/list_notes` | 5 to 17 ms | 6 to 14 ms |

The first after-run (751 ms) is Vite loading the screen's modules, not the read,
and is left out of the median.

## The reads, without a browser

`cargo run --release -p bridge-core --example notes_perf -- --store <data dir>`.
Medians over 15 runs on the same store:

| Read | Before | After |
|---|---:|---:|
| `notes/list_notes` | 0.2 ms | 0.2 ms |
| `notes/read_page` (tab strip) | 0.1 ms | 0.1 ms |
| `notes/query_database` | 2,106 ms | 0.5 ms |
| The three in series | 2,274 ms | 0.9 ms |

The document replay is what costs: the store's largest page is 52 KB of text
with a 1.2 MB save in its log, and replaying that one document took 0.7 to 2.0 s
depending on the run. The tab of 67 rows replays all 67.

On the seeded 600-page store (pages edited three times each), the same table
reads 155 ms before and 1.7 ms after for the tab query, and 141 ms against
4.5 ms for the three reads in series.

## Reproducing

```bash
# Core reads, seeded store
cargo run --release -p bridge-core --example notes_perf -- 600 15

# Core reads, a real store
cargo run --release -p bridge-core --example notes_perf -- --store ~/Library/Application\ Support/dev.bridge.deck
```

The browser numbers need a release `bridged`, `bunx vite --port 1421`, and a
Chromium run against
`http://127.0.0.1:1421/#bridge-daemon=ws://127.0.0.1:4401&bridge-token=<token>`
with the daemon started as
`bridged --listen 127.0.0.1:4401 --allowed-origin http://127.0.0.1:1421`.
