import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative } from "node:path";
import { describe, expect, it } from "vitest";

// Raw error text reached toasts, cards and banners verbatim — Rust error
// Display strings, JSON envelopes, parser reports — because each screen
// turned a caught error into text its own way. Display goes through
// `friendlyError` (src/errors.ts); logic that reads the raw text goes through
// `errorMessage`. This guard keeps the ad-hoc conversions from coming back.
//
// A line that truly needs one (the text is parsed, logged, or sent back to a
// model, never shown) ends with `// raw-error: <why>`.

const ROOT = join(__dirname);
const RAW = /String\((?:error|err|e|reason|cause|failure|ex)\)|\b(?:error|err|e|reason|cause|failure|ex)\.message\b|instanceof Error \?/;
const SKIP = /(\.test\.tsx?$|\/generated\/|^errors\.ts$|\.d\.ts$)/;

function sources(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) return sources(path);
    return /\.tsx?$/.test(name) ? [path] : [];
  });
}

describe("raw error text", () => {
  it("is never turned into display text outside src/errors.ts", () => {
    const offenders: string[] = [];
    for (const path of sources(ROOT)) {
      const file = relative(ROOT, path);
      if (SKIP.test(file)) continue;
      readFileSync(path, "utf8").split("\n").forEach((line, index) => {
        if (RAW.test(line) && !line.includes("raw-error:")) offenders.push(`src/${file}:${index + 1}: ${line.trim()}`);
      });
    }
    expect(offenders, "Show errors with friendlyError(error, \"Couldn't …\"); read raw text with errorMessage(error).").toEqual([]);
  });
});
