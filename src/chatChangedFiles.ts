import { useMemo } from "react";
import { mergeConversationProjections, reduceConversation, toolCallDisplay, type ConversationItem, type ToolCallDisplay } from "./conversation";
import { projectSessionConversationCached } from "./transcript/projectionCache";
import type { AgentEvent, SessionEntry } from "./types";

/**
 * The files a chat has written to, read off its own transcript.
 *
 * The `@` menu marks a file this chat already changed, so a follow-up about
 * "the file you just edited" finds it at a glance. The workspace's git status
 * cannot answer that: it counts every dirty file in the checkout, including
 * the person's own edits and another chat's. The transcript's edit calls are
 * exactly what this chat did.
 *
 * `A` is a file the chat created, `M` one it changed that was already there.
 * The first write decides: a file created and then edited again is still new
 * to this chat. A delete forgets the file, so a later write starts over.
 */
export type ChatFileChange = "A" | "M";

type Op = "add" | "update" | "delete";

/**
 * Each changed file, keyed by its path relative to the chat's folder, which is
 * how the workspace file list names it. Edits that failed, or whose arguments
 * are still streaming, changed nothing yet and are skipped. A path outside
 * every root cannot be one of the listed files and is dropped.
 */
export function chatChangedFiles(
  items: Iterable<ConversationItem>,
  roots: readonly (string | null | undefined)[],
): Map<string, ChatFileChange> {
  const bases = normalizedRoots(roots);
  const changes = new Map<string, ChatFileChange>();
  for (const item of items) {
    if (item.type !== "activity" && item.type !== "diff") continue;
    const tool = toolCallDisplay(item);
    if (tool.verb !== "edit" || tool.status === "failed" || tool.preparing) continue;
    for (const touch of touchedFiles(item, tool)) {
      const path = workspaceRelative(touch.path, bases);
      if (!path) continue;
      if (touch.op === "delete") { changes.delete(path); continue; }
      if (!changes.has(path)) changes.set(path, touch.op === "add" ? "A" : "M");
    }
  }
  return changes;
}

const NO_CHANGES: ReadonlyMap<string, ChatFileChange> = new Map();

/**
 * `chatChangedFiles` for a chat on screen, over its stored history and the live
 * frames that have not been stored yet. Projects nothing while `enabled` is
 * false, so a closed menu costs nothing during streaming.
 */
export function useChatChangedFiles({ enabled, sessionId, entries, activeLeafId, events, roots }: {
  enabled: boolean;
  sessionId: string | undefined;
  entries: SessionEntry[] | undefined;
  activeLeafId: string | null | undefined;
  /** Live frames; filtered to `sessionId` here. */
  events: readonly AgentEvent[];
  roots: readonly (string | null | undefined)[];
}): ReadonlyMap<string, ChatFileChange> {
  const rootKey = roots.filter(Boolean).join("\0");
  return useMemo(() => {
    if (!enabled || !sessionId) return NO_CHANGES;
    const durable = entries?.length ? projectSessionConversationCached(entries, activeLeafId ?? null) : [];
    const live = reduceConversation(events.filter(event => event.sessionId === sessionId));
    return chatChangedFiles(mergeConversationProjections(durable, live), rootKey ? rootKey.split("\0") : []);
  }, [enabled, sessionId, entries, activeLeafId, events, rootKey]);
}

interface Touch {
  path: string;
  op: Op;
}

function touchedFiles(item: ConversationItem, tool: ToolCallDisplay): Touch[] {
  const data = item.data;
  // Codex names every file of one change, each with its own kind.
  if (Array.isArray(data.changes) && data.changes.length) {
    return data.changes.flatMap(change => {
      const record = change && typeof change === "object" ? change as Record<string, unknown> : {};
      const path = typeof record.path === "string" ? record.path : undefined;
      return path ? [{ path, op: changeOp(record.kind) ?? "update" }] : [];
    });
  }
  const op = toolOp(item, tool);
  // A normalized file-change event can name several paths without kinds.
  if (Array.isArray(data.paths) && data.paths.length) {
    return data.paths.filter((path): path is string => typeof path === "string" && !!path.trim()).map(path => ({ path, op }));
  }
  return tool.path ? [{ path: tool.path, op }] : [];
}

/** Codex writes the kind as a word or as `{ type }`. */
function changeOp(kind: unknown): Op | undefined {
  const value = typeof kind === "string" ? kind : kind && typeof kind === "object" ? (kind as Record<string, unknown>).type : undefined;
  if (value === "add" || value === "create") return "add";
  if (value === "delete" || value === "remove") return "delete";
  if (value === "update" || value === "modify") return "update";
  return undefined;
}

function toolOp(item: ConversationItem, tool: ToolCallDisplay): Op {
  // ACP names a delete as its own tool kind.
  if (item.data.kind === "delete") return "delete";
  // A whole-file write creates the file unless the harness says it replaced one.
  if (tool.glyph === "file-plus") return /\bhas been (?:updated|overwritten)\b/i.test(tool.output ?? "") ? "update" : "add";
  if (tool.patch && /^(?:--- \/dev\/null|new file mode)/m.test(tool.patch)) return "add";
  return "update";
}

function normalizedRoots(roots: readonly (string | null | undefined)[]): string[] {
  return roots
    .filter((root): root is string => !!root && root.startsWith("/"))
    .map(root => root.replace(/\/+$/, ""))
    .filter(Boolean)
    // Longest first, so a worktree nested inside its repository wins.
    .sort((a, b) => b.length - a.length);
}

function workspaceRelative(path: string, roots: readonly string[]): string | undefined {
  const trimmed = path.trim();
  if (!trimmed) return undefined;
  if (!trimmed.startsWith("/")) return trimmed.replace(/^(?:\.\/)+/, "") || undefined;
  for (const root of roots) {
    if (trimmed.startsWith(`${root}/`)) return trimmed.slice(root.length + 1) || undefined;
  }
  return undefined;
}
