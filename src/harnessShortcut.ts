// `$harness` composer shortcut: typing `$codex do you think we're right?`
// anywhere skips the current session and starts a fresh direct chat pinned
// to that harness instead. Distinct from `/name` slash commands, which stay
// inside the current direct chat and only switch its harness.
const HARNESS_SHORTCUT = /^\$([A-Za-z][A-Za-z0-9._-]*)\s+([\s\S]*)$/;

/** A completed `$id <message>` shortcut, or null if the composer doesn't have one yet. */
export function parseHarnessShortcut(text: string): { harnessId: string; rest: string } | null {
  const match = HARNESS_SHORTCUT.exec(text);
  if (!match) return null;
  const rest = match[2].trim();
  if (!rest) return null;
  return { harnessId: match[1], rest };
}

/** The `$id` token being typed at the very end of the composer with nothing after it yet — drives the autocomplete dropdown. */
export function harnessShortcutQuery(text: string): string | undefined {
  return /^\$([^\s]*)$/.exec(text)?.[1];
}

function editDistance(left: string, right: string): number {
  const previous = Array.from({ length: right.length + 1 }, (_, index) => index);
  for (let leftIndex = 1; leftIndex <= left.length; leftIndex += 1) {
    const current = [leftIndex];
    for (let rightIndex = 1; rightIndex <= right.length; rightIndex += 1) {
      current[rightIndex] = Math.min(
        current[rightIndex - 1] + 1,
        previous[rightIndex] + 1,
        previous[rightIndex - 1] + Number(left[leftIndex - 1] !== right[rightIndex - 1]),
      );
    }
    previous.splice(0, previous.length, ...current);
  }
  return previous[right.length];
}

/** A uniquely close installed harness id, used only as a typo hint. */
export function closestHarnessShortcut(requested: string, harnessIds: string[]): string | undefined {
  const token = requested.toLowerCase();
  const ranked = harnessIds
    .map(harnessId => ({ harnessId, distance: editDistance(token, harnessId.toLowerCase()) }))
    .sort((left, right) => left.distance - right.distance || left.harnessId.localeCompare(right.harnessId));
  const closest = ranked[0];
  if (!closest || closest.distance > Math.max(1, Math.ceil(token.length / 3))) return undefined;
  if (ranked[1]?.distance === closest.distance) return undefined;
  return closest.harnessId;
}
