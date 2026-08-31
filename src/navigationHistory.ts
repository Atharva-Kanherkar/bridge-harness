export type AppView = "workspace" | "work" | "projects" | "memory" | "marketplace" | "settings";

export type AppPlace = {
  view: AppView;
  sessionId: string | null;
  paradigm: "single" | "grid";
};

export function placesEqual(a: AppPlace, b: AppPlace): boolean {
  return a.view === b.view && a.sessionId === b.sessionId && a.paradigm === b.paradigm;
}

export function recordPlace(stack: AppPlace[], index: number, next: AppPlace): { stack: AppPlace[]; index: number } {
  const current = stack[index];
  if (current && placesEqual(current, next)) return { stack, index };
  const trimmed = stack.slice(0, index + 1);
  trimmed.push(next);
  return { stack: trimmed, index: trimmed.length - 1 };
}
