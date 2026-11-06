// Option lists written against the schema as stored, never a copy a menu or
// picker took when it opened. `setOptions` replaces the whole list, so each
// write carries `base`, the ids it was built from: if another writer moved the
// list in between, the write is refused and rebuilt from a fresh read, never
// deleting options someone else added or clearing the rows that use them.

import { bridgeApi } from "../../../api";
import type { NoteDatabase, NoteOption, NoteOptionInput } from "../../../types";

const asInput = (option: NoteOption): NoteOptionInput => ({ id: option.id, name: option.name, color: option.color ?? null, group: option.group ?? null });

/** An edited option list merged with the stored one: an edited option whose id
 *  the store no longer has is dropped (someone deleted it), and a stored option
 *  the editor never saw (someone added it) is kept, after the edited ones. */
export function reconcileOptions(seen: string[], edited: NoteOptionInput[], stored: NoteOption[]): NoteOptionInput[] {
  const storedIds = new Set(stored.map(option => option.id));
  const editedIds = new Set(edited.flatMap(option => option.id ? [option.id] : []));
  const kept = edited.filter(option => !option.id || storedIds.has(option.id));
  const added = stored.filter(option => !seen.includes(option.id) && !editedIds.has(option.id)).map(asInput);
  return [...kept, ...added];
}

/** The refusal a conditional write gets when the list moved under it. */
const CHANGED = "changed since they were read";
const ATTEMPTS = 3;
/** An error's words, whether the host rejects with an Error, a string or a
 *  `{message}` object. */
const errorText = (error: unknown): string => error instanceof Error ? error.message : typeof error === "object" && error && "message" in error ? String((error as { message: unknown }).message) : String(error);

async function stored(databaseId: string, propertyId: string): Promise<NoteOption[]> {
  const page = await bridgeApi.readNotePage(databaseId);
  const property = page.database?.properties.find(candidate => candidate.id === propertyId);
  if (!property) throw new Error("this property no longer exists");
  return property.options ?? [];
}

/** Build a list from the options as stored and write it on the condition
 *  that they are still stored; if they moved, read again and rebuild. */
async function writeOptions(databaseId: string, propertyId: string, build: (current: NoteOption[]) => NoteOptionInput[]): Promise<NoteDatabase> {
  for (let attempt = 1; ; attempt += 1) {
    const current = await stored(databaseId, propertyId);
    try {
      return await bridgeApi.updateDatabaseSchema({ id: databaseId, changes: [{ op: "setOptions", propertyId, options: build(current), base: current.map(option => option.id) }] });
    } catch (error) {
      if (attempt >= ATTEMPTS || !errorText(error).includes(CHANGED)) throw error;
    }
  }
}

/** Save an options editor's list; `seen` is the ids it opened with. */
export async function saveOptions(databaseId: string, propertyId: string, edited: NoteOptionInput[], seen: string[]): Promise<NoteDatabase> {
  return writeOptions(databaseId, propertyId, current => reconcileOptions(seen, edited, current));
}

/** Add one option to the stored list. */
export async function addOption(databaseId: string, propertyId: string, option: NoteOptionInput): Promise<NoteDatabase> {
  return writeOptions(databaseId, propertyId, current => [...current.map(asInput), option]);
}
