import { reduceConversation } from "../../conversation";
import type { AgentEvent, Project, Session, SessionEntry, Workspace } from "../../types";

// What a tile is called. The backend names a pasted link by what it points at
// ("kairo PR #43"); inside kairo's own tile the repository is noise.
const LINK_HEADING = /^(\S+) (PR|issue) #(\d+)$/i;

/** A title for the tile. Only a derived link heading changes; every other title passes through. */
export function displayTitle(raw: string, project?: string | null): string {
  const title = raw.trim();
  const heading = LINK_HEADING.exec(title);
  if (!heading) return title;
  const [, repo, kind, number] = heading;
  const name = `${kind.toUpperCase() === "PR" ? "PR" : "Issue"} #${number}`;
  return project && project.toLowerCase() === repo.toLowerCase() ? name : `${repo} ${name}`;
}

/** The project a chat belongs to, by the name the sidebar uses. */
export function projectLabel(session: Session, workspace?: Workspace, projects: readonly Project[] = []): string | null {
  const project = workspace?.projectId ? projects.find(candidate => candidate.id === workspace.projectId) : undefined;
  if (project?.name) return project.name;
  if (workspace?.title) return workspace.title;
  const folder = session.cwd?.split(/[\\/]/).filter(Boolean).at(-1);
  return folder ?? null;
}

/** Links read as where they point: `https://github.com/o/kairo/pull/43` becomes `kairo/pull/43`. */
export function compactLinks(text: string): string {
  return text
    .replace(/https?:\/\/(?:www\.)?github\.com\/[^/\s]+\/(\S+)/gi, "$1")
    .replace(/https?:\/\/(?:www\.)?(\S+?)\/?(?=\s|$)/gi, "$1");
}

/** A message as the ask line shows it: one line, links compacted. */
export function askText(text: unknown): string | null {
  if (typeof text !== "string") return null;
  const collapsed = text.replace(/\s+/g, " ").trim();
  return collapsed ? compactLinks(collapsed) : null;
}

function entryText(entry: SessionEntry): string | null {
  const payload = entry.payload as { text?: unknown; message?: unknown } | null;
  return askText(payload?.text) ?? askText(payload?.message);
}

/**
 * The newest thing the user said to this chat: the goal a returning reader
 * needs first. Live events are the tail of the same stream the forest records,
 * so a user message among them is never older than the forest's latest.
 */
export function latestAsk(entries: readonly SessionEntry[] | undefined, events: AgentEvent[] = []): string | null {
  // the transcript codec is the one reader of live wire kinds.
  const live = events.length ? reduceConversation(events) : [];
  for (let index = live.length - 1; index >= 0; index -= 1) {
    const item = live[index];
    if (item.type !== "message" || item.role !== "user") continue;
    const text = askText(item.text);
    if (text) return text;
  }
  if (!entries) return null;
  for (let index = entries.length - 1; index >= 0; index -= 1) {
    const entry = entries[index];
    if (entry.kind !== "user.message") continue;
    const text = entryText(entry);
    if (text) return text;
  }
  return null;
}
