// The webview zoom level, which Bridge owns rather than Tauri.
//
// Tauri can inject a polyfill that answers Cmd+ and pinch, and it is enabled in
// `src-tauri/tauri.conf.json`. We turn it off because the step it takes is a
// hardcoded `0.2` inside the crate, applied once per event, with no
// configuration beside it: one press from 100% lands on 120%, and one trackpad
// pinch emits a burst of wheel events that walks it to 360% in a single gesture.
// There is no setting between "no zoom" and "zoom at 20% per event", so owning
// the level is the only way to make a step finer.
//
// Three things come with owning it, none of which the polyfill could express:
// a level that survives a relaunch, a level the UI can read and show, and a
// pinch that means one step instead of one step per event.

import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

/** Where the level is remembered, so a relaunch does not silently discard it. */
export const ZOOM_STORAGE_KEY = "bridge.window.zoom";

/** Tauri's own reset target, and the rung the ladder is centred on. */
export const ACTUAL_SIZE = 1;

/**
 * The rungs a step moves between, ascending. Fine next to 100% where people
 * spend their day, coarse past 200% where one press should still read as a
 * press. The first rung above `ACTUAL_SIZE` is 1.1, so a single Cmd+ is a tenth
 * rather than a fifth.
 *
 * Both ends are deliberate. 50% is there to fit a wide transcript into a narrow
 * window, which `tauri.conf.json` allows down to 420 points wide. 500% is where
 * the ladder stops: past that the composer and the sidebar are both gone, and a
 * runaway pinch should not be able to reach it.
 */
export const ZOOM_STEPS: readonly number[] = [
  0.5, 0.67, 0.75, 0.8, 0.9, ACTUAL_SIZE, 1.1, 1.25, 1.5, 1.75, 2, 2.5, 3, 4, 5,
];

/**
 * Wheel travel, in pixels, that costs one rung.
 *
 * The polyfill stepped per event, so a gesture that emitted eight events cost
 * eight rungs. Accumulating instead means a gesture costs one, and a long
 * deliberate pinch can still reach a second. The remainder carries across
 * events, so several small nudges add up to a step rather than being discarded.
 */
export const WHEEL_RUNG_PX = 100;

export type ZoomDirection = 1 | -1;

type StorageReader = Pick<Storage, "getItem">;
type StorageWriter = Pick<Storage, "setItem">;

/** The index of the rung nearest a level, for a value read back from storage. */
function rungIndex(level: number): number {
  if (!Number.isFinite(level)) return ZOOM_STEPS.indexOf(ACTUAL_SIZE);
  let best = 0;
  let bestDistance = Math.abs(ZOOM_STEPS[0] - level);
  for (let index = 1; index < ZOOM_STEPS.length; index += 1) {
    const distance = Math.abs(ZOOM_STEPS[index] - level);
    if (distance < bestDistance) {
      best = index;
      bestDistance = distance;
    }
  }
  return best;
}

/** Snap any level onto the ladder, so nothing off-ladder can reach the webview. */
export function snapZoom(level: number): number {
  return ZOOM_STEPS[rungIndex(level)];
}

/**
 * Move `rungs` steps up or down the ladder, stopping at either end.
 *
 * Moving by index rather than by arithmetic is what keeps a long in, out, in
 * sequence landing back where it started: there is no running total to drift.
 */
export function stepZoom(level: number, rungs: number): number {
  if (!Number.isFinite(rungs) || rungs === 0) return snapZoom(level);
  const target = rungIndex(level) + Math.trunc(rungs);
  const clamped = Math.min(Math.max(target, 0), ZOOM_STEPS.length - 1);
  return ZOOM_STEPS[clamped];
}

export function zoomIn(level: number): number {
  return stepZoom(level, 1);
}

export function zoomOut(level: number): number {
  return stepZoom(level, -1);
}

export function zoomReset(): number {
  return ACTUAL_SIZE;
}

/** True when a step in this direction would move, so a control can disable itself. */
export function canZoom(level: number, direction: ZoomDirection): boolean {
  const next = rungIndex(level) + direction;
  return next >= 0 && next < ZOOM_STEPS.length;
}

/**
 * Charge wheel travel against the ladder.
 *
 * `pending` is signed pixels still owed a rung. `rungs` is whole steps to take
 * now, positive meaning zoom in, which is the direction of a negative delta, so
 * a pinch that grows the page reads as a positive `rungs`. Whatever travel is
 * left over is returned so the next event continues from it.
 */
export function chargeWheel(
  pending: number,
  deltaY: number,
  rungPx: number = WHEEL_RUNG_PX,
): { pending: number; rungs: number } {
  if (!Number.isFinite(deltaY) || rungPx <= 0) return { pending, rungs: 0 };
  const total = pending + deltaY;
  const whole = Math.trunc(total / rungPx);
  // Negating a zero would hand callers a negative zero, which compares equal to
  // zero but is not the zero a `rungs === 0` guard reads cleanly.
  return { pending: total - whole * rungPx, rungs: whole === 0 ? 0 : -whole };
}

/** The level to remember, snapped, and `ACTUAL_SIZE` when there is nothing usable. */
export function readZoom(storage: StorageReader = localStorage): number {
  try {
    const stored = storage.getItem(ZOOM_STORAGE_KEY);
    if (stored === null) return ACTUAL_SIZE;
    const parsed = Number.parseFloat(stored);
    if (!Number.isFinite(parsed)) return ACTUAL_SIZE;
    return snapZoom(parsed);
  } catch {
    // A storage that throws must not stop the app rendering at its natural size.
    return ACTUAL_SIZE;
  }
}

export function writeZoom(level: number, storage: StorageWriter = localStorage): void {
  try {
    storage.setItem(ZOOM_STORAGE_KEY, String(snapZoom(level)));
  } catch {
    // Losing the preference is survivable; failing the zoom that caused it is not.
  }
}

/**
 * Apply a level to the webview.
 *
 * This is Tauri's own `core:webview` command, the same one the polyfill used,
 * granted in `src-tauri/capabilities/default.json`. Going through it rather
 * than reaching into the webview keeps the `Resized` event that
 * `window_chrome::sync_fullscreen_chrome` already handles for zoom, so the
 * corner radius and the flush rectangle keep tracking the level.
 */
export async function applyZoom(level: number): Promise<number> {
  const snapped = snapZoom(level);
  await invoke("plugin:webview|set_webview_zoom", { value: snapped });
  return snapped;
}

/** Set, remember, and return a new level. The one path the UI and keys share. */
export async function commitZoom(level: number): Promise<number> {
  const snapped = await applyZoom(level);
  writeZoom(snapped);
  return snapped;
}

/** Move one rung from the remembered level. The path the keys and menu share. */
export async function nudgeZoom(direction: ZoomDirection): Promise<number> {
  const applied = await commitZoom(stepZoom(readZoom(), direction));
  announceZoom(applied);
  return applied;
}

/** Return to 100%, which is the only route back once a level is off 1. */
export async function resetZoom(): Promise<number> {
  const applied = await commitZoom(zoomReset());
  announceZoom(applied);
  return applied;
}

const ZOOM_CHANGED_EVENT = "bridge:window-zoom";

function announceZoom(level: number): void {
  window.dispatchEvent(new CustomEvent<number>(ZOOM_CHANGED_EVENT, { detail: level }));
}

/**
 * Re-apply the remembered level and answer pinch gestures.
 *
 * The wheel listener is what keeps pinch working at all: `zoomHotkeysEnabled`
 * being off also removes the polyfill's `ctrl+wheel` handler, so without this a
 * trackpad pinch would stop zooming. Charging travel against a threshold rather
 * than stepping per event is the fix for the gesture that used to reach 360%.
 */
export function installZoom(): () => void {
  applyZoom(readZoom()).catch(() => {
    // A zoom that cannot be applied leaves the webview where it is, which is
    // the natural size, and is not worth interrupting startup over.
  });

  let pending = 0;
  const onWheel = (event: WheelEvent) => {
    if (!event.ctrlKey) return;
    event.preventDefault();
    const charged = chargeWheel(pending, event.deltaY);
    pending = charged.pending;
    if (charged.rungs === 0) return;
    commitZoom(stepZoom(readZoom(), charged.rungs))
      .then(announceZoom)
      .catch(() => {
        pending = 0;
      });
  };

  window.addEventListener("wheel", onWheel, { passive: false });
  return () => window.removeEventListener("wheel", onWheel);
}

/** The current level, kept in step with changes from the keys, menu, or pinch. */
export function useZoomLevel(): [number, (next: number) => void] {
  const [level, setLevel] = useState<number>(() => readZoom());

  useEffect(() => {
    const onChange = (event: Event) => {
      setLevel((event as CustomEvent<number>).detail);
    };
    window.addEventListener(ZOOM_CHANGED_EVENT, onChange);
    window.addEventListener("storage", onChange);
    return () => {
      window.removeEventListener(ZOOM_CHANGED_EVENT, onChange);
      window.removeEventListener("storage", onChange);
    };
  }, []);

  const change = useCallback((next: number) => {
    setLevel(next);
    commitZoom(next)
      .then(announceZoom)
      .catch(() => {
        // Leave the control showing what the user asked for; the webview keeps
        // the last level that did apply.
      });
  }, []);

  return [level, change];
}
