import type { MenuBarProvider } from "../../protocol/generated/protocol";

export const defaultFavorites: MenuBarProvider[] = ["codex", "claude", "cursor"];

export function chooseFavorite(current: readonly MenuBarProvider[], position: number, provider: MenuBarProvider | ""): MenuBarProvider[] {
  if (position < 0 || position > 2) return [...current];
  const next: (MenuBarProvider | "")[] = [...current];
  const previous = next[position] ?? "";
  const existing = provider ? next.indexOf(provider) : -1;
  if (existing >= 0 && existing !== position) next[existing] = previous;
  next[position] = provider;
  return next.filter((id): id is MenuBarProvider => Boolean(id)).slice(0, 3);
}
