// Option lists written against the schema as stored, never a copy a menu or
// picker took when it opened. `setOptions` replaces the whole list, so a
// stale copy would delete options someone else added and clear the rows that
// use them.

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

async function stored(databaseId: string, propertyId: string): Promise<NoteOption[]> {
  const page = await bridgeApi.readNotePage(databaseId);
  const property = page.database?.properties.find(candidate => candidate.id === propertyId);
  if (!property) throw new Error("this property no longer exists");
  return property.options ?? [];
}

/** Save an options editor's list; `seen` is the ids it opened with. */
export async function saveOptions(databaseId: string, propertyId: string, edited: NoteOptionInput[], seen: string[]): Promise<NoteDatabase> {
  const options = reconcileOptions(seen, edited, await stored(databaseId, propertyId));
  return bridgeApi.updateDatabaseSchema({ id: databaseId, changes: [{ op: "setOptions", propertyId, options }] });
}

/** Add one option to the stored list. */
export async function addOption(databaseId: string, propertyId: string, option: NoteOptionInput): Promise<NoteDatabase> {
  const options = [...(await stored(databaseId, propertyId)).map(asInput), option];
  return bridgeApi.updateDatabaseSchema({ id: databaseId, changes: [{ op: "setOptions", propertyId, options }] });
}
