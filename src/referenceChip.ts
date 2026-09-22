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

/** The mention spelling the sidebar drops into a draft. */
export function mentionToken(sessionId: string): string {
  return `@session:${toPublicAlias(sessionId)}`;
}

/** Insert a mention into a draft, keeping one space on each side. */
export function insertMention(draft: string, sessionId: string): string {
  const token = mentionToken(sessionId);
  if (draft.includes(token)) return draft;
  const lead = draft.length === 0 || /\s$/.test(draft) ? "" : " ";
  return `${draft}${lead}${token} `;
}

/**
 * Remove every occurrence of `token` from a draft along with one adjacent
 * space, leaving everything else byte-for-byte as typed. Collapsing runs of
 * whitespace across the whole draft would flatten indented code pasted
 * beside the reference.
 */
export function removeReferenceToken(draft: string, token: string): string {
  let out = draft;
  let at = out.indexOf(token);
  while (at >= 0) {
    let start = at;
    let end = at + token.length;
    if (out[end] === " ") end += 1;
    else if (start > 0 && out[start - 1] === " ") start -= 1;
    out = out.slice(0, start) + out.slice(end);
    at = out.indexOf(token);
  }
  return out;
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

/**
 * The line under a chip's title. It says what will actually happen on send:
 * Bridge attaches the referenced chat's stored history to the turn, so the
 * agent can continue it. The old copy named a restoration mode and a
 * "checkpoint present" flag — true, and useless to the person reading it.
 */
export function chipDetail(reference: ResolveReferenceResult): string {
  switch (reference.kind) {
    case "session":
      return `${reference.harness}${reference.parentSessionId ? " · fork" : ""} · history attaches on send`;
    case "entry":
      return `${reference.entryKind} #${reference.sequence} · attaches on send`;
    case "unknown":
      return "no such chat — sent as plain text";
  }
}