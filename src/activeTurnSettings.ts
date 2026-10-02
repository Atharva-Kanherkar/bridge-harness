import { useSyncExternalStore } from "react";

export type ActiveTurnInput = "steer" | "queue";
export const ACTIVE_TURN_INPUT_KEY = "bridge.composer.activeTurnInput";
const CHANGE_EVENT = "bridge:active-turn-input";

export function readActiveTurnInput(): ActiveTurnInput {
  try { return localStorage.getItem(ACTIVE_TURN_INPUT_KEY) === "queue" ? "queue" : "steer"; }
  catch { return "steer"; }
}

export function writeActiveTurnInput(value: ActiveTurnInput): void {
  localStorage.setItem(ACTIVE_TURN_INPUT_KEY, value);
  window.dispatchEvent(new Event(CHANGE_EVENT));
}

function subscribe(onChange: () => void): () => void {
  window.addEventListener(CHANGE_EVENT, onChange);
  window.addEventListener("storage", onChange);
  return () => {
    window.removeEventListener(CHANGE_EVENT, onChange);
    window.removeEventListener("storage", onChange);
  };
}

export function useActiveTurnInput(): ActiveTurnInput {
  return useSyncExternalStore(subscribe, readActiveTurnInput, () => "steer");
}
