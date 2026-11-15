// Apps the person pinned to the sidebar: at most three, in the order pinned.
//
// Core keeps the pins, so every client shows the same sidebar and core holds
// the limit. Slice 1 kept them in this browser's localStorage; the first time
// this client reaches core it moves those pins over once and forgets the key.

import { bridgeApi } from "../api";
import { errorMessage } from "../errors";
import { applyAnswer, pinsOf, useApps } from "./store";
import type { AppId } from "./registry";

export { importLegacyPins, MAX_PINS, parsePins, PINS_KEY } from "./store";
export const PIN_LIMIT_MESSAGE = `You can pin 3 apps. Unpin one first.`;

export type PinResult = { ok: true } | { ok: false; message: string };

export async function setPinned(id: AppId, pinned: boolean): Promise<PinResult> {
  try {
    applyAnswer(await bridgeApi.setAppPinned(id, pinned));
    return { ok: true };
  } catch (error) {
    return { ok: false, message: errorMessage(error) };
  }
}

export function togglePin(id: AppId, pins: readonly AppId[]): Promise<PinResult> {
  return setPinned(id, !pins.includes(id));
}

export function usePins(): readonly AppId[] {
  return pinsOf(useApps());
}
