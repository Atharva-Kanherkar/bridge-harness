// What Notes says to agents and about them, kept pure for tests.
//
// `taskBrief` is the first message of a chat started from a task or page.
// `notesContext` is the open page as the docked Notes chat reads it, folded
// onto a message as a fence only when it changed, the way the Storage page
// sends its snapshot. `actorLabel` turns a page's last actor into a byline.

import { harnessLabel } from "./utils";
import type { Note } from "./types";

export const NOTES_CONTEXT_FENCE = "notes-context";
const BRIEF_BODY_CHARS = 6000;
const CONTEXT_BODY_CHARS = 4000;

const clip = (text: string, limit: number) => text.length > limit ? `${text.slice(0, limit)}\n[clipped: ${text.length - limit} more characters]` : text;
const quote = (text: string) => text.trimEnd().split("\n").map(line => `> ${line}`.trimEnd()).join("\n");

export interface TaskBriefInput {
  title: string;
  /** The task page's body: its detail and "Done when". */
  body: string;
  /** The verbatim span of the parent page the task came from. */
  excerpt?: string | null;
  /** The page the task came from, by title. */
  parentTitle?: string | null;
}

/** The first message of a chat started from a note. */
export function taskBrief({ title, body, excerpt, parentTitle }: TaskBriefInput): string {
  const parts = [title.trim()];
  const detail = body.trim();
  if (detail && detail !== title.trim()) parts.push(clip(detail, BRIEF_BODY_CHARS));
  if (excerpt?.trim()) parts.push(`${parentTitle?.trim() ? `From my note "${parentTitle.trim()}"` : "From my notes"}:\n${quote(excerpt)}`);
  else if (parentTitle?.trim()) parts.push(`From my note "${parentTitle.trim()}".`);
  parts.push("Before you start, say in one line what done means here. Ask me only about ambiguity that would change the outcome; otherwise make a sensible call and go.");
  return parts.join("\n\n");
}

/** A byline for a page's latest actor, or null when the person wrote it. */
export function actorLabel(actor: string, chatTitle: (sessionId: string) => string | undefined): string | null {
  if (actor === "file-import") return "Edited outside Bridge";
  const agent = /^agent:([^:]+):(.+)$/.exec(actor);
  if (!agent) return null;
  const [, harness, sessionId] = agent;
  return `Written by ${harnessLabel(harness)} in ${chatTitle(sessionId) ?? "a chat"}`;
}

/** The open page as the Notes chat reads it. */
export function notesContext({ note, markdown, projectName }: { note: Note; markdown: string; projectName?: string | null }): string {
  const lines = [
    `Open page: ${note.title?.trim() || "Untitled"} (id ${note.id}, seq ${note.seq})`,
    `Kind: ${note.kind} · State: ${note.state} · Project: ${projectName ?? "none"}${note.tags.length ? ` · Tags: ${note.tags.join(", ")}` : ""}`,
    "---",
    clip(markdown, CONTEXT_BODY_CHARS),
  ];
  return lines.join("\n");
}

/** A message with the open page folded onto its end. The fence outgrows any
 *  backtick run in the page, so a code block in a note cannot close it. */
export function withNotesContext(question: string, context: string): string {
  const text = question.trim();
  if (!context) return text;
  const longest = Math.max(0, ...[...context.matchAll(/`+/g)].map(run => run[0].length));
  const fence = "`".repeat(Math.max(3, longest + 1));
  return `${text}\n\n${fence}${NOTES_CONTEXT_FENCE}\n${context}\n${fence}`;
}

/** Undo `withNotesContext` for display: what was typed, and the page apart. */
export function splitNotesContext(text: string): { text: string; context: string | null } {
  const opening = [...text.matchAll(new RegExp(`(\`{3,})${NOTES_CONTEXT_FENCE}\n`, "g"))].pop();
  if (!opening || opening.index === undefined) return { text, context: null };
  const fence = opening[1];
  const rest = text.slice(opening.index + opening[0].length);
  const body = rest.endsWith(`\n${fence}`) ? rest.slice(0, -fence.length - 1) : rest.replace(/\n?`{3,}\s*$/, "");
  return { text: text.slice(0, opening.index).trimEnd(), context: body };
}
