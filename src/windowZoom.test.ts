import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

describe("desktop text zoom", () => {
  it("enables Tauri's native zoom shortcuts and grants their webview command", () => {
    const tauri = JSON.parse(readFileSync(join(__dirname, "..", "src-tauri", "tauri.conf.json"), "utf8"));
    const capability = JSON.parse(readFileSync(join(__dirname, "..", "src-tauri", "capabilities", "default.json"), "utf8"));

    expect(tauri.app.windows[0].zoomHotkeysEnabled).toBe(true);
    expect(capability.permissions).toContain("core:webview:allow-set-webview-zoom");
  });
});
