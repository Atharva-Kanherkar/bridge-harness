import type { MenuBarProvider } from "../../protocol/generated/protocol";

export const favoriteProviders: MenuBarProvider[] = ["codex", "claude", "cursor", "opencode"];

export const defaultFavorites: MenuBarProvider[] = ["codex", "claude"];

export function chooseFavorite(current: readonly MenuBarProvider[], position: number, provider: MenuBarProvider | ""): MenuBarProvider[] {
  if (position < 0 || position >= favoriteProviders.length) return [...current];
  const next: (MenuBarProvider | "")[] = [...current];
  const previous = next[position] ?? "";
  const existing = provider ? next.indexOf(provider) : -1;
  if (existing >= 0 && existing !== position) next[existing] = previous;
  next[position] = provider;
  return next.filter((id): id is MenuBarProvider => Boolean(id)).slice(0, favoriteProviders.length);
}
