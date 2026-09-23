// Every `@tauri-apps/api/window` call is an IPC command behind Tauri's ACL. A
// call with no matching permission in `src-tauri/capabilities/` does not fail
// at build time — it fails at the user's click, as a rejected promise reading
// `window.show not allowed. Permissions associated with this command:
// core:window:allow-show`. That is exactly how the tray shipped broken.
//
// So the requirement is derived from the source rather than restated: find the
// window-API methods the frontend actually calls, map each to its permission,
// and assert a capability grants it.
import { describe, expect, it } from "vitest";
import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";

const SOURCE_ROOT = join(__dirname);
const CAPABILITY_ROOT = join(__dirname, "..", "src-tauri", "capabilities");

/** camelCase window method → the core permission gating its IPC command. */
const PERMISSION_BY_METHOD: Record<string, string> = {
  show: "core:window:allow-show",
  hide: "core:window:allow-hide",
  close: "core:window:allow-close",
  destroy: "core:window:allow-destroy",
  minimize: "core:window:allow-minimize",
  unminimize: "core:window:allow-unminimize",
  maximize: "core:window:allow-maximize",
  unmaximize: "core:window:allow-unmaximize",
  setFocus: "core:window:allow-set-focus",
  setTitle: "core:window:allow-set-title",
  setPosition: "core:window:allow-set-position",
  setSize: "core:window:allow-set-size",
  setAlwaysOnTop: "core:window:allow-set-always-on-top",
  setSkipTaskbar: "core:window:allow-set-skip-taskbar",
  setDecorations: "core:window:allow-set-decorations",
  setResizable: "core:window:allow-set-resizable",
  setFullscreen: "core:window:allow-set-fullscreen",
  startDragging: "core:window:allow-start-dragging",
};

function sourceFiles(directory: string): string[] {
  return readdirSync(directory, { withFileTypes: true }).flatMap(entry => {
    const full = join(directory, entry.name);
    if (entry.isDirectory()) return entry.name === "node_modules" ? [] : sourceFiles(full);
    return /\.tsx?$/.test(entry.name) && !/\.test\.tsx?$/.test(entry.name) ? [full] : [];
  });
}

/** Methods invoked on a handle obtained from the window module, per file. */
function windowMethodCalls(source: string): string[] {
  if (!source.includes("@tauri-apps/api/window")) return [];
  // Bind the handles first (`const w = getCurrentWindow()`), then collect the
  // methods called on them. Scoping to those identifiers keeps unrelated
  // `.show(` calls on other objects out of the requirement set.
  const handles = new Set<string>();
  for (const match of source.matchAll(/(?:const|let|var)\s+([A-Za-z_$][\w$]*)\s*=\s*(?:await\s+)?(?:getCurrentWindow|getCurrent)\s*\(/g)) {
    handles.add(match[1]);
  }
  const called: string[] = [];
  for (const handle of handles) {
    for (const match of source.matchAll(new RegExp(`\\b${handle}\\s*\\.\\s*([A-Za-z_$][\\w$]*)\\s*\\(`, "g"))) {
      called.push(match[1]);
    }
  }
  // Direct chaining, e.g. `getCurrentWindow().hide()`.
  for (const match of source.matchAll(/(?:getCurrentWindow|getCurrent)\s*\(\s*\)\s*\.\s*([A-Za-z_$][\w$]*)\s*\(/g)) {
    called.push(match[1]);
  }
  return called;
}

/** Every window that renders this frontend. The bundle is shared, so a call in
 *  `src/` can execute in any of them and must be permitted in all of them. */
const APP_WINDOWS = ["main", "meter"];

/** Shell webview label → permissions granted to it. Capabilities are scoped
 * to the exact shell view so a browser child cannot inherit its window ACL. */
function grantedByWindow(): Map<string, Set<string>> {
  const granted = new Map<string, Set<string>>(APP_WINDOWS.map(label => [label, new Set<string>()]));
  for (const name of readdirSync(CAPABILITY_ROOT)) {
    if (!name.endsWith(".json")) continue;
    const capability = JSON.parse(readFileSync(join(CAPABILITY_ROOT, name), "utf8")) as {
      windows?: string[];
      webviews?: string[];
      permissions?: Array<string | { identifier?: string }>;
    };
    for (const label of [...(capability.windows ?? []), ...(capability.webviews ?? [])]) {
      const bucket = granted.get(label);
      if (!bucket) continue;
      for (const permission of capability.permissions ?? []) {
        const identifier = typeof permission === "string" ? permission : permission.identifier;
        if (identifier) bucket.add(identifier);
      }
    }
  }
  return granted;
}

describe("tauri window capabilities", () => {
  it("grants a permission for every window API the frontend calls", () => {
    const granted = grantedByWindow();
    const missing: string[] = [];
    for (const file of sourceFiles(SOURCE_ROOT)) {
      for (const method of windowMethodCalls(readFileSync(file, "utf8"))) {
        const permission = PERMISSION_BY_METHOD[method];
        // An unmapped method is not a pass — the map has to grow with usage,
        // otherwise this guard quietly stops guarding.
        expect(
          permission,
          `${file.replace(SOURCE_ROOT, "src")} calls window.${method}(); add it to PERMISSION_BY_METHOD`,
        ).toBeDefined();
        if (!permission) continue;
        for (const label of APP_WINDOWS) {
          if (!granted.get(label)?.has(permission)) {
            missing.push(`${file.replace(SOURCE_ROOT, "src")} calls window.${method}() but window "${label}" is not granted ${permission}`);
          }
        }
      }
    }
    expect(missing).toEqual([]);
  });

  it("gives the meter panel window its own capability", () => {
    const covered = readdirSync(CAPABILITY_ROOT)
      .filter(name => name.endsWith(".json"))
      .flatMap(name => {
        const capability = JSON.parse(readFileSync(join(CAPABILITY_ROOT, name), "utf8")) as { webviews?: string[]; windows?: string[] };
        expect(capability.windows ?? [], "Window-wide permissions would include untrusted browser children").toEqual([]);
        return capability.webviews ?? [];
      });
    // A window absent from every capability can invoke no plugin command at
    // all, which is a silent, click-time failure rather than a build error.
    expect(covered).toContain("meter");
    expect(covered).toContain("main");
  });

  it("grants the main window the reveal permissions", () => {
    const main = grantedByWindow().get("main")!;
    for (const permission of [
      "core:window:allow-show",
      "core:window:allow-hide",
      "core:window:allow-unminimize",
      "core:window:allow-set-focus",
    ]) {
      expect([...main], `main must stay granted ${permission}`).toContain(permission);
    }
  });
});
