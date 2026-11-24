/** Every top-level screen, as a value so a test can walk them. */
export const APP_VIEWS = ["workspace", "agent-fleet", "mission-control", "work", "projects", "memory", "notes", "trails", "marketplace", "usage", "pulls", "reviews", "settings", "workflows", "archives", "saved-setups", "briefing-settings", "personal-app"] as const;

export type AppView = typeof APP_VIEWS[number];

export type AppPlace = {
  view: AppView;
  sessionId: string | null;
  paradigm: "single" | "grid";
  /** The personal app on screen, for the `personal-app` view. */
  appId?: string | null;
};

export function placesEqual(a: AppPlace, b: AppPlace): boolean {
  return a.view === b.view && a.sessionId === b.sessionId && a.paradigm === b.paradigm && (a.appId ?? null) === (b.appId ?? null);
}

export function recordPlace(stack: AppPlace[], index: number, next: AppPlace): { stack: AppPlace[]; index: number } {
  const current = stack[index];
  if (current && placesEqual(current, next)) return { stack, index };
  const trimmed = stack.slice(0, index + 1);
  trimmed.push(next);
  return { stack: trimmed, index: trimmed.length - 1 };
}
