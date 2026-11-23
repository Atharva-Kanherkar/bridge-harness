# feat/import-conductor: Test Contract

Issues #1056 (adopt other tools' worktrees in place) and #1057 (Conductor), for Conductor first. Conductor's workspaces come over as Bridge workspaces, left where they are on disk. Their chats come over with the Claude Code and Codex imports and land under them.

Facts this is built on, researched without running Conductor's UI: its installed bundle (0.97.0), its database structure, the rollback SQL it keeps for 98 of its 178 migrations, its bundled agent guide, and its public docs and changelog. Structure and counts only.

- A workspace is a git worktree, under `~/conductor/workspaces/<repo>/<directory>` (since 0.25.0), else `<repo root>/.conductor/<directory>`. A repo's Workspaces path can be changed in its settings. `~/conductor/repos` holds clones (main checkouts, not workspaces). `~/conductor/cloud-workspaces` and `~/conductor/remote-workspace-sync` hold cloud sync, not local workspaces.
- `~/Library/Application Support/com.conductor.app/conductor.db` (SQLite, WAL, sqlx migrations) records:
  - `workspaces`: `directory_name`, `workspace_name`, `branch`, `state`, `workspace_path`, `derived_status`, `manual_status`, `pr_title`, `pinned_at`.
    - Archiving sets `state='archived'` and records `archive_commit`.
    - Values seen in the app: `active`, `archived`, `initializing`, `setting_up`.
  - `repos`: `name`, `root_path`, `storage_version`, `setup_script`, `run_script`, `run_script_mode`, `archive_script`.
  - `sessions`: `workspace_id`, `agent_type` (`claude`, `codex`, `cursor`, `muse`, `acp`), and `claude_session_id`, the agent's own session id.
- Migration 179 added `projects`, `project_repositories` and `workspace_checkouts`, so one workspace can hold several repos. Migration 180 renamed `workspaces.repository_id` to `project_id`. Earlier versions link a workspace straight to one repo.
- Since 0.95.0 settings live in the app. `conductor.json` is legacy: it is imported once and not read after.
- The same database holds `env_vars` (secrets), `settings`, `identity_associations`, and message copies.

## Functional Behavior

### Adopting worktrees (#1056, any tool)

- A candidate is adopted only when the folder exists and `git worktree list --porcelain`, run from its main checkout, lists it on a branch.
  - A worktree on a detached HEAD is reported as needing a branch, not adopted.
  - A missing folder is reported, not adopted.
  - A folder that is not a worktree is reported, not adopted.
  - A main checkout is not a workspace.
- The project is the main checkout. It is created when Bridge does not have it, matched by real path.
- The adopted workspace keeps its folder, branch and uncommitted changes. Its name is the tool's name for it, else the folder name.
- Adopted workspaces are marked as the other tool's. Archiving one in Bridge forgets it and never deletes or changes its folder, even with uncommitted changes. Worker worktrees Bridge itself made for it are still cleaned up.
- Chats already imported whose folder is the adopted worktree move under the workspace. Chats imported later land there by the existing folder match.
- A worktree Bridge already has is "already in Bridge". Adopting again changes nothing.
- No git command runs while Bridge's database is held. No file on disk is created, changed or deleted by an import.

### Conductor (#1057)

- **Path A, always:**
  - workspaces are the worktrees under `~/conductor/workspaces/<repo>/` and under `<repo>/.conductor/` for repos Conductor knows;
  - named by folder;
  - no Conductor database read needed.
- **Path B, when the database is a version Bridge knows:**
  - names, archived state, PR titles and scripts come from `conductor.db`, opened read-only;
  - a workspace archived in Conductor is left out and counted;
  - the setup, run and archive scripts are kept with the workspace and reported, never run.
- Both versions of the database are read:
  - from migration 179 on: projects, with workspace checkouts;
  - before it: one repo per workspace.
- **The version gate.** It fails closed, falling back to Path A with a note, when the database:
  - is newer than any version Bridge knows;
  - is older than the oldest version Bridge knows;
  - is missing a column Bridge reads;
  - is not a SQLite database;
  - is locked or damaged.
- Only named columns of `repos`, `projects`, `project_repositories`, `workspaces`, `workspace_checkouts`, `sessions` and `_sqlx_migrations` are queried. Never `env_vars`, `settings`, `identity_associations` or any message table.
- **The Import page:**
  - Conductor's section has an Import button. It runs in the background like the other tools and reports workspaces brought over, already in Bridge, archived and left out, and those that could not be adopted, with why.
  - It says Conductor's chats come over with the Claude Code and Codex imports.
- Detection counts Conductor's workspaces and repos from folders, and from the database when the gate passes.

## Unit Tests

- Worktree probe, against real git repos built in the test:
  - a worktree on a branch;
  - one with uncommitted changes;
  - detached HEAD;
  - a missing folder;
  - a plain folder;
  - a main checkout;
  - a nested `.conductor/` layout;
  - paths with spaces and non-ASCII characters.
- Adoption:
  - the project is created once;
  - the workspace keeps its name, branch and path;
  - imported chats move under it;
  - a second run adopts nothing;
  - Bridge's own workspace at the same path is "already in Bridge".
- Archive guard: archiving an adopted workspace forgets it and leaves its folder and uncommitted changes as they were. A Bridge-made worktree is still removed as before.
- Conductor reader, on fixtures built from the real schemas:
  - the current version (181, Conductor 0.97.0);
  - the version before projects (178);
  - the oldest Bridge supports;
  - for each: names, archived, scripts, multi-repo checkouts, legacy `.conductor` paths, custom workspace paths.
- Gate: newer, older, missing a column, not SQLite, locked, empty.
- Deny-list:
  - every SQL statement the reader runs is in one list, and none names a denied table;
  - canaries in `env_vars` and `settings` never appear in any output.

## Fuzz Tests

- **Plan fuzzing (property-based, thousands of cases).** Random Conductor databases are generated, with random sets of:
  - workspace rows: states, statuses, names (empty, long, non-ASCII, path-like, SQL-like), duplicate and missing paths;
  - repos and checkouts;
  - folder facts: exists, a worktree on a branch, detached, dirty, a main checkout.

  Each case must hold these properties:
  - no panic;
  - adopted ⊆ valid worktrees on a branch;
  - no path adopted twice;
  - archived never adopted;
  - every candidate accounted for exactly once (adopted, already, or left out with a reason);
  - a second run adopts nothing;
  - no canary in the output.
- **Schema mutation fuzzing.** The real schemas are mutated at random: a column dropped or renamed, a table dropped, a type changed, garbage rows, migration versions shifted. The reader must fail closed to Path A with a note, or read correctly. It must never panic and never return a workspace with the wrong path.
- **Filesystem fuzzing (fewer cases, real git).** Random small layouts of real repos and worktrees under the Conductor roots, adopted end to end. No file on disk changes: a hash of the tree is taken before and after.

## Mutation Testing

- `cargo mutants` over `import_worktrees.rs` and `conductor_import.rs`. Every surviving mutant is either killed by a new test or listed in the PR with why it is equivalent.

## Integration / Functional Tests

- Through the import job, on a fake home with real repos, worktrees and a Conductor database built from the real schema:
  - adopted;
  - archived left out;
  - detached and missing reported;
  - progress and the summary;
  - a second run brings nothing.
- With chats already imported from Claude Code, adoption moves the matching ones under the workspace.
- `import_status` returns Conductor's last import beside the other tools.
- `MoveToBridge.test.tsx`: Conductor's section has its own Import button and its summary counts workspaces.

## Smoke Tests

- `cargo test -p bridge-core --lib -- import_worktrees conductor_import import_job import_sources workspaces`
- `npx vitest run src/components/MoveToBridge.test.tsx src/moveToBridge.test.ts`
- `bun run build`

## E2E Tests

- Scripted fake user, against a real `bridged` on a fake home holding real git worktrees and a Conductor database built from the real 0.97.0 schema:
  - imports Conductor from its own section;
  - the workspaces appear with their names;
  - a chat imported from Claude Code shows under its workspace;
  - archiving one in Bridge leaves its folder on disk.
- Scale: the job over 2,000 generated workspaces in 50 repos, with time and peak memory measured on the test binary.

## Manual Tests

- On a Mac with Conductor workspaces: Import, then Conductor. The workspaces appear under their repos, Conductor keeps working with them, and archiving one in Bridge leaves Conductor's folder alone.
