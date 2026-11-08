// The seven saved views the space-wide system database starts with: the Notes
// list's default tabs. The core seeds the same filters
// (`notes::database::system_views`); the mock and "Reset to defaults" read
// them from here.

import type { NoteDatabaseView } from "./types";

export type SeededKind = "all" | "inbox" | "pages" | "tasks" | "ideas" | "agent" | "archived";

const condition = (property: string, op: string, value?: unknown) => (value === undefined ? { property, op } : { property, op, value });
const notArchived = condition("state", "isNot", "archived");
const notChildTask = { or: [condition("kind", "isNot", "task"), condition("parent", "isEmpty")] };

export const SEEDED_VIEWS: { kind: SeededKind; name: string; filter: unknown }[] = [
  { kind: "all", name: "All", filter: { and: [notArchived, notChildTask] } },
  { kind: "inbox", name: "Inbox", filter: { and: [condition("state", "is", "inbox"), notChildTask] } },
  { kind: "pages", name: "Pages", filter: { and: [condition("kind", "isNot", "task"), notArchived] } },
  { kind: "tasks", name: "Tasks", filter: { and: [condition("kind", "is", "task"), notArchived] } },
  { kind: "ideas", name: "Ideas", filter: { and: [condition("kind", "is", "idea")] } },
  { kind: "agent", name: "In progress with an agent", filter: { and: [condition("state", "is", "doing"), condition("chats", "isNotEmpty")] } },
  { kind: "archived", name: "Archived", filter: { and: [condition("state", "is", "archived")] } },
];

const key = (filter: unknown) => JSON.stringify(filter ?? null);

/** Which seeded list a view is, by its filter; null for a new or edited one. */
export function seededKind(view: Pick<NoteDatabaseView, "filter">): SeededKind | null {
  return SEEDED_VIEWS.find(seed => key(seed.filter) === key(view.filter))?.kind ?? null;
}
