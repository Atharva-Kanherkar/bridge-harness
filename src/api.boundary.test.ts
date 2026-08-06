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
    // One raw listen() — the body of subscribe() itself.
    expect(source.match(/\blisten[(<]/g)).toHaveLength(1);
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
