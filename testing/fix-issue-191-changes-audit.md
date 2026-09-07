# fix/issue-191-changes-audit — Test Contract

Locked before implementation. The authoritative baseline is the freshly fetched
`origin/main` commit `8f21001ef5913022687104b282f43f21dec0a20e` (merge-base
is the same commit). This contract hardens the Changes-tab MVP associated with
issue #191 and evidence `2e4ef256-6e8e-4098-bc8a-07db55b7165a`.

The issue's deferred work remains out of scope: AST/symbol classification, LLM
summaries or hunk scoring, dependency-centrality ranking, worker-attribution
grouping, and skim/deep modes.

## Functional Behavior

The audit found eight durable defects. Each is source-proven on the baseline and
must be fixed with the mapped regression coverage below.

1. **Bounded extraction and transport.** Changeset enumeration, per-file patch
   extraction, and the aggregate patch payload have explicit limits. When a
   file patch or the file list is incomplete, the wire result says so and the
   UI visibly explains that more data exists; truncation is never represented
   as a complete diff. Large untracked files are not read wholly into memory.
2. **Latest refresh wins.** Initial loads, stats-driven reloads, and save-driven
   reloads may overlap, but an older success or failure can never overwrite a
   newer Changes-tab result. Switching workspaces invalidates every request for
   the previous workspace.
3. **Repository state is representable.** A connected plain directory returns
   a non-Git state rather than a raw Git failure. A newly initialized repository
   without `HEAD` returns an unborn state and still reports staged and untracked
   changes against the empty tree.
4. **Mode-only changes remain reviewable.** Executable-bit or other mode-only
   changes preserve their `old mode`/`new mode` metadata and are identified as
   mode-only rather than rendering an empty-patch message.
5. **Git filenames are parsed losslessly on the wire-supported path domain.**
   NUL-delimited Git output is used so tabs, newlines, quotes, backslashes, and
   Unicode in valid UTF-8 filenames remain a single exact path and subsequent
   patch lookup targets that path.
6. **Path labels are visible.** Every backend `labels` value is rendered on its
   file row alongside the importance badge, using existing Tailwind v4
   utilities/components only.
7. **Renames retain one-file identity.** A detected rename is one file change,
   carries its previous path, and does not contradict the one-entry dirty count
   with delete-plus-add rows.
8. **Deleted files cannot enter Edit mode.** A deleted file remains reviewable
   as a diff but exposes no editor control that can only fail to open.

Existing MVP behavior remains intact: files sort High, Medium, Low and then by
path; low-signal files are reversibly collapsed with a visible count; viewed
tracking remains available; binary files remain explicit; quotes and Code-pane
handoffs keep using the displayed current path.

## Unit Tests

### Rust — `src-tauri/bridge-core/src/git.rs`

- `workspace_changeset_bounds_file_and_patch_payloads` — file enumeration and
  patch bytes stop at the documented limits and set visible truncation flags.
- `workspace_changeset_handles_non_repository` — plain folders return
  `notGit`, no files, and no raw Git error.
- `workspace_changeset_handles_unborn_repository` — an unborn repository
  returns `unborn` and includes staged and untracked additions.
- `workspace_changeset_preserves_mode_only_changes` — a chmod-only change has
  kind `modeOnly`, zero line counts, and readable mode metadata.
- `workspace_changeset_parses_unusual_filenames` — tabs, newlines, quotes,
  backslashes, and Unicode survive NUL-delimited parsing and have their patch.
- `workspace_changeset_reports_rename_once` — a rename has kind `renamed`, the
  new path, the old path, and exactly one result row.
- Existing tracked/untracked, deletion, binary, risk-tier, low-signal, and
  header-stripping coverage remains green.

### React — `src/components/ChangesPanel.test.tsx`

- `keeps the newest overlapping refresh result` — resolve request B before
  request A and keep B rendered; a late A error is also ignored.
- `invalidates an old request when the workspace changes` — the previous
  workspace cannot overwrite or error the new workspace.
- `renders repository-state guidance` — non-Git and unborn repositories get
  accurate, non-fatal copy.
- `renders patch and file-list truncation disclosures` — incomplete data is
  conspicuous at both row and panel level.
- `renders every path label` — backend labels appear on the matching row.
- `does not offer Edit for a deleted file` — deletion diff is expandable but
  only the Diff control is present.
- `renders a rename as one old-to-new row` — one row communicates both paths.
- Existing importance ordering, reversible low-signal reveal, viewed tracking,
  clean/loading/error states, quoting, and Code-pane handoff tests remain green.

### Protocol serialization

- Workspace change kind, repository state, rename origin, and truncation fields
  round-trip with stable camelCase/snake_case wire names.

## Integration / Functional Tests

- `workspaces/workspace_changes` returns the same enriched result shape through
  bridge-core, bridge-protocol, daemon dispatch, embedded Tauri, generated
  TypeScript types, and `bridgeApi.workspaceChanges`.
- Focused Rust changeset tests and focused ChangesPanel Vitest tests pass
  together after every relevant implementation checkpoint.
- `bun run build` passes, including strict TypeScript and the Vite production
  bundle.
- `bun run test` passes, including the complete Vitest and Cargo suites.

## Smoke Tests

- A normal dirty repository shows importance-sorted files, additions/deletions,
  labels, patches, viewed controls, and reversible low-signal disclosure.
- A clean repository says the workspace is clean.
- A plain folder and an unborn repository open the Changes tab without a raw
  Git failure.
- A truncated large changeset visibly states that the review data is partial.

## E2E Tests

N/A — the repository has no automated desktop-webview E2E harness for this
surface. Embedded/daemon contract coverage, focused DOM tests, the production
build, and the full repository suite are the automated boundary for this fix.

## Manual / cURL Tests

No cURL endpoint exists; the command is a local Tauri/daemon RPC. Manual review:

1. In a repository, modify a source file, rename another file, chmod a tracked
   script, delete a text file, and create a low-signal lockfile.
2. Open Changes and verify importance order, labels, rename and mode metadata,
   no Edit control for the deletion, viewed tracking, and low-signal reveal.
3. Open a connected non-Git folder and an unborn `git init` folder; verify the
   state copy and absence of raw `fatal:` stderr.
4. Generate a file or changeset beyond the documented caps; verify partial-data
   disclosure appears and the app remains responsive.
5. Inspect `git diff origin/main...HEAD` for MVP-only scope, Tailwind v4-only
   styling, no generated artifacts, and a contract-to-test mapping for all
   eight findings.
