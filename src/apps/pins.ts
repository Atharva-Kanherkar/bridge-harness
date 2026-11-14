// Apps the person pinned to the sidebar: at most three, in the order pinned.
//
// One module-level store, so the sidebar, the launcher and the Store all read
// the same list and redraw together. Local to this machine for now; a later
// slice moves pins into core so every client shares them.

import { useSyncExternalStore } from "react";
import { isAppId, type AppId } from "./registry";

export const PINS_KEY = "bridge.apps.pins.v1";
export const MAX_PINS = 3;
export const PIN_LIMIT_MESSAGE = `You can pin ${MAX_PINS} apps. Unpin one first.`;

export type PinResult = { ok: true; pins: readonly AppId[] } | { ok: false; reason: "full" };

const EMPTY: readonly AppId[] = [];
const listeners = new Set<() => void>();
let cache: readonly AppId[] | null = null;

function storage(): Storage | null {
  try {
    return typeof localStorage === "undefined" ? null : localStorage;
  } catch {
    return null;
  }
}

/** Unknown ids, repeats and anything past the limit are dropped, so a stale
 *  or hand-edited value can never pin a fourth app or a removed one. */
export function parsePins(raw: string | null): readonly AppId[] {
  if (!raw) return EMPTY;
  let stored: unknown;
  try {
    stored = JSON.parse(raw);
  } catch {
    return EMPTY;
  }
  if (!Array.isArray(stored)) return EMPTY;
  const pins: AppId[] = [];
  for (const value of stored) {
    if (pins.length === MAX_PINS) break;
    if (isAppId(value) && !pins.includes(value)) pins.push(value);
  }
  return pins;
}

export function readPins(): readonly AppId[] {
  if (cache === null) {
    let raw: string | null = null;
    try {
      raw = storage()?.getItem(PINS_KEY) ?? null;
    } catch {
      raw = null;
    }
    cache = parsePins(raw);
  }
  return cache;
}

function write(next: readonly AppId[]) {
  cache = next;
  try {
    storage()?.setItem(PINS_KEY, JSON.stringify(next));
  } catch {
    // Storage full or blocked: the pin still holds for this session.
  }
  for (const listener of listeners) listener();
}

export function pinApp(id: AppId): PinResult {
  const pins = readPins();
  if (pins.includes(id)) return { ok: true, pins };
  if (pins.length >= MAX_PINS) return { ok: false, reason: "full" };
  const next = [...pins, id];
  write(next);
  return { ok: true, pins: next };
}

export function unpinApp(id: AppId): readonly AppId[] {
  const pins = readPins();
  if (!pins.includes(id)) return pins;
  const next = pins.filter(pin => pin !== id);
  write(next);
  return next;
}

export function togglePin(id: AppId): PinResult {
  return readPins().includes(id) ? { ok: true, pins: unpinApp(id) } : pinApp(id);
}

export function subscribePins(listener: () => void): () => void {
  listeners.add(listener);
  return () => { listeners.delete(listener); };
}

export function usePins(): readonly AppId[] {
  return useSyncExternalStore(subscribePins, readPins, readPins);
}

/** Tests only: forget the cached read so the next one sees storage again. */
export function resetPinsCache() {
  cache = null;
}
