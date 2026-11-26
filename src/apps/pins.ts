// Apps the person pinned to the sidebar: at most three, in the order pinned.
//
// Core keeps the pins, so every client shows the same sidebar and core holds
// the limit. Slice 1 kept them in this browser's localStorage; the first time
// this client reaches core it moves those pins over once and forgets the key.

import { bridgeApi } from "../api";
import { friendlyError } from "../errors";
import { applyAnswer, pinsOf, useApps, usePersonalApps } from "./store";
import { coreIdOf, type AppKey } from "./registry";

export { importLegacyPins, MAX_PINS, parsePins, PINS_KEY } from "./store";
export const PIN_LIMIT_MESSAGE = `You can pin 3 apps. Unpin one first.`;

export type PinResult = { ok: true } | { ok: false; message: string };

export async function setPinned(key: AppKey, pinned: boolean): Promise<PinResult> {
  try {
    applyAnswer(await bridgeApi.setAppPinned(coreIdOf(key), pinned));
    return { ok: true };
  } catch (error) {
    return { ok: false, message: friendlyError(error, "Couldn't change the pin.") };
  }
}

export function togglePin(id: AppKey, pins: readonly AppKey[]): Promise<PinResult> {
  return setPinned(id, !pins.includes(id));
}

export function usePins(): readonly AppKey[] {
  return pinsOf(useApps(), usePersonalApps());
}
