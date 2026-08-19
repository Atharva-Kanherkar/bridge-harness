import { invoke } from "@tauri-apps/api/core";

// macOS draws its window controls on top of our content, so the rail cannot quiet
// them with CSS. The native side hides them at startup and this reveals them while
// the pointer is in the corner they occupy — the web layer is the only side that
// sees the cursor.

/** The corner the buttons sit in: trafficLightPosition (18,15) plus their width. */
export const REVEAL_WIDTH = 112;
export const REVEAL_HEIGHT = 44;

export function inTrafficLightCorner(x: number, y: number): boolean {
  return x <= REVEAL_WIDTH && y <= REVEAL_HEIGHT;
}

const isTauri = () => typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

/** Watches the pointer and toggles the buttons as it enters and leaves the
 * corner. Returns a teardown. Only the transitions are sent, not every move. */
export function watchTrafficLights(): () => void {
  if (!isTauri()) return () => {};

  let visible: boolean | undefined;

  const apply = (next: boolean) => {
    if (next === visible) return;
    visible = next;
    void invoke("set_traffic_lights_visible", { visible: next }).catch(() => {
      // An older daemon-hosted build has no such command; leaving the buttons
      // as they are beats spamming the console on every pointer move.
      visible = undefined;
    });
  };

  const onPointerMove = (event: PointerEvent) => apply(inTrafficLightCorner(event.clientX, event.clientY));
  // Leaving the window entirely counts as leaving the corner, and a window that
  // loses focus should not keep them lit.
  const onPointerLeave = () => apply(false);
  const onBlur = () => apply(false);

  window.addEventListener("pointermove", onPointerMove, { passive: true });
  window.addEventListener("pointerleave", onPointerLeave);
  window.addEventListener("blur", onBlur);
  return () => {
    window.removeEventListener("pointermove", onPointerMove);
    window.removeEventListener("pointerleave", onPointerLeave);
    window.removeEventListener("blur", onBlur);
  };
}
