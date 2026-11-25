// Starred models for the model picker. Local to this window's storage: a
// favorite is a shortcut on this machine, not a setting the backend has to
// know about.
//
// A favorite names a model the way the picker emits it: the harness, the
// catalog id, and the account group it was starred in (null for the user's
// own login). The picker resolves each against the live catalog and skips one
// that no longer exists, so a retired model never shows as a dead row.

import { useCallback, useEffect, useRef, useState } from "react";

export const MODEL_FAVORITES_STORAGE_KEY = "bridge.modelFavorites";

/** Fired on `window` after a write, so every open picker repaints at once. */
export const MODEL_FAVORITES_EVENT = "bridge-model-favorites";

/** Enough for anyone's short list; a corrupt or runaway value stays bounded. */
const MAX_FAVORITES = 50;

export type ModelFavorite = { harness: string; model: string; account: string | null };

function defaultStorage(): Storage | null {
  try {
    return typeof localStorage === "undefined" ? null : localStorage;
  } catch {
    return null;
  }
}

function sameFavorite(a: ModelFavorite, b: ModelFavorite): boolean {
  return a.harness === b.harness && a.model === b.model && (a.account ?? null) === (b.account ?? null);
}

function parseFavorite(value: unknown): ModelFavorite | null {
  if (!value || typeof value !== "object") return null;
  const { harness, model, account } = value as Record<string, unknown>;
  if (typeof harness !== "string" || !harness || typeof model !== "string" || !model) return null;
  if (account != null && typeof account !== "string") return null;
  return { harness, model, account: account || null };
}

export function readModelFavorites(storage: Pick<Storage, "getItem"> | null = defaultStorage()): ModelFavorite[] {
  let raw: string | null = null;
  try {
    raw = storage?.getItem(MODEL_FAVORITES_STORAGE_KEY) ?? null;
  } catch {
    return [];
  }
  if (!raw) return [];
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    return [];
  }
  if (!Array.isArray(parsed)) return [];
  const favorites: ModelFavorite[] = [];
  for (const entry of parsed) {
    const favorite = parseFavorite(entry);
    if (favorite && !favorites.some(existing => sameFavorite(existing, favorite))) favorites.push(favorite);
    if (favorites.length >= MAX_FAVORITES) break;
  }
  return favorites;
}

/** True when the list was stored. */
export function writeModelFavorites(favorites: readonly ModelFavorite[], storage: Pick<Storage, "setItem"> | null = defaultStorage()): boolean {
  if (!storage) return false;
  try {
    storage.setItem(MODEL_FAVORITES_STORAGE_KEY, JSON.stringify(favorites.slice(0, MAX_FAVORITES)));
    return true;
  } catch {
    // A read-only storage should never stop the picker from working; the star
    // just does not survive a reload.
    return false;
  }
}

export function isModelFavorite(favorites: readonly ModelFavorite[], candidate: ModelFavorite): boolean {
  return favorites.some(favorite => sameFavorite(favorite, candidate));
}

/** Adds the model at the end, or removes it if it was already starred. */
export function toggleModelFavorite(favorites: readonly ModelFavorite[], candidate: ModelFavorite): ModelFavorite[] {
  const normalized = { harness: candidate.harness, model: candidate.model, account: candidate.account ?? null };
  return isModelFavorite(favorites, normalized)
    ? favorites.filter(favorite => !sameFavorite(favorite, normalized))
    : [...favorites, normalized].slice(0, MAX_FAVORITES);
}

/** The starred models and a toggle that persists and broadcasts the change. */
export function useModelFavorites(): [ModelFavorite[], (candidate: ModelFavorite) => void] {
  const [favorites, setFavorites] = useState<ModelFavorite[]>(() => readModelFavorites());
  const current = useRef(favorites);
  current.current = favorites;
  useEffect(() => {
    const reread = () => setFavorites(readModelFavorites());
    const onStorage = (event: StorageEvent) => { if (event.key === null || event.key === MODEL_FAVORITES_STORAGE_KEY) reread(); };
    window.addEventListener(MODEL_FAVORITES_EVENT, reread);
    window.addEventListener("storage", onStorage);
    return () => {
      window.removeEventListener(MODEL_FAVORITES_EVENT, reread);
      window.removeEventListener("storage", onStorage);
    };
  }, []);
  const toggle = useCallback((candidate: ModelFavorite) => {
    const next = toggleModelFavorite(current.current, candidate);
    current.current = next;
    setFavorites(next);
    // Only a stored change is announced: the other pickers re-read storage, and
    // a failed write would have them, and this one, read the old list back.
    if (writeModelFavorites(next)) window.dispatchEvent(new Event(MODEL_FAVORITES_EVENT));
  }, []);
  return [favorites, toggle];
}
