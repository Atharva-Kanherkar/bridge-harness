/** Fired on `document.documentElement` when the native window fills the display. */
export const FLUSH_WINDOW_EVENT = "bridge-flush-window";

export function isFlushWindowDocument(): boolean {
  return document.documentElement.hasAttribute("data-flush-window");
}

export function setLayoutFullscreenDocument(on: boolean): void {
  document.documentElement.toggleAttribute("data-fullscreen", on);
}

/** Tell the shell to square the native window for in-app ⌥⌘F. Native
 *  fullscreen/zoom is detected in Rust and must not go through this. */
export function notifyLayoutFullscreen(on: boolean): void {
  if (!("__TAURI_INTERNALS__" in window)) return;
  void import("@tauri-apps/api/event").then(({ emit }) => {
    void emit("bridge-layout-fullscreen", on);
  });
}
