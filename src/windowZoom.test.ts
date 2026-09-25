// What Bridge owes the webview about zoom, as a source-level contract.
//
// Tauri can answer Cmd+ and pinch by injecting a script, and the flag that does
// it is in `tauri.conf.json`. We keep it off, because the step that script takes
// is a hardcoded `0.2` inside the Tauri crate applied once per event, with no
// configuration beside it. These assertions are what stop it creeping back on,
// and what stop the permission it needed from being dropped when it went.

import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { SHORTCUTS } from "./keymap";
import { ZOOM_STEPS } from "./zoom";

const tauriConfig = JSON.parse(readFileSync(join(__dirname, "..", "src-tauri", "tauri.conf.json"), "utf8"));
const capability = JSON.parse(readFileSync(join(__dirname, "..", "src-tauri", "capabilities", "default.json"), "utf8"));
const menuSource = readFileSync(join(__dirname, "..", "src-tauri", "src", "menu.rs"), "utf8");

describe("desktop text zoom", () => {
  it("keeps Tauri's injected zoom polyfill switched off", () => {
    // On would reinstate the flat 0.2 step and the per-event pinch, and the
    // two handlers would fight over the same webview.
    expect(tauriConfig.app.windows[0].zoomHotkeysEnabled).toBe(false);
  });

  it("still grants the webview command Bridge now applies the level with", () => {
    expect(capability.permissions).toContain("core:webview:allow-set-webview-zoom");
  });

  it("answers zoom from the command table, at a finer step than 20%", () => {
    const ids = ["zoom-in", "zoom-out", "zoom-reset"];
    for (const id of ids) {
      expect(SHORTCUTS.some(shortcut => shortcut.id === id), `${id} is a command`).toBe(true);
    }
    // The step the polyfill could not be asked to change.
    const above = ZOOM_STEPS.filter(step => step > 1);
    expect(above[0]).toBeLessThan(1.2);
  });

  it("carries all three zoom commands in the View menu", () => {
    for (const id of ["zoom-in", "zoom-out", "zoom-reset"]) {
      expect(menuSource, `${id} is a menu item`).toContain(`Command("${id}"`);
    }
  });
});
