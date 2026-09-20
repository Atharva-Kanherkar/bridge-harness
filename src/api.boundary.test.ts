import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { BRIDGE_METHODS } from "./protocol/generated/protocol";

// The typed protocol boundary is a source-level invariant, mirroring the
// Rust shell's own source tests: every Tauri round-trip in api.ts goes
// through call()/subscribe(), whose params, results, and names come from the
// generated contract. A raw invoke()/listen() call would reopen the door to
// hand-typed wire shapes — exactly what #126 removed.
describe("the api boundary consumes the generated contract", () => {
  const source = readFileSync(fileURLToPath(new URL("./api.ts", import.meta.url)), "utf8");

  it("routes every command through the typed call() helper", () => {
    // One raw invoke() — the body of call() itself.
    expect(source.match(/\binvoke\(/g)).toHaveLength(1);
    expect(source).not.toMatch(/\binvoke\("/);
  });

  it("routes every event subscription through the typed subscribe() helper", () => {
    // Naming the call sites rather than counting them: a raw listen() is only
    // allowed for subscribe()'s own body and for the three Tauri events the
    // desktop shell owns rather than the protocol — the menu channel, the
    // meter-tray channel (native tray menu/left-click, same exemption), and
    // the batched agent stream the shell coalesces on its way to the webview
    // (`src-tauri/src/agent_batch.rs`; the daemon still speaks `agent-event`
    // one frame at a time). Any other literal here would be a hand-typed wire
    // shape.
    const targets = [...source.matchAll(/\blisten(?:<[^>]*>)?\(([^,]+),/g)].map(match => match[1].trim());
    // Shell-owned usage presentation carries a generated snapshot; navigation
    // belongs to the native host, like the main menu channel.
    expect(targets).toEqual(["notification", '"bridge-provider-usage-overviews"', '"bridge-menu-bar-connection"', '"bridge-menu-bar-settings-changed"', '"bridge-usage-overview"', '"bridge-menu-bar-settings"', '"bridge-meter-tray"', "MENU_COMMAND_EVENT", "AGENT_EVENT_BATCH"]);
  });

  it("only calls methods the generated registry declares", () => {
    const registered = new Set<string>(BRIDGE_METHODS.map(entry => entry.method));
    const called = [...source.matchAll(/\bcall\("([^"]+)"/g)].map(match => match[1]);
    expect(called.length).toBeGreaterThan(70);
    for (const method of called) {
      expect(registered, `${method} is not in the generated method registry`).toContain(method);
    }
  });
});
