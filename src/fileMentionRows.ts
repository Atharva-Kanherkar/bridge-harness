/**
 * How the composer's `@` menu draws one workspace path: the file name first,
 * the folder after it, and the typed query marked where it matched.
 *
 * A full path reads left to right through folders nobody is looking for before
 * it reaches the one word they typed. Splitting it puts that word first and
 * lets the folder sit dimmed beside it, there to tell two `index.ts` apart.
 *
 * Presentation only. Which paths match and in what order is the menu's ranker;
 * this never filters or reorders, it only says where to draw the mark.
 *
 * React-free on purpose, so it is testable as data.
 */

/** A path as the row shows it. `folder` has no trailing slash and is empty for a root file. */
export interface MentionPathParts {
  name: string;
  folder: string;
}

/** A half-open `[start, end)` range into one string. */
export type MatchRange = readonly [start: number, end: number];

/** Where the query sits in each half of the row. Either may be absent. */
export interface MentionMatch {
  name?: MatchRange;
  folder?: MatchRange;
}

/** One run of text, marked or not. */
export interface MarkedSegment {
  text: string;
  marked: boolean;
}

export function splitMentionPath(path: string): MentionPathParts {
  const slash = path.lastIndexOf("/");
  if (slash === -1) return { name: path, folder: "" };
  return { name: path.slice(slash + 1), folder: path.slice(0, slash) };
}

/**
 * Where to mark `query` in `path`, case-insensitively.
 *
 * The name wins: `pol` in `docs/policy/delegation-policy.md` marks the name,
 * because that is the half the reader looks at. Only when the name does not
 * hold the query is the first match in the whole path used, which can fall in
 * the folder or straddle the slash (`auth/tok`); then each half gets its share.
 */
export function mentionMatch(path: string, query: string): MentionMatch {
  const needle = query.toLowerCase();
  if (!needle) return {};
  const { name, folder } = splitMentionPath(path);
  const inName = name.toLowerCase().indexOf(needle);
  if (inName !== -1) return { name: clamp([inName, inName + needle.length], name.length) };
  const at = path.toLowerCase().indexOf(needle);
  if (at === -1) return {};
  const end = at + needle.length;
  const nameStart = path.length - name.length;
  const match: { name?: MatchRange; folder?: MatchRange } = {};
  if (folder && at < folder.length) match.folder = clamp([at, Math.min(end, folder.length)], folder.length);
  if (end > nameStart) match.name = clamp([Math.max(at, nameStart) - nameStart, end - nameStart], name.length);
  return match;
}

/** `text` cut into at most three runs around `range`, empty runs dropped. */
export function markSegments(text: string, range?: MatchRange): MarkedSegment[] {
  if (!range || range[1] <= range[0]) return text ? [{ text, marked: false }] : [];
  const [start, end] = clamp(range, text.length);
  return [
    { text: text.slice(0, start), marked: false },
    { text: text.slice(start, end), marked: true },
    { text: text.slice(end), marked: false },
  ].filter(segment => segment.text.length > 0);
}

/** Lower-casing can change a string's length for a few scripts; never index past the end. */
function clamp([start, end]: MatchRange, length: number): MatchRange {
  return [Math.max(0, Math.min(start, length)), Math.max(0, Math.min(end, length))];
}
