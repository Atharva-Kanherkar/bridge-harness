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

/** What stays behind whichever way an imported chat continues. */
export const CONTINUATION_LEAVES_BEHIND = "Unfinished tool calls, background processes and hidden reasoning stay behind.";

/**
 * What the composer on an imported chat says before the first send: whether
 * the next message resumes the tool's own session or starts a new chat with
 * the history, and why not natively.
 */
export function continuationNotice(continuation: { mode: string; reason?: string | null }, tool?: string | null): { headline: string; detail: string } {
  const label = importSourceLabel(tool);
  if (continuation.mode === "native") {
    return { headline: `Continues the ${label} session`, detail: `Your message opens a new chat that resumes it. ${CONTINUATION_LEAVES_BEHIND}` };
  }
  const reason = continuation.reason?.trim() || `${label} cannot resume it.`;
  return { headline: "Starts a new chat with this history", detail: `${reason} ${CONTINUATION_LEAVES_BEHIND}` };
}
