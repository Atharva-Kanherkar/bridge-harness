# fix/import-projects: Test Contract

Imported chats landed in "No project", and most of them were not chats. On this Mac, 29,521 of the 29,763 chats imported from Claude Code were runs, not conversations. 24,000+ of them were Bridge's own Slack inbox, title and notes runs, which Claude Code saves like any session. 5,348 were `/usage` probes. The real chats got a project only when Bridge already had one at exactly their folder.

## Functional Behavior

- **Runs are not chats.** An import leaves out, and counts as background runs:
  - **Bridge's own runs:** a chat whose folder is inside Bridge's data folder. That covers this Bridge's, or another Bridge's (a development build or a test `--data-dir`), recognised by `chats/` or `worktrees/` beside its `bridge.db`. When that folder is gone, its exact layout is enough: `chats/<chat id>`, `chats/title-runs`, `chats/note-runs`, `worktrees/orchestrators` or `worktrees/workers`.
  - **Command runs:** chats made only of slash commands the person typed (`/usage`, `/login`, `/mcp`) and Claude Code's local output for them. No reply, tool call or thinking. At least one command.
  - Claude Code's project folders for this Bridge's data folder are not even read. The Import page counts them as background runs too.
- **Earlier imports are tidied.** On startup, and after every import, imported copies of runs are removed with their subagents and history. Imported history is otherwise immutable:
  - only these are ever deleted;
  - the guard on their entries is lifted inside each batch's transaction and restored before it commits;
  - provenance stays as the audit;
  - removal goes in batches of 500, so Bridge's database is never held for long;
  - foreign keys are checked when each batch commits.
- **Every imported chat lands in its project.**
  - The folder resolves to its repository's main checkout. Worktrees belong to their repository, including Claude Code's `<repo>/.claude/worktrees/<name>` after it is deleted.
  - A folder that is not a repository is its own project.
  - Bridge makes the workspace (and the project, for a repository) when it has none, named after the folder, on its current branch.
  - A workspace at the chat's exact folder wins, such as an adopted worktree.
  - Adopting a worktree later moves its chats from the repository's project to it.
- **Left without a project:** folders that are gone, temporary, Bridge's own, the home folder or above it, and hidden folders in it (`~/.claude`, `~/.cache`).
- No git command runs while Bridge's database is held.

## Unit Tests

- Slash commands vs things said; command runs, with and without replies, tool calls or plain text.
- Bridge folders by path, by Claude Code project name, by another Bridge's layout, and by layout once gone. A project holding a `bridge.db` is still a project.
- Project folders: a subfolder, the repository root, a worktree, a deleted Claude Code worktree, a plain folder. Gone, temporary, Bridge's, home, above home and hidden folders get none.
- Placement:
  - one project made per repository and reused;
  - an existing project reused;
  - subagents follow;
  - chats already filed are left alone;
  - an adopted worktree keeps its chats, and adopting later moves them;
  - a plain folder gets a workspace without a repository project;
  - a second run changes nothing.
- Purge:
  - Bridge's runs and probes go, with subagents, entries and native ids;
  - real chats and Bridge-run chats Bridge itself owns stay;
  - provenance stays;
  - the immutability guard is back afterwards;
  - batches past 500.
- Job: Bridge's runs and a probe are left out and counted, a real chat lands in the project made for it, a chat whose folder is gone stays loose. Startup tidying removes runs an earlier import brought and files the rest.
- Detection counts Bridge's runs as background runs.

## E2E

- The fake user imports Claude Code from a home that also holds a chat in a repository subfolder, a `/usage` probe, and a Slack inbox run in the Bridge's own data folder. The real chat is under its repository's project; the probe and the run are not chats. Conductor and T3 Code chats still end under their adopted worktrees.

## Real data (read-only measurement)

- On this Mac's Bridge database (29,763 imported): 29,521 removed; of 242 kept, 103 already in a project, 125 filed into 7 projects, 14 left loose (`/private/tmp`, `~/.cache`).
