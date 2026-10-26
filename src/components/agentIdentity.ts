import type { Session } from "../types";

// How the Agents pane names a worker. The session label only says which role
// and tier ran ("Research · standard"), which every worker of that kind
// shares, so three research workers read as one. A worker gets a name of its
// own, picked from its id so it never changes, and its topic is the title
// Bridge cut from the objective the orchestrator gave it.

const NAMES = [
  "Alder", "Aspen", "Birch", "Bramble", "Cedar", "Clover", "Cobalt", "Coral",
  "Cypress", "Dune", "Ember", "Fennel", "Fern", "Finch", "Flint", "Garnet",
  "Hazel", "Heron", "Indigo", "Iris", "Jasper", "Juniper", "Kestrel", "Lark",
  "Linden", "Maple", "Marlin", "Moss", "Nova", "Oak", "Onyx", "Orchid",
  "Pebble", "Pike", "Quartz", "Quill", "Rain", "Reed", "Rowan", "Sage",
  "Slate", "Sorrel", "Sparrow", "Tamarind", "Teal", "Thistle", "Wren", "Yarrow",
];

/// A stable personal name for a worker: the same id always gets the same name.
export function agentName(sessionId: string): string {
  // FNV-1a: cheap, and spreads short uuid prefixes well enough for a name pool.
  let hash = 0x811c9dc5;
  for (let index = 0; index < sessionId.length; index += 1) {
    hash ^= sessionId.charCodeAt(index);
    hash = Math.imul(hash, 0x01000193);
  }
  return NAMES[(hash >>> 0) % NAMES.length];
}

/// The role half of a worker label: "Research · standard" is a Research worker.
export function agentRole(label: string): string {
  return label.split(" · ")[0]?.trim() || label;
}

/// What the worker is about, when Bridge has a title for it that says more
/// than the label.
export function agentTopic(session: Pick<Session, "title" | "label">): string | undefined {
  const title = session.title?.trim();
  return title && title !== session.label ? title : undefined;
}
