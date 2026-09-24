import type { Project, Session, SessionEntry, Workspace } from "../../types";

// What a tile is called. The stored title is a three-word heading cut from the
// first message, so a pasted link arrives as the raw URL; a grid of those tells
// nobody which chat is which.

const GITHUB = /^https?:\/\/(?:www\.)?github\.com\/([^/\s]+)\/([^/\s#?]+)(?:\/(pull|pulls|issues)\/(\d+))?\S*$/i;
const URL_PATTERN = /^https?:\/\/\S+$/i;
// what the backend now derives from such a link: "kairo PR #43".
const LINK_HEADING = /^(\S+) (PR|issue) #(\d+)$/i;

function hostTitle(url: string): string {
  try {
    const parsed = new URL(url);
    const tail = decodeURIComponent(parsed.pathname.split("/").filter(Boolean).at(-1) ?? "");
    const host = parsed.host.replace(/^www\./, "");
    // the last path segment usually says what the page is; an opaque id does not.
    const readable = tail.length <= 32 && /\p{L}/u.test(tail) && !/^[0-9a-f-]{8,}$/i.test(tail);
    return readable ? `${host} ${tail}` : host;
  } catch {
    return url;
  }
}

/** A readable name for a stored title. Plain titles, including ones the user chose, pass through unchanged. */
export function displayTitle(raw: string, project?: string | null): string {
  const title = raw.trim();
  const heading = LINK_HEADING.exec(title);
  if (heading) {
    const [, repo, kind, number] = heading;
    const name = `${kind.toUpperCase() === "PR" ? "PR" : "Issue"} #${number}`;
    return project && project.toLowerCase() === repo.toLowerCase() ? name : `${repo} ${name}`;
  }
  const [first = "", ...rest] = title.split(/\s+/);
  if (!URL_PATTERN.test(first)) return title;
  // the heading was cut at a word limit; a word ending in "…" is a fragment.
  const words = rest.filter(word => !word.endsWith("…"));
  const github = GITHUB.exec(first);
  if (github) {
    const [, , rawRepo, kind, number] = github;
    const repo = rawRepo.replace(/\.git$/i, "");
    const sameProject = !!project && project.toLowerCase() === repo.toLowerCase();
    if (number) {
      const name = `${kind.toLowerCase().startsWith("pull") ? "PR" : "Issue"} #${number}`;
      return sameProject ? name : `${repo} ${name}`;
    }
    return [repo, ...words].join(" ");
  }
  return [hostTitle(first), ...words].join(" ");
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

function entryText(entry: SessionEntry): string | null {
  const payload = entry.payload as { text?: unknown; message?: unknown } | null;
  const text = typeof payload?.text === "string" ? payload.text : typeof payload?.message === "string" ? payload.message : null;
  const collapsed = text?.replace(/\s+/g, " ").trim();
  return collapsed || null;
}

/** The newest thing the user said to this chat: the goal a returning reader needs first. */
export function latestAsk(entries: readonly SessionEntry[] | undefined): string | null {
  if (!entries) return null;
  for (let index = entries.length - 1; index >= 0; index -= 1) {
    const entry = entries[index];
    if (entry.kind !== "user.message") continue;
    const text = entryText(entry);
    if (text) return compactLinks(text);
  }
  return null;
}
