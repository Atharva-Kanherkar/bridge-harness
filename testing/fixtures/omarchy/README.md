# Omarchy theme fixtures

`themes/<name>.toml` is each shipped theme's `colors.toml`, copied verbatim from
[basecamp/omarchy](https://github.com/basecamp/omarchy) at
`7dd477b35bbf9737dd45c41bff3e7a0e96787e09` (branch `quattro`, 2026-10-13).
Omarchy is MIT licensed, copyright David Heinemeier Hansson.

They back three tests: the shell's `colors.toml` parser
(`src-tauri/src/omarchy.rs`), the palette-to-token table
(`src/lib/omarchyTheme.test.ts`), and the landing page's theme list
(`landing/test/omarchy.test.ts`). To refresh, copy
`themes/*/colors.toml` from a newer Omarchy checkout and update the commit above.

Three themes (`last-horizon`, `solitude`, `white`) ship without `orange`. That
is real, not a copy error, and the token table falls back for it.
