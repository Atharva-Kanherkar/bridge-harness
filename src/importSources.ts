// The tools Bridge can import from, by the id the transfer bundle uses
// (`bridge-core/src/import_bundle.rs` `tool_label` is the Rust twin).

export const IMPORT_SOURCE_LABELS: Record<string, string> = {
  claude_code: "Claude Code",
  codex: "Codex",
  opencode: "OpenCode",
  conductor: "Conductor",
  t3code: "T3 Code",
  orca: "Orca",
  superset: "Superset",
  cursor: "Cursor",
};

/** "Claude Code" for `claude_code`; an unknown id is shown as itself. */
export function importSourceLabel(tool: string | undefined | null): string {
  if (!tool) return "Claude Code";
  return IMPORT_SOURCE_LABELS[tool] ?? tool;
}

/** The badge on an imported chat's toolbar: where it came from and which file. */
export function importedChatBadge(tool: string | undefined, fingerprint: string | undefined): string {
  return `Imported · ${importSourceLabel(tool)}${fingerprint ? ` · ${fingerprint.slice(0, 12)}…` : ""}`;
}
