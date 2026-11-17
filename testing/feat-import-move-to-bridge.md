# Test contract: Move to Bridge

Branch `feat/import-move-to-bridge`, stacked on `feat/import-claude-v2`. Issue #1052, epic #1050.

A person finds the tools already on their Mac, reviews what each holds by
project, imports in batches, and reads what came over and what did not, with
no file picker on the happy path.

## Backend

| # | Expectation | Test |
|---|---|---|
| B1 | `imports/detect_import_sources` lists all seven tools, Claude Code first, from folder listings only | `import_sources::tests::a_fresh_machine_lists_every_tool_as_not_found` |
| B2 | Claude counts cover every config folder (home, accounts); subagent files and temp-folder probes are not counted as chats; temp-folder sessions are counted apart as background runs | `import_sources::tests::counts_come_from_file_listings_across_every_claude_folder` |
| B3 | ADEs are found by their worktree folders; Codex rollouts are counted without being read | `import_sources::tests::ades_are_found_by_their_folders_and_count_workspaces` |
| B4 | Discovery gives each chat a title, folder and start time from a 32 KiB peek | `claude_import::transcript::tests::peek_reads_title_folder_and_start_without_the_whole_file` |
| B5 | Commit re-reads only the artifacts named in `artifactIds`, and refuses one the discovery did not find | `api::tests` (existing commit tests, now with `artifact_ids`) |
| B6 | Commit writes in batches of 25 so the database lock is held one batch at a time; a chat and its subagents never split across batches | `import_sources::batch_tests::batches_keep_families_together_and_cover_every_selected_chat` |
| B7 | Preview sends counts, not message bodies, so a big import never exceeds the daemon's 32 MiB message limit | `api::preview_summaries`, exercised by the fake user |

## Frontend

| # | Expectation | Test |
|---|---|---|
| F1 | Found list: counts per tool, Review only where Bridge imports today, "Coming" and "Not found" otherwise, notes that ADE chats come over with Claude Code | `MoveToBridge.test.tsx` "lists every tool…" |
| F2 | Review groups chats by folder, newest first; per-project switch; long projects collapse | "reviews by project, imports 30 chats in two batches…" |
| F3 | Background runs from temp folders sit in their own group, last and unselected | `moveToBridge.test.ts` |
| F4 | Import goes 25 artifacts at a time; each commit names exactly the artifacts its preview read | `MoveToBridge.test.tsx` |
| F5 | A failed batch is reported and the rest still import | "keeps going after a batch fails" |
| F6 | Done shows came over, already in Bridge, changed, could not be imported, and what did not come over by reason | `moveToBridge.test.ts` |
| F7 | Images stored on a user message render after a reload or an import | `conversation.test.ts` "reads images a stored forest entry keeps under data" |
| F8 | Existing folder and JSONL picker flow is unchanged | `ImportHarnessSection.test.tsx` |

## Fake user (`scripts/e2e/import-fake-user.mjs`)

A scripted person drives the real app in a browser tab attached to a real
`bridged` whose home is built by `scripts/e2e/import-fake-home.py`. Every step
asserts what is on screen and the run is recorded. It must pass these checks
in order: setup completes; the Found list shows Claude Code with 5 chats in 3
projects, 1 memory and 1 setup item, Conductor with 3 workspaces in 2 repos,
Codex as coming and absent tools as not found; review groups by folder and
shows the /rename title; all 5 chats start selected; deselecting one leaves 4;
importing brings over 6 items (4 chats, the Explore subagent, the memory) with
0 already present; the imported chat carries its source badge, the pasted
screenshot, the thinking row, the tool group with the failing command, the
answer and the compaction summary; the real Claude Code session renders; a
second import brings over only the chat left behind and recognises all 6.

## Review amendments

Added after review of #1066. Each row names the regression test that failed before its fix.

| # | Expectation | Test |
|---|---|---|
| C1 | A Claude config folder with any name (`CLAUDE_CONFIG_DIR=~/.claude-work`) is read as a config home: the chats detection counts are the chats review lists | `claude_import::tests::a_custom_config_folder_is_read_as_a_claude_home` |
| C2 | Selected memories import together: the quick flow imports them alongside existing ones instead of letting the first block the rest | `MoveToBridge.test.tsx` "imports independent memories alongside each other", `external_import::tests` "independent memories import alongside" |
| C3 | What did not come over includes the transcript records the reader skipped (documents, orphan results, unreadable lines), with counts; repeats it removed are not losses | `moveToBridge.test.ts` "reports skipped transcript records" |
