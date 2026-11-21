# Contract: the Changes badge and pane count what the chat changed

Branch `feat/chat-scoped-changes`. Plan note:
`bridge://p/01a1ff44-0c98-7603-9ab6-b42eddf0ef98`.

The header's Review badge and the Changes pane used to count the folder's
whole `git status` against HEAD. In a shared checkout that is every chat's,
every agent's, and the person's own work, plus whatever was left lying
around: one chat in `~/kairo` read "1817 files +291222". They now count what
this chat's own turns changed, and keep the folder's uncommitted changes one
click away.

## Snapshots

1. At every `turn.started` and `turn.completed`, Bridge writes the chat
   folder's working tree, as `git add -A` sees it, to a tree. The write goes
   through a private index per chat (`GIT_INDEX_FILE` under
   `<git-dir>/bridge-checkpoints/`), so the person's index, HEAD, branch and
   working tree are never modified.
2. Each snapshot is committed onto the chat's own chain at
   `refs/bridge/chats/<session>`. The commit message records the phase, the
   turn id, and (for an end) the paths that differ from the chain's previous
   snapshot, both sides of a rename.
3. Snapshots run on one background worker, in arrival order, never under the
   database lock.
4. Untracked files over 20 MiB are left out. A snapshot that runs past 30 s is
   abandoned.
5. Gitlinks (nested repositories) never count as a chat's change.
6. A folder that is not a Git repository is skipped without error.

## What a chat changed

7. Its paths are the union of every recorded end's paths, plus, while a turn
   is running, the paths changed since that turn's start.
8. Its changeset is the diff from its first snapshot to a live one, kept to
   those paths, in the same `WorkspaceChangesResult` shape as the folder's.
   An edit later undone shows no change.
9. Edits made between the chat's turns by anyone else stay out unless they
   land on a path the chat also changed. Another writer editing the folder
   *during* a turn cannot be told apart from the turn.
10. Totals (`additions`, `deletions`) cover every changed file, including any
    past the 100-file window.

## Wire

11. `workspaces/session_changes {sessionId, includePatches?}`, protocol 1.49.
    A 1.48 client still pairs with a 1.49 daemon.
12. Unknown params are a `-32602` client error.
13. The result is `{available, unavailableReason, turns, since, additions,
    deletions, changes}`. `unavailableReason` is one of:
    - `no_folder`: the chat has no folder.
    - `not_git`: the folder is not a repository.
    - `no_turns`: nothing has been recorded since this shipped.
    - `snapshot_failed`: the live snapshot failed.

## Interface

14. The Review badge shows the chat's file count and totals, labelled as the
    chat's.
15. When the chat changed nothing, the badge hides, however dirty the folder
    is.
16. Until the chat's first answer arrives, the badge shows nothing rather
    than flashing the folder's count.
17. When the chat cannot be measured (any `unavailableReason`, or a daemon
    without the method), the badge falls back to the folder's count,
    labelled as the folder's.
18. The dock's Changes tab count matches the badge.
19. The Changes pane opens on **This chat**, with **All uncommitted** next to
    it. Switching never shows one scope's files under the other's heading.
20. The chat scope states its basis: turns in this chat, and how long ago the
    baseline was taken.
21. A chat that cannot be measured explains why and offers **Show all
    uncommitted**.
22. The folder scope's totals say "in the N shown" when its list is cut.

## Tests

- `bridge-core/src/chat_changes/tests.rs`: rules 1, 2, 4, 5, 6, 7, 8, 9, 10,
  and an end with no recorded start.
- `live_turn` `turn_boundaries_snapshot_the_chats_folder_so_its_changes_can_be_scoped`:
  rules 1 and 3, driven by real `turn/started` and `turn/completed` frames.
- `bridged/tests/chat_changes.rs`: rules 11, 12 and 13 over the socket.
- `protocol_mirror`: the core result matches its wire mirror.
- `src/chatChanges.test.ts`: rules 14 to 17.
- `src/components/ChangesPanel.test.tsx` "scoped to the chat": rules 19 to 22.
