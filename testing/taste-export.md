# taste-export — Test Contract

Sub-issue 10 of the taste layer epic (#1196): #1206. A repository's taste
rules go out to its plain instruction files, inside a section Bridge manages,
and come back from them. Taste is never locked into Bridge, and tools that
only read instruction files benefit too.

## Functional Behavior

1. **Files and kinds.** `AGENTS.md` and `CLAUDE.md` carry `code_prefer` rules;
   `.cursor/BUGBOT.md` carries `review_avoid` and `review_prefer`. Each gets a
   section between `<!-- bridge:taste:start v1 -->` and `<!--
   bridge:taste:end -->`:
   - a heading;
   - one bullet per rule: `- <rule> (paths: <globs>) <!-- id:<id> kind:<kind> -->`;
   - `_No rules yet._` when the file carries none.
2. **Merge.** An existing section is replaced. Text outside the markers is
   kept byte for byte. A file without the markers gets the section at its end
   after a blank line, and an empty or missing file becomes the section.
   Merging the same section again changes nothing.
3. **Parse.** Bullets read back with their id, kind, text and paths. A section
   with a start and no end is refused, naming the line it starts on, and
   nothing is written.
4. **Export** (`taste/export_taste_rules {scopeKey, workspaceId?, targets?,
   write}`):
   - With `write: false`, it previews each file (before, after, changed).
   - With `write: true`, it writes the changed files, creating `.cursor/`
     when needed, under the workspace lock.
   - It never commits.
   - The checkout is a Bridge workspace whose repository (from its reviews or
     its `origin` remote) is the scope. If there is none, it is refused.
5. **Import** (`taste/import_taste_rules {scopeKey, workspaceId?, apply}`):
   - A bullet whose id names one of the repository's active rules with
     different text edits it (supersede).
   - A new bullet, not repeating a rule, becomes a rule the person wrote, of
     its marked kind or the file's (code for AGENTS/CLAUDE, avoid for
     BUGBOT).
   - Rules a read file would carry but does not list are reported as
     missing, never deleted.
   - Without `apply` nothing changes. Importing what export wrote changes
     nothing.
6. **Taste tab.** Each repository's rules have a **Files** panel:
   - the checkout's path;
   - each file "up to date", "will be updated" or "will be created", with the
     section to be written;
   - **Write files**;
   - **Read back from files**, listing new, changed and missing rules, with
     **Apply**.

## Unit Tests

**Rust (`bridge-core/src/taste/export_tests.rs`, new)**
- `each_file_gets_the_rules_it_carries_between_the_markers`
- `text_outside_the_markers_is_kept_byte_for_byte`
- `bullets_read_back_with_their_ids_kinds_and_paths`
- `export_previews_then_writes_and_import_of_its_own_output_changes_nothing`
- `import_edits_named_rules_adds_new_ones_and_reports_the_missing`
- `a_malformed_section_is_refused_and_nothing_is_written`
- `the_api_finds_the_repositorys_checkout_and_says_when_there_is_none`

**Vitest (`TasteRules.test.tsx`)**
- previews a repository's rule files, writes them, and reads them back

## Verification
`cargo test -p bridge-core --lib taste::`; `cargo test -p bridge-protocol`;
the shell's contract test; Vitest for `TasteRules`; lint and clippy ratchets.
