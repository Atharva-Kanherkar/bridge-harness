import type { ResolveReferenceResult } from "./protocol/generated/protocol";

/**
 * Session references: the `brio_` public alias (first 8 hex chars of the
 * session uuid) and its `@session:` mention spelling. These are the tokens
 * the composer recognizes and the sidebar copies.
 */
export const REFERENCE_TOKEN = /(?:@session:[a-zA-Z0-9_-]+|\bbrio_[0-9a-f]{8})\b/g;

/** The public alias a session's raw id is copied as and pasted back in. */
export function toPublicAlias(sessionId: string): string {
  return `brio_${sessionId.replace(/-/g, "").slice(0, 8)}`;
}

/** Every reference-shaped token in a draft, in order, deduplicated. */
export function findReferences(text: string): string[] {
  const unique = new Set<string>();
  for (const match of text.matchAll(REFERENCE_TOKEN)) unique.add(match[0]);
  return [...unique];
}

/** The alias a token resolves to (mention spellings include the alias). */
export function referenceAlias(token: string): string {
  const bare = token.replace(/^@session:/, "");
  return bare.startsWith("brio_") ? bare : toPublicAlias(bare);
}

/** The leading fragment of an id, for a label that has nothing better. */
function shortId(id: string | undefined): string {
  return (id ?? "").slice(0, 8) || "unknown";
}

export type ReferenceChipModel = {
  token: string;
  alias: string;
  resolved: ResolveReferenceResult;
};

/** Chip label: a short title for the resolved reference. */
export function chipSummary(reference: ResolveReferenceResult): string {
  switch (reference.kind) {
    case "session":
      return reference.label;
    case "entry":
      return reference.summary || `entry ${shortId(reference.entryId)}`;
    case "unknown":
      return "Unknown reference";
  }
}

/** The activity line shown under a chip's title. */
export function chipDetail(reference: ResolveReferenceResult): string {
  switch (reference.kind) {
    case "session":
      return `${reference.harness} · ${reference.restorationMode}${reference.workspaceId ? " · workspace" : ""}${reference.parentSessionId ? " · fork" : ""}`;
    case "entry":
      return `${reference.entryKind} · #${reference.sequence}`;
    case "unknown":
      return "no such session or entry";
  }
}

/** The text appended to the draft when a chip is pulled into the chat. */
export function referencePullText(reference: ResolveReferenceResult): string {
  switch (reference.kind) {
    case "session":
      return `[session ${reference.label} — checkpoint ${reference.latestCheckpointEntryId ? "present" : "none"}]`;
    case "entry":
      return reference.summary ? `> ${reference.summary}` : `[entry ${shortId(reference.entryId)}]`;
    case "unknown":
      return "";
  }
}